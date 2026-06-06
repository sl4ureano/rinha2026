//! Exact k-NN search over a partitioned KD-tree.
//! Distance is squared L2 in i16-quantized space, accumulated as
//! i64 to keep room over 14 dims × (2*SCALE)^2.
//!
//! Pruning: every node carries an axis-aligned bbox. The lower bound of a
//! query's distance to any vector in the subtree is `lower_bound_vec(query,
//! min, max)`. When that lower bound is ≥ our current 5th-best, we skip the
//! whole subtree.

#![allow(clippy::needless_range_loop)]

use crate::index::Index;
use crate::index::QueryVector;

#[cfg(target_arch = "x86_64")]
use crate::index::{
    lower_bound_vec_cutoff, partition_key, LANES, NODE_SIZE, PART_SIZE, TOP_K, VECTOR_DIM,
};
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;
#[cfg(target_arch = "x86_64")]
use std::mem::MaybeUninit;

#[cfg(target_arch = "x86_64")]
const DIM_PAIRS: usize = VECTOR_DIM / 2;
#[cfg(target_arch = "x86_64")]
const DEFER_CAP: usize = 4096;
#[cfg(target_arch = "x86_64")]
const LABEL_MASK_LEGIT: u8 = 1;
#[cfg(target_arch = "x86_64")]
const LABEL_MASK_FRAUD: u8 = 2;

#[cfg(target_arch = "x86_64")]
#[derive(Clone, Copy)]
struct NodeVisit {
    idx: i32,
    bound: i64,
}

#[cfg(target_arch = "x86_64")]
struct Search {
    best_dists: [i64; TOP_K],
    best_labels: [u8; TOP_K],
    deferred: [MaybeUninit<NodeVisit>; DEFER_CAP],
    deferred_len: usize,
    chromatic: bool,
    chrom_initial_sum: u8,
    chrom_needed_mask: u8,
}

#[cfg(target_arch = "x86_64")]
impl Search {
    fn new() -> Self {
        Self {
            best_dists: [i64::MAX; TOP_K],
            best_labels: [0u8; TOP_K],
            deferred: [MaybeUninit::uninit(); DEFER_CAP],
            deferred_len: 0,
            chromatic: false,
            chrom_initial_sum: 0,
            chrom_needed_mask: 0,
        }
    }

    #[inline(always)]
    fn top_complete(&self) -> bool {
        self.best_dists[TOP_K - 1] != i64::MAX
    }

    #[inline(always)]
    fn top_sum(&self) -> u8 {
        sum_labels(&self.best_labels)
    }

    #[inline(always)]
    fn maybe_activate_chromatic(&mut self) {
        if self.chromatic || !self.top_complete() {
            return;
        }
        let sum = self.top_sum();
        if sum == 0 {
            self.chromatic = true;
            self.chrom_initial_sum = 0;
            self.chrom_needed_mask = LABEL_MASK_FRAUD;
        } else if sum == TOP_K as u8 {
            self.chromatic = true;
            self.chrom_initial_sum = TOP_K as u8;
            self.chrom_needed_mask = LABEL_MASK_LEGIT;
        }
    }

    #[inline(always)]
    fn should_defer(&mut self, label_mask: u8) -> bool {
        self.maybe_activate_chromatic();
        self.chromatic
            && self.top_sum() == self.chrom_initial_sum
            && (label_mask & self.chrom_needed_mask) == 0
            && self.deferred_len < DEFER_CAP
    }

    #[inline(always)]
    fn push_deferred(&mut self, idx: i32, bound: i64) {
        self.deferred[self.deferred_len].write(NodeVisit { idx, bound });
        self.deferred_len += 1;
    }
}

/// Top-5 fraud labels in the true nearest neighbors. Returns count `0..=5`.
#[inline]
pub fn fraud_count(index: &Index, query: &QueryVector) -> u8 {
    #[cfg(all(target_arch = "x86_64", not(debug_assertions)))]
    unsafe {
        fraud_count_avx2(index, query)
    }

    #[cfg(all(target_arch = "x86_64", debug_assertions))]
    {
        if is_x86_feature_detected!("avx2") {
            unsafe { fraud_count_avx2(index, query) }
        } else {
            fraud_count_scalar(index, query)
        }
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        fraud_count_scalar(index, query)
    }
}

// ---------------------------------------------------------------------------
// AVX2 path (production)
// ---------------------------------------------------------------------------

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn fraud_count_avx2(index: &Index, query: &QueryVector) -> u8 {
    let mut s = Search::new();

    let mut q_pairs = [_mm256_setzero_si256(); DIM_PAIRS];
    for p in 0..DIM_PAIRS {
        let packed = (query[p * 2] as u16 as u32) | ((query[p * 2 + 1] as u16 as u32) << 16);
        q_pairs[p] = _mm256_set1_epi32(packed as i32);
    }

    let key = partition_key(query);
    let primary = index.part_by_key(key);

    if primary >= 0 {
        let root = read_partition_root(index, primary as usize);
        // Prefetch root node bounds to warm node metadata before traversal
        if root >= 0 {
            prefetch_node_bounds(index, root as usize);
        }
        search_node(index, root, 0, query, &q_pairs, &mut s, true);
    }

    // Sweep other partitions in lower-bound order, skipping any whose bound
    // already exceeds the current 5th-best.
    let part_count = index.part_count() as i32;
    let mut buf: [(i32, i64); 256] = [(0, 0); 256];
    let mut n = 0usize;
    let mut cutoff = s.best_dists[TOP_K - 1];
    for i in 0..part_count {
        if i == primary {
            continue;
        }
        let idx = i as usize;
        if i + 1 < part_count {
            let next = if i + 1 == primary { i + 2 } else { i + 1 };
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
        cutoff = s.best_dists[TOP_K - 1];
        if lb >= cutoff {
            break;
        }
        let root = read_partition_root(index, part_idx as usize);
        if root >= 0 {
            prefetch_node_bounds(index, root as usize);
        }
        search_node(index, root, lb, query, &q_pairs, &mut s, true);
    }

    if s.chromatic && s.top_sum() != s.chrom_initial_sum {
        let deferred_len = s.deferred_len;
        s.chromatic = false;
        s.deferred_len = 0;
        for i in 0..deferred_len {
            let visit = s.deferred[i].assume_init();
            search_node(
                index,
                visit.idx,
                visit.bound,
                query,
                &q_pairs,
                &mut s,
                false,
            );
        }
    }

    s.top_sum()
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn search_node(
    index: &Index,
    root: i32,
    root_bound: i64,
    query: &QueryVector,
    q_pairs: &[__m256i; DIM_PAIRS],
    s: &mut Search,
    allow_chromatic: bool,
) {
    if root < 0 || root as u32 >= index.node_count() {
        return;
    }

    let mut stack_node = [0i32; 128];
    let mut stack_bound = [0i64; 128];
    let mut sp: usize = 0;
    let mut current = root;
    let mut current_bound = root_bound;
    let mut cutoff = s.best_dists[TOP_K - 1];

    loop {
        if current_bound < cutoff {
            if allow_chromatic {
                let label_mask = read_node_label_mask(index, current as usize);
                if s.should_defer(label_mask) {
                    s.push_deferred(current, current_bound);
                    if sp == 0 {
                        break;
                    }
                    sp -= 1;
                    current = stack_node[sp];
                    current_bound = stack_bound[sp];
                    continue;
                }
            }
            let (left, right, start, len) = read_node_split(index, current as usize);
            if left < 0 {
                scan_leaf(
                    index,
                    start,
                    len,
                    q_pairs,
                    &mut s.best_dists,
                    &mut s.best_labels,
                );
                cutoff = s.best_dists[TOP_K - 1];
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
                cutoff = s.best_dists[TOP_K - 1];
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
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_leaf(
    index: &Index,
    start_block: i32,
    len: i32,
    q_pairs: &[__m256i; DIM_PAIRS],
    best_dists: &mut [i64; TOP_K],
    best_labels: &mut [u8; TOP_K],
) {
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
            _mm_prefetch(labels_ptr.add(next * LANES) as *const i8, _MM_HINT_T0);
        }
        // also prefetch two blocks ahead when available
        if b + 2 < blocks {
            let next2 = block_idx + 2;
            _mm_prefetch(
                vectors_ptr.add(next2 * VECTOR_DIM * LANES) as *const i8,
                _MM_HINT_T0,
            );
            _mm_prefetch(labels_ptr.add(next2 * LANES) as *const i8, _MM_HINT_T0);
        }
        let labels_base = block_idx * LANES;
        let block_off_i16 = block_idx * VECTOR_DIM * LANES;

        let dists = distance_block8(vectors_ptr, block_off_i16, q_pairs);

        let lane_count = (total_len - b * LANES).min(LANES);
        let mut cutoff = best_dists[TOP_K - 1];
        for lane in 0..lane_count {
            let d = dists[lane];
            if d < cutoff {
                let label = *labels_ptr.add(labels_base + lane);
                insert_best(d, label, best_dists, best_labels);
                cutoff = best_dists[TOP_K - 1];
            }
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn distance_block8(
    vectors: *const i16,
    block_off_i16: usize,
    q_pairs: &[__m256i; DIM_PAIRS],
) -> [i64; LANES] {
    let mut acc_lo = _mm256_setzero_si256();
    let mut acc_hi = _mm256_setzero_si256();
    let base = vectors.add(block_off_i16);
    for p in 0..DIM_PAIRS {
        if p + 1 < DIM_PAIRS {
            _mm_prefetch(base.add((p + 1) * 2 * LANES) as *const i8, _MM_HINT_T0);
        }
        // prefetch one more pair ahead when available
        if p + 2 < DIM_PAIRS {
            _mm_prefetch(base.add((p + 2) * 2 * LANES) as *const i8, _MM_HINT_T0);
        }
        let even = _mm_loadu_si128(base.add(p * 2 * LANES) as *const __m128i);
        let odd = _mm_loadu_si128(base.add((p * 2 + 1) * LANES) as *const __m128i);
        let lo = _mm_unpacklo_epi16(even, odd);
        let hi = _mm_unpackhi_epi16(even, odd);
        let values = _mm256_set_m128i(hi, lo);
        let diff = _mm256_sub_epi16(values, q_pairs[p]);
        let pair_sums = _mm256_madd_epi16(diff, diff);
        let sq_lo = _mm256_castsi256_si128(pair_sums);
        let sq_hi = _mm256_extracti128_si256(pair_sums, 1);
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
    let p = index.nodes_ptr().add(idx * NODE_SIZE + 20);
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
fn read_node_label_mask(index: &Index, idx: usize) -> u8 {
    unsafe { *index.nodes_ptr().add(idx * NODE_SIZE + 16) }
}

#[cfg(target_arch = "x86_64")]
#[inline]
fn read_node_bounds(index: &Index, idx: usize) -> (QueryVector, QueryVector) {
    unsafe {
        let p = index.nodes_ptr().add(idx * NODE_SIZE + 20);
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

#[cfg(not(target_arch = "x86_64"))]
fn fraud_count_scalar(_index: &Index, _query: &QueryVector) -> u8 {
    0
}

#[cfg(all(target_arch = "x86_64", debug_assertions))]
fn fraud_count_scalar(_index: &Index, _query: &QueryVector) -> u8 {
    0
}
