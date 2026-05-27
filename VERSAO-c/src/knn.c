#define _GNU_SOURCE
#include "knn.h"

#include <immintrin.h>
#include <limits.h>
#include <stdint.h>
#include <string.h>
#include <xmmintrin.h>

extern void distance_block8_avx2(const int16_t *vectors, size_t block_off_i16,
                                 const query_vec_t *query, int64_t out[8]);

// Top-K with deterministic tie-break.
// key layout: [dist_sq: high bits][slot: low bits]
// slot is the absolute reference slot (block_idx * lanes + lane).
// SHIFT must be >= bits needed to represent slot.
enum { KEY_SHIFT = 24 };
static inline uint64_t make_key(uint64_t dist_sq, uint32_t slot)
{
    return (dist_sq << KEY_SHIFT) | ((uint64_t)slot & ((1ull << KEY_SHIFT) - 1ull));
}
static inline int64_t key_dist(uint64_t key) { return (int64_t)(key >> KEY_SHIFT); }

static inline void insert_best_key(uint64_t key, uint8_t label, uint64_t *keys, uint8_t *labels)
{
    if (key >= keys[IDX_TOP_K - 1]) return;

    int i = IDX_TOP_K - 1;
    while (i > 0 && key < keys[i - 1]) {
        keys[i] = keys[i - 1];
        labels[i] = labels[i - 1];
        i--;
    }
    keys[i] = key;
    labels[i] = label;
}

static inline uint8_t sum_labels(const uint8_t labels[IDX_TOP_K])
{
    return labels[0] + labels[1] + labels[2] + labels[3] + labels[4];
}

static inline int early_done(const uint64_t best_keys[IDX_TOP_K])
{
    return key_dist(best_keys[IDX_TOP_K - 1]) <= IDX_EARLY_DISTANCE_LIMIT;
}

static inline void read_qv(const uint8_t *p, query_vec_t *v)
{
    memcpy(v, p, 28);
    memset((char *)v + 28, 0, 4);
}

static int32_t read_i32_le(const uint8_t *p) { int32_t v; memcpy(&v, p, 4); return v; }

static void read_partition_root(const index_t *idx, int i, int32_t *root)
{
    const uint8_t *p = idx->data + idx->partitions_off + i * IDX_PART_SIZE;
    *root = read_i32_le(p + 4);
}

static void read_partition_bbox(const index_t *idx, int i, query_vec_t *min, query_vec_t *max)
{
    const uint8_t *p = idx->data + idx->partitions_off + i * IDX_PART_SIZE;
    read_qv(p + 12, min);
    read_qv(p + 44, max);
}

static void read_node_split(const index_t *idx, int i, int32_t *left, int32_t *right,
                            int32_t *start, int32_t *len)
{
    const uint8_t *p = idx->data + idx->nodes_off + i * IDX_NODE_SIZE;
    *left = read_i32_le(p);
    *right = read_i32_le(p + 4);
    *start = read_i32_le(p + 8);
    *len = read_i32_le(p + 12);
}

static void read_node_bounds(const index_t *idx, int i, query_vec_t *min, query_vec_t *max)
{
    const uint8_t *p = idx->data + idx->nodes_off + i * IDX_NODE_SIZE + 16;
    read_qv(p, min);
    read_qv(p + 32, max);
}

static inline void prefetch_partition_bbox(const index_t *idx, int i)
{
    const char *p = (const char *)(idx->data + idx->partitions_off + i * IDX_PART_SIZE + 12);
    _mm_prefetch(p, _MM_HINT_T0);
}

static inline void prefetch_node_bounds(const index_t *idx, int i)
{
    const char *p = (const char *)(idx->data + idx->nodes_off + i * IDX_NODE_SIZE + 16);
    _mm_prefetch(p, _MM_HINT_T0);
}

static int scan_leaf(const index_t *idx, int32_t start_block, int32_t len,
                     const query_vec_t *query, uint64_t *best_keys, uint8_t *best_labels)
{
    const int16_t *vecs = index_vectors_base(idx);
    int blocks = (len + IDX_LANES - 1) / IDX_LANES;
    int total_len = len;

    for (int b = 0; b < blocks; b++) {
        int block_idx = (int)start_block + b;
        if (b + 1 < blocks) {
            size_t next_off = (size_t)(block_idx + 1) * IDX_VECTOR_DIM * IDX_LANES;
            _mm_prefetch((const char *)(vecs + next_off), _MM_HINT_T0);
            _mm_prefetch((const char *)(idx->data + idx->labels_off + (block_idx + 1) * IDX_LANES),
                         _MM_HINT_T0);
        }
        size_t block_off = (size_t)block_idx * IDX_VECTOR_DIM * IDX_LANES;
        int64_t dists[IDX_LANES];
        distance_block8_avx2(vecs, block_off, query, dists);

        int lane_count = total_len - b * IDX_LANES;
        if (lane_count > IDX_LANES) lane_count = IDX_LANES;
        int labels_base = block_idx * IDX_LANES;
        int64_t cutoff = key_dist(best_keys[IDX_TOP_K - 1]);
        for (int lane = 0; lane < lane_count; lane++) {
            if (dists[lane] < cutoff) {
                uint8_t label = index_label_at(idx, labels_base + lane);
                const uint32_t slot = (uint32_t)(labels_base + lane);
                uint64_t key = make_key((uint64_t)dists[lane], slot);
                insert_best_key(key, label, best_keys, best_labels);
                cutoff = key_dist(best_keys[IDX_TOP_K - 1]);
                if (cutoff <= IDX_EARLY_DISTANCE_LIMIT) return 1;
            }
        }
    }
    return 0;
}

static int search_node(const index_t *idx, int32_t root, int64_t root_bound,
                       const query_vec_t *query,
                       uint64_t *best_keys, uint8_t *best_labels)
{
    if (root < 0 || (uint32_t)root >= index_node_count(idx)) return 0;

    int32_t stack_node[128];
    int64_t stack_bound[128];
    int sp = 0;
    int32_t current = root;
    int64_t current_bound = root_bound;
    int64_t cutoff = key_dist(best_keys[IDX_TOP_K - 1]);

    for (;;) {
        if (current_bound < cutoff) {
            int32_t left, right, start, length;
            read_node_split(idx, current, &left, &right, &start, &length);
            if (left < 0) {
                if (scan_leaf(idx, start, length, query, best_keys, best_labels)) return 1;
                cutoff = key_dist(best_keys[IDX_TOP_K - 1]);
            } else {
                query_vec_t lmin, lmax, rmin, rmax;
                prefetch_node_bounds(idx, left);
                prefetch_node_bounds(idx, right);
                read_node_bounds(idx, left, &lmin, &lmax);
                read_node_bounds(idx, right, &rmin, &rmax);
                int64_t lb = lower_bound_vec_cutoff(query, &lmin, &lmax, cutoff);
                int64_t rb = lower_bound_vec_cutoff(query, &rmin, &rmax, cutoff);
                int32_t near, far;
                int64_t near_b, far_b;
                if (lb <= rb) {
                    near = left; near_b = lb; far = right; far_b = rb;
                } else {
                    near = right; near_b = rb; far = left; far_b = lb;
                }
                if (far_b < cutoff && sp < 128) {
                    stack_node[sp] = far;
                    stack_bound[sp] = far_b;
                    sp++;
                }
                current = near;
                current_bound = near_b;
                cutoff = key_dist(best_keys[IDX_TOP_K - 1]);
                continue;
            }
        }
        if (sp == 0) break;
        sp--;
        current = stack_node[sp];
        current_bound = stack_bound[sp];
    }
    return early_done(best_keys);
}

static void sort_probes(int32_t *parts, int64_t *lbs, int n)
{
    for (int i = 1; i < n; i++) {
        int j = i;
        while (j > 0 && lbs[j] < lbs[j - 1]) {
            int32_t tp = parts[j]; parts[j] = parts[j - 1]; parts[j - 1] = tp;
            int64_t tl = lbs[j]; lbs[j] = lbs[j - 1]; lbs[j - 1] = tl;
            j--;
        }
    }
}

uint8_t fraud_count(const index_t *idx, const query_vec_t *query)
{
    uint64_t best_keys[IDX_TOP_K];
    uint8_t best_labels[IDX_TOP_K];
    for (int i = 0; i < IDX_TOP_K; i++) {
        best_keys[i] = UINT64_MAX;
        best_labels[i] = 0;
    }

    uint32_t key = partition_key(query);
    int32_t primary = index_part_by_key(idx, key);

    if (primary >= 0) {
        int32_t root;
        read_partition_root(idx, primary, &root);
        if (search_node(idx, root, 0, query, best_keys, best_labels))
            return sum_labels(best_labels);
    }

    int32_t part_count = (int32_t)index_part_count(idx);
    int32_t probe_parts[256];
    int64_t probe_lbs[256];
    int n = 0;
    int64_t cutoff = key_dist(best_keys[IDX_TOP_K - 1]);

    for (int32_t i = 0; i < part_count; i++) {
        if (i == primary) continue;
        if (i + 1 < part_count) {
            int32_t next = (i + 1 == primary) ? i + 2 : i + 1;
            if (next < part_count) prefetch_partition_bbox(idx, next);
        }
        query_vec_t pmin, pmax;
        read_partition_bbox(idx, i, &pmin, &pmax);
        int64_t lb = lower_bound_vec_cutoff(query, &pmin, &pmax, cutoff);
        if (lb >= cutoff) continue;
        probe_parts[n] = i;
        probe_lbs[n] = lb;
        n++;
        if (n == 256) break;
    }
    sort_probes(probe_parts, probe_lbs, n);

    for (int pi = 0; pi < n; pi++) {
        cutoff = key_dist(best_keys[IDX_TOP_K - 1]);
        if (probe_lbs[pi] >= cutoff) break;
        int32_t root;
        read_partition_root(idx, probe_parts[pi], &root);
        if (search_node(idx, root, probe_lbs[pi], query, best_keys, best_labels))
            break;
    }
    return sum_labels(best_labels);
}
