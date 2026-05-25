//! Construtor offline do índice particionado (KD-tree + blocos SoA).
//!
//! Algorithm:
//! 1. Bucket every reference by `partition_key`. A few dozen buckets typically
//!    emerge (8 bits → 256 max, but most combinations don't exist in the data).
//! 2. Within each bucket, recursively split on the widest dim at the median.
//!    Stop when len ≤ LEAF_SIZE; emit a leaf node pointing into the global
//!    blocks vector. Each non-leaf node spans `left.len + right.len` blocks
//!    so its bbox is the join of its children.
//! 3. After all trees are built, pack the leaf vectors into block-grouped
//!    SoA: per block, dim 0 of all 8 lanes contiguous, then dim 1, etc.
//! 4. Serialize header + partitions + nodes + vectors + labels.

use crate::index::{
    partition_key, IndexHeader, QueryVector, BLOCK_BYTES, HEADER_SIZE, LANES, MAGIC,
    MCC_TABLE_SIZE, NODE_SIZE, PACKED_DIMS, PART_SIZE, QUANT_SCALE, VECTOR_DIM, VERSION,
};

pub const DEFAULT_LEAF_SIZE: usize = 128;

pub struct BuildInputs<'a> {
    pub vectors: &'a [QueryVector],
    pub labels: &'a [u8],
    /// MCC → risk i16 table (`MCC_TABLE_SIZE` entries). Server looks this up
    /// per query at runtime so build/query agree on partition_key.
    pub mcc_table: &'a [i16; MCC_TABLE_SIZE],
}

#[derive(Clone, Copy)]
struct BuildNode {
    left: i32,
    right: i32,
    start: i32, // index into `blocks` in *Reference units* (not block units yet)
    len: i32,
    min: QueryVector,
    max: QueryVector,
}

#[derive(Clone, Copy)]
struct PartitionRoot {
    key: u32,
    root: i32,
}

pub fn build_index(input: &BuildInputs) -> Vec<u8> {
    build_index_with_leaf(input, DEFAULT_LEAF_SIZE)
}

pub fn build_index_with_leaf(input: &BuildInputs, leaf_size: usize) -> Vec<u8> {
    assert_eq!(
        input.vectors.len(),
        input.labels.len(),
        "vectors and labels length mismatch"
    );
    let leaf_size = leaf_size.clamp(32, 2048);

    // 1. Bucket by partition_key.
    let mut buckets: Vec<Vec<usize>> = vec![Vec::new(); 256];
    for (i, v) in input.vectors.iter().enumerate() {
        let key = partition_key(v) as usize;
        buckets[key].push(i);
    }

    // 2. Build trees. `blocks` holds (vector, label) per slot in leaf-emission
    // order. `nodes` is the global node array. `roots` records which node is
    // the root of each non-empty partition.
    let mut nodes: Vec<BuildNode> = Vec::new();
    let mut blocks: Vec<(QueryVector, u8)> = Vec::with_capacity(input.vectors.len() + LANES);
    let mut roots: Vec<PartitionRoot> = Vec::new();

    for (key, indices) in buckets.iter().enumerate() {
        if indices.is_empty() {
            continue;
        }
        let root = build_tree(
            input.vectors,
            input.labels,
            indices,
            leaf_size,
            &mut blocks,
            &mut nodes,
        );
        roots.push(PartitionRoot {
            key: key as u32,
            root: root as i32,
        });
    }

    // Sanity: blocks emitted must be a multiple of LANES (we padded each leaf).
    assert_eq!(blocks.len() % LANES, 0, "block padding broken");
    let block_count = blocks.len() / LANES;

    // 3. Serialize.
    let parts_off = HEADER_SIZE;
    let nodes_off = parts_off + roots.len() * PART_SIZE;
    let vectors_off = nodes_off + nodes.len() * NODE_SIZE;
    let labels_off = vectors_off + block_count * BLOCK_BYTES;
    let mcc_table_off = labels_off + block_count * LANES;
    let total = mcc_table_off + MCC_TABLE_SIZE * 2;
    let mut out = vec![0u8; total];

    // Header
    {
        let h = IndexHeader {
            magic: MAGIC,
            scale: QUANT_SCALE as u32,
            dims: VECTOR_DIM as u32,
            packed_dims: PACKED_DIMS as u32,
            lanes: LANES as u32,
            ref_count: input.vectors.len() as u32,
            part_count: roots.len() as u32,
            node_count: nodes.len() as u32,
            block_count: block_count as u32,
            mcc_table_offset: mcc_table_off as u32,
            _padding: [0u8; 20],
        };
        let _ = h.scale; // version is implicit in MAGIC for this format
        let _ = VERSION;
        let bytes: [u8; HEADER_SIZE] = unsafe { std::mem::transmute(h) };
        out[..HEADER_SIZE].copy_from_slice(&bytes);
    }

    // Partitions: key u32, root i32, length i32, min[16], max[16]
    for (i, r) in roots.iter().enumerate() {
        let off = parts_off + i * PART_SIZE;
        let n = &nodes[r.root as usize];
        out[off..off + 4].copy_from_slice(&r.key.to_le_bytes());
        out[off + 4..off + 8].copy_from_slice(&r.root.to_le_bytes());
        out[off + 8..off + 12].copy_from_slice(&n.len.to_le_bytes());
        write_vec16(&mut out[off + 12..off + 44], &n.min);
        write_vec16(&mut out[off + 44..off + 76], &n.max);
    }

    // Nodes
    for (i, n) in nodes.iter().enumerate() {
        let off = nodes_off + i * NODE_SIZE;
        out[off..off + 4].copy_from_slice(&n.left.to_le_bytes());
        out[off + 4..off + 8].copy_from_slice(&n.right.to_le_bytes());
        // `start` in BuildNode is in slot units; convert to block index here.
        // Same conversion at write time as at query time.
        let start_block = if n.left < 0 {
            n.start / LANES as i32
        } else {
            n.start
        };
        out[off + 8..off + 12].copy_from_slice(&start_block.to_le_bytes());
        out[off + 12..off + 16].copy_from_slice(&n.len.to_le_bytes());
        write_vec16(&mut out[off + 16..off + 48], &n.min);
        write_vec16(&mut out[off + 48..off + 80], &n.max);
    }

    // Vectors: per block, dim 0 of LANES lanes, dim 1 of LANES lanes, ..., dim 13.
    for b in 0..block_count {
        let block_off = vectors_off + b * BLOCK_BYTES;
        for d in 0..VECTOR_DIM {
            let dim_off = block_off + d * LANES * 2;
            for lane in 0..LANES {
                let slot = b * LANES + lane;
                let val = blocks[slot].0[d];
                out[dim_off + lane * 2..dim_off + lane * 2 + 2].copy_from_slice(&val.to_le_bytes());
            }
        }
    }

    // Labels: 1 byte per slot
    for b in 0..block_count {
        let base = labels_off + b * LANES;
        for lane in 0..LANES {
            out[base + lane] = blocks[b * LANES + lane].1;
        }
    }

    // MCC risk table: MCC_TABLE_SIZE × i16 little-endian
    for (i, &v) in input.mcc_table.iter().enumerate() {
        let off = mcc_table_off + i * 2;
        out[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }

    out
}

fn write_vec16(dst: &mut [u8], v: &QueryVector) {
    debug_assert_eq!(dst.len(), 32);
    for i in 0..PACKED_DIMS {
        dst[i * 2..i * 2 + 2].copy_from_slice(&v[i].to_le_bytes());
    }
}

fn bounds(vectors: &[QueryVector], indices: &[usize]) -> (QueryVector, QueryVector) {
    let mut lo: QueryVector = [i16::MAX; PACKED_DIMS];
    let mut hi: QueryVector = [i16::MIN; PACKED_DIMS];
    for &i in indices {
        let v = &vectors[i];
        for d in 0..PACKED_DIMS {
            if v[d] < lo[d] {
                lo[d] = v[d];
            }
            if v[d] > hi[d] {
                hi[d] = v[d];
            }
        }
    }
    (lo, hi)
}

fn widest_dim(lo: &QueryVector, hi: &QueryVector) -> usize {
    let mut best = 0;
    let mut best_w = i32::MIN;
    for d in 0..VECTOR_DIM {
        let w = hi[d] as i32 - lo[d] as i32;
        if w > best_w {
            best_w = w;
            best = d;
        }
    }
    best
}

fn build_tree(
    vectors: &[QueryVector],
    labels: &[u8],
    indices: &[usize],
    leaf_size: usize,
    blocks: &mut Vec<(QueryVector, u8)>,
    nodes: &mut Vec<BuildNode>,
) -> usize {
    let (lo, hi) = bounds(vectors, indices);

    let node_idx = nodes.len();
    nodes.push(BuildNode {
        left: -1,
        right: -1,
        start: 0,
        len: indices.len() as i32,
        min: lo,
        max: hi,
    });

    if indices.len() <= leaf_size {
        // Emit leaf: append vectors padded up to a multiple of LANES.
        let start_slot = blocks.len() as i32;
        for &i in indices {
            blocks.push((vectors[i], labels[i]));
        }
        // Pad to LANES boundary with zero (label=0). These dummy vectors have
        // huge distance (or 0?) — actually they have ZERO distance so they'd
        // win the top-5. We need to flag them so they don't get selected.
        // Zero-padded vectors; distance scoring uses the same layout at search time.
        // because real vectors with sentinel -SCALE in dim 5/6 are far from
        // (0, 0, ..., 0). But pad slots have label=0 and distance 0 → wrong.
        //
        // Fix: pad with i16::MAX in all dims so distance is enormous.
        while blocks.len() % LANES != 0 {
            blocks.push(([i16::MAX; PACKED_DIMS], 0));
        }
        let node = &mut nodes[node_idx];
        node.left = -1;
        node.right = -1;
        node.start = start_slot;
        node.len = indices.len() as i32;
        return node_idx;
    }

    let split_dim = widest_dim(&lo, &hi);
    let mut sorted = indices.to_vec();
    sorted.sort_unstable_by_key(|&i| vectors[i][split_dim]);
    let mid = sorted.len() / 2;
    let (left_idx, right_idx) = sorted.split_at(mid);

    let left = build_tree(vectors, labels, left_idx, leaf_size, blocks, nodes);
    let right = build_tree(vectors, labels, right_idx, leaf_size, blocks, nodes);

    let left_start = nodes[left].start;
    let total_len = nodes[left].len + nodes[right].len;
    let node = &mut nodes[node_idx];
    node.left = left as i32;
    node.right = right as i32;
    node.start = left_start;
    node.len = total_len;
    node_idx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_vec(seed: i16) -> QueryVector {
        let mut v: QueryVector = [0; PACKED_DIMS];
        for d in 0..VECTOR_DIM {
            v[d] = seed.wrapping_mul((d as i16) + 1).wrapping_add(d as i16);
        }
        v
    }

    #[test]
    fn build_small_blob_round_trip() {
        let mut vectors = Vec::new();
        let mut labels = Vec::new();
        for i in 0..600 {
            vectors.push(mk_vec(i as i16));
            labels.push((i % 5 == 0) as u8);
        }
        let mcc_table = [0i16; MCC_TABLE_SIZE];
        let bytes = build_index_with_leaf(
            &BuildInputs {
                vectors: &vectors,
                labels: &labels,
                mcc_table: &mcc_table,
            },
            64,
        );
        assert!(bytes.len() > HEADER_SIZE);
        // Header sanity
        assert_eq!(&bytes[..8], &MAGIC);
        // dims/scale/packed_dims/lanes are at fixed offsets
        let scale = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        assert_eq!(scale, QUANT_SCALE as u32);
        let dims = u32::from_le_bytes(bytes[12..16].try_into().unwrap());
        assert_eq!(dims, VECTOR_DIM as u32);
    }
}
