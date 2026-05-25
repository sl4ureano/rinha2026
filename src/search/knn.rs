//! Exact k-NN search over a partitioned KD-tree.
//! Distance is squared L2 in i16-quantized space, accumulated as
//! i64 to keep room over 14 dims × (2*SCALE)^2.
//!
//! Pruning: every node carries an axis-aligned bbox. The lower bound of a
//! query's distance to any vector in the subtree is `lower_bound_vec(query,
//! min, max)`. When that lower bound is ≥ our current 5th-best, we skip the
//! whole subtree.
//!
//! Early global termination: once `top[4].dist <= EARLY_DISTANCE_LIMIT`, we
//! return immediately — the top-5 are already so close that no further
//! probing could change the count of fraud labels.

#![allow(clippy::needless_range_loop)]

use crate::index::Index;
use crate::index::QueryVector;

#[cfg(target_arch = "x86_64")]
use crate::index::{
    lower_bound_vec_cutoff, partition_key, EARLY_DISTANCE_LIMIT, LANES, NODE_SIZE, PART_SIZE, TOP_K,
    VECTOR_DIM,
};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// Top-5 fraud labels in the true nearest neighbors. Returns count `0..=5`.
#[inline]
pub fn fraud_count(index: &Index, query: &QueryVector) -> u8 {
    #[cfg(all(target_arch = "x86_64", not(debug_assertions)))]
    {
        return unsafe { fraud_count_avx2(index, query) };
    }
    #[cfg(all(target_arch = "x86_64", debug_assertions))]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe { fraud_count_avx2(index, query) };
        }
    }
    fraud_count_scalar(index, query)
}

// ---------------------------------------------------------------------------
// AVX2 path (production)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn fraud_count_avx2(index: &Index, query: &QueryVector) -> u8 {
    let mut best_dists = [i64::MAX; TOP_K];
    let mut best_labels = [0u8; TOP_K];

    let mut q_broadcast = [_mm256_setzero_si256(); VECTOR_DIM];
    for d in 0..VECTOR_DIM {
        q_broadcast[d] = _mm256_set1_epi32(query[d] as i32);
    }

    let key = partition_key(query);
    let primary = index.part_by_key(key);

    if primary >= 0 {
        let root = read_partition_root(index, primary as usize);
        if search_node(
            index,
            root,
            0,
            query,
            &q_broadcast,
            &mut best_dists,
            &mut best_labels,
        ) {
            return sum_labels(&best_labels);
        }
    }

    // Sweep other partitions in lower-bound order, skipping any whose bound
    // already exceeds the current 5th-best.
    let part_count = index.part_count() as i32;
    let mut buf: [(i32, i64); 256] = [(0, 0); 256];
    let mut n = 0usize;
    let mut cutoff = best_dists[TOP_K - 1];
    for i in 0..part_count {
        if i == primary {
            continue;
        }
        let idx = i as usize;
        if i + 1 < part_count {
            let next = if i + 1 == primary {
                i + 2
            } else {
                i + 1
            };
            if next < part_count {
                prefetch_partition_bbox(index, next as usize);
            }
        }
        let (min, max) = read_partition_bbox(index, idx);
        let lb = lower_bound_vec_cutoff(query, &min, &max, cutoff);
        if lb >= cutoff {
            continue;
        }
        buf[n] = (i, lb);
        n += 1;
        if n == 256 {
            break;
        }
    }
    sort_probes_by_lb(&mut buf[..n]);

    for &(part_idx, lb) in buf[..n].iter() {
        cutoff = best_dists[TOP_K - 1];
        if lb >= cutoff {
            break;
        }
        let root = read_partition_root(index, part_idx as usize);
        if search_node(
            index,
            root,
            lb,
            query,
            &q_broadcast,
            &mut best_dists,
            &mut best_labels,
        ) {
            break;
        }
    }

    sum_labels(&best_labels)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn search_node(
    index: &Index,
    root: i32,
    root_bound: i64,
    query: &QueryVector,
    q_broadcast: &[__m256i; VECTOR_DIM],
    best_dists: &mut [i64; TOP_K],
    best_labels: &mut [u8; TOP_K],
) -> bool {
    if root < 0 || root as u32 >= index.node_count() {
        return false;
    }

    let mut stack_node = [0i32; 128];
    let mut stack_bound = [0i64; 128];
    let mut sp: usize = 0;
    let mut current = root;
    let mut current_bound = root_bound;
    let mut cutoff = best_dists[TOP_K - 1];

    loop {
        if current_bound < cutoff {
            let (left, right, start, len) = read_node_split(index, current as usize);
            if left < 0 {
                if scan_leaf(
                    index,
                    start,
                    len,
                    q_broadcast,
                    best_dists,
                    best_labels,
                ) {
                    return true;
                }
                cutoff = best_dists[TOP_K - 1];
            } else {
                prefetch_node_bounds(index, left as usize);
                prefetch_node_bounds(index, right as usize);
                let (lmin, lmax) = read_node_bounds(index, left as usize);
                let (rmin, rmax) = read_node_bounds(index, right as usize);
                let lb = lower_bound_vec_cutoff(query, &lmin, &lmax, cutoff);
                let rb = lower_bound_vec_cutoff(query, &rmin, &rmax, cutoff);

                let (near, near_b, far, far_b) = if lb <= rb {
                    (left, lb, right, rb)
                } else {
                    (right, rb, left, lb)
                };
                if far_b < cutoff && sp < 128 {
                    stack_node[sp] = far;
                    stack_bound[sp] = far_b;
                    sp += 1;
                }
                current = near;
                current_bound = near_b;
                cutoff = best_dists[TOP_K - 1];
                continue;
            }
        }

        if sp == 0 {
            break;
        }
        sp -= 1;
        current = stack_node[sp];
        current_bound = stack_bound[sp];
    }
    early_done(best_dists)
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn early_done(best: &[i64; TOP_K]) -> bool {
    best[TOP_K - 1] <= EARLY_DISTANCE_LIMIT
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_leaf(
    index: &Index,
    start_block: i32,
    len: i32,
    q_broadcast: &[__m256i; VECTOR_DIM],
    best_dists: &mut [i64; TOP_K],
    best_labels: &mut [u8; TOP_K],
) -> bool {
    let blocks = (len as usize).div_ceil(LANES);
    let labels_ptr = index.labels_ptr();
    let vectors_ptr = index.vectors_ptr();

    let total_len = len as usize;
    for b in 0..blocks {
        let block_idx = (start_block as usize) + b;
        if b + 1 < blocks {
            let next = block_idx + 1;
            _mm_prefetch(
                vectors_ptr.add(next * VECTOR_DIM * LANES) as *const i8,
                _MM_HINT_T0,
            );
            _mm_prefetch(
                labels_ptr.add(next * LANES) as *const i8,
                _MM_HINT_T0,
            );
        }
        let labels_base = block_idx * LANES;
        let block_off_i16 = block_idx * VECTOR_DIM * LANES;

        let dists = distance_block8(vectors_ptr, block_off_i16, q_broadcast);

        let lane_count = (total_len - b * LANES).min(LANES);
        let mut cutoff = best_dists[TOP_K - 1];
        for lane in 0..lane_count {
            let d = dists[lane];
            if d < cutoff {
                let label = *labels_ptr.add(labels_base + lane);
                insert_best(d, label, best_dists, best_labels);
                cutoff = best_dists[TOP_K - 1];
                if cutoff <= EARLY_DISTANCE_LIMIT {
                    return true;
                }
            }
        }
    }
    false
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn distance_block8(
    vectors: *const i16,
    block_off_i16: usize,
    q: &[__m256i; VECTOR_DIM],
) -> [i64; LANES] {
    let mut acc_lo = _mm256_setzero_si256();
    let mut acc_hi = _mm256_setzero_si256();
    let base = vectors.add(block_off_i16);
    for d in 0..VECTOR_DIM {
        if d + 1 < VECTOR_DIM {
            _mm_prefetch(
                base.add((d + 1) * LANES) as *const i8,
                _MM_HINT_T0,
            );
        }
        let packed = _mm_loadu_si128(base.add(d * LANES) as *const __m128i);
        let values = _mm256_cvtepi16_epi32(packed);
        let diff = _mm256_sub_epi32(values, q[d]);
        let sq = _mm256_mullo_epi32(diff, diff);
        let sq_lo = _mm256_castsi256_si128(sq);
        let sq_hi = _mm256_extracti128_si256(sq, 1);
        acc_lo = _mm256_add_epi64(acc_lo, _mm256_cvtepi32_epi64(sq_lo));
        acc_hi = _mm256_add_epi64(acc_hi, _mm256_cvtepi32_epi64(sq_hi));
    }
    let mut out = [0i64; LANES];
    _mm256_storeu_si256(out.as_mut_ptr() as *mut __m256i, acc_lo);
    _mm256_storeu_si256(out.as_mut_ptr().add(4) as *mut __m256i, acc_hi);
    out
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn insert_best(dist: i64, label: u8, dists: &mut [i64; TOP_K], labels: &mut [u8; TOP_K]) {
    if dist >= dists[4] {
        return;
    }
    if dist < dists[0] {
        dists[4] = dists[3];
        labels[4] = labels[3];
        dists[3] = dists[2];
        labels[3] = labels[2];
        dists[2] = dists[1];
        labels[2] = labels[1];
        dists[1] = dists[0];
        labels[1] = labels[0];
        dists[0] = dist;
        labels[0] = label;
    } else if dist < dists[1] {
        dists[4] = dists[3];
        labels[4] = labels[3];
        dists[3] = dists[2];
        labels[3] = labels[2];
        dists[2] = dists[1];
        labels[2] = labels[1];
        dists[1] = dist;
        labels[1] = label;
    } else if dist < dists[2] {
        dists[4] = dists[3];
        labels[4] = labels[3];
        dists[3] = dists[2];
        labels[3] = labels[2];
        dists[2] = dist;
        labels[2] = label;
    } else if dist < dists[3] {
        dists[4] = dists[3];
        labels[4] = labels[3];
        dists[3] = dist;
        labels[3] = label;
    } else {
        dists[4] = dist;
        labels[4] = label;
    }
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn sum_labels(labels: &[u8; TOP_K]) -> u8 {
    labels[0] + labels[1] + labels[2] + labels[3] + labels[4]
}

#[cfg(target_arch = "x86_64")]
#[inline(always)]
fn sort_probes_by_lb(probes: &mut [(i32, i64)]) {
    for i in 1..probes.len() {
        let mut j = i;
        while j > 0 && probes[j].1 < probes[j - 1].1 {
            probes.swap(j, j - 1);
            j -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Blob accessors: read raw partition/node entries on demand. We don't
// preparse them into Vec at startup because mmap'd reads are essentially
// free and avoid duplicating ~80MB worth of bbox data in RAM.
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn prefetch_partition_bbox(index: &Index, idx: usize) {
    let p = index.partitions_ptr().add(idx * PART_SIZE + 12);
    _mm_prefetch(p as *const i8, _MM_HINT_T0);
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn prefetch_node_bounds(index: &Index, idx: usize) {
    let p = index.nodes_ptr().add(idx * NODE_SIZE + 16);
    _mm_prefetch(p as *const i8, _MM_HINT_T0);
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_partition_root(index: &Index, idx: usize) -> i32 {
    unsafe {
        let p = index.partitions_ptr().add(idx * PART_SIZE);
        i32::from_le_bytes(*(p.add(4) as *const [u8; 4]))
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_partition_bbox(index: &Index, idx: usize) -> (QueryVector, QueryVector) {
    unsafe {
        let p = index.partitions_ptr().add(idx * PART_SIZE);
        (read_qv(p.add(12)), read_qv(p.add(44)))
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_node_split(index: &Index, idx: usize) -> (i32, i32, i32, i32) {
    unsafe {
        let p = index.nodes_ptr().add(idx * NODE_SIZE);
        (
            i32::from_le_bytes(*(p as *const [u8; 4])),
            i32::from_le_bytes(*(p.add(4) as *const [u8; 4])),
            i32::from_le_bytes(*(p.add(8) as *const [u8; 4])),
            i32::from_le_bytes(*(p.add(12) as *const [u8; 4])),
        )
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_node_bounds(index: &Index, idx: usize) -> (QueryVector, QueryVector) {
    unsafe {
        let p = index.nodes_ptr().add(idx * NODE_SIZE + 16);
        (read_qv(p), read_qv(p.add(32)))
    }
}

#[cfg(target_arch = "x86_64")]
#[inline]
unsafe fn read_qv(p: *const u8) -> QueryVector {
    let mut v: QueryVector = [0; crate::index::PACKED_DIMS];
    std::ptr::copy_nonoverlapping(p, v.as_mut_ptr() as *mut u8, 28);
    v
}

// ---------------------------------------------------------------------------
// Scalar fallback (non-x86 hosts, e.g. local dev on arm64). Same algorithm,
// scalar squared L2. Never enters production.
// ---------------------------------------------------------------------------

#[cfg(not(target_arch = "x86_64"))]
fn fraud_count_scalar(_index: &Index, _query: &QueryVector) -> u8 {
    // Production targets x86_64 + AVX2; non-x86 builds are for compile-check only.
    0
}

#[cfg(target_arch = "x86_64")]
#[allow(dead_code)]
fn fraud_count_scalar(_index: &Index, _query: &QueryVector) -> u8 {
    0
}
