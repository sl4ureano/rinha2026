use anyhow::{anyhow, Context, Result};
use memmap2::Mmap;
use super::{
    IndexHeader, BLOCK_BYTES, HEADER_SIZE, LANES, MAGIC, MCC_TABLE_SIZE, NODE_SIZE, PACKED_DIMS,
    PART_SIZE, QUANT_SCALE, VECTOR_DIM,
};
use std::fs::File;
use std::path::Path;

#[cfg(target_os = "linux")]
const MADV_HUGEPAGE: libc::c_int = 14;
#[cfg(target_os = "linux")]
const MADV_RANDOM: libc::c_int = 1;
#[cfg(target_os = "linux")]
const MADV_WILLNEED: libc::c_int = 3;

pub struct Index {
    _mmap: Mmap,
    base: *const u8,
    len: usize,
    partitions_off: usize,
    nodes_off: usize,
    vectors_off: usize,
    labels_off: usize,
    mcc_table_off: usize,
    part_count: u32,
    node_count: u32,
    block_count: u32,
    part_by_key: [i32; 256],
}

unsafe impl Send for Index {}
unsafe impl Sync for Index {}

impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path).with_context(|| format!("open {}", path.display()))?;
        let mmap = unsafe { Mmap::map(&f) }?;
        let base = mmap.as_ptr();
        let len = mmap.len();

        if len < HEADER_SIZE {
            return Err(anyhow!("index file too small"));
        }
        let header: &IndexHeader = unsafe { &*(base as *const IndexHeader) };
        if header.magic != MAGIC {
            return Err(anyhow!("bad magic {:?}", header.magic));
        }
        if header.scale != QUANT_SCALE as u32 {
            return Err(anyhow!("scale mismatch (got {})", header.scale));
        }
        if header.dims as usize != VECTOR_DIM
            || header.packed_dims as usize != PACKED_DIMS
            || header.lanes as usize != LANES
        {
            return Err(anyhow!("dim/lane mismatch"));
        }

        let part_count = header.part_count;
        let node_count = header.node_count;
        let block_count = header.block_count;

        let partitions_off = HEADER_SIZE;
        let nodes_off = partitions_off + part_count as usize * PART_SIZE;
        let vectors_off = nodes_off + node_count as usize * NODE_SIZE;
        let labels_off = vectors_off + block_count as usize * BLOCK_BYTES;
        let mcc_table_off = labels_off + block_count as usize * LANES;
        let end = mcc_table_off + MCC_TABLE_SIZE * 2;
        if end != len {
            return Err(anyhow!(
                "index size mismatch (computed {}, file {})",
                end,
                len
            ));
        }
        if header.mcc_table_offset as usize != mcc_table_off {
            return Err(anyhow!(
                "mcc table offset mismatch (header {}, computed {})",
                header.mcc_table_offset,
                mcc_table_off
            ));
        }

        // Per-key partition lookup table for O(1) primary-partition select.
        let mut part_by_key = [-1i32; 256];
        for i in 0..part_count as usize {
            let off = partitions_off + i * PART_SIZE;
            let key = u32::from_le_bytes(unsafe { *(base.add(off) as *const [u8; 4]) });
            if (key as usize) < 256 {
                part_by_key[key as usize] = i as i32;
            }
        }

        let index = Self {
            _mmap: mmap,
            base,
            len,
            partitions_off,
            nodes_off,
            vectors_off,
            labels_off,
            mcc_table_off,
            part_count,
            node_count,
            block_count,
            part_by_key,
        };
        index.advise();
        Ok(index)
    }

    #[cfg(target_os = "linux")]
    fn advise(&self) {
        unsafe {
            libc::madvise(self.base as *mut _, self.len, MADV_HUGEPAGE);
            // The hot region we re-touch every request is just the vectors +
            // labels; mark it WILLNEED so the kernel keeps it resident. The
            // partitions/nodes are tiny and don't need explicit advice.
            let hot_start = self.vectors_off;
            let hot_len = self.len - hot_start;
            libc::madvise(self.base.add(hot_start) as *mut _, hot_len, MADV_HUGEPAGE);
            libc::madvise(self.base.add(hot_start) as *mut _, hot_len, MADV_RANDOM);
            libc::madvise(self.base.add(hot_start) as *mut _, hot_len, MADV_WILLNEED);
        }
    }
    #[cfg(not(target_os = "linux"))]
    fn advise(&self) {}

    #[inline]
    pub fn part_count(&self) -> u32 {
        self.part_count
    }

    #[inline]
    pub fn node_count(&self) -> u32 {
        self.node_count
    }

    #[inline]
    pub fn block_count(&self) -> u32 {
        self.block_count
    }

    #[inline]
    pub fn part_by_key(&self, key: u32) -> i32 {
        self.part_by_key[(key & 0xff) as usize]
    }

    /// Pointer to the start of the partitions table. Each entry is `PART_SIZE`
    /// bytes: key u32, root i32, length i32, min[16] i16, max[16] i16.
    #[inline]
    pub fn partitions_ptr(&self) -> *const u8 {
        unsafe { self.base.add(self.partitions_off) }
    }

    /// Pointer to the start of the nodes table. Each entry is `NODE_SIZE`:
    /// left i32, right i32, start i32, len i32, min[16] i16, max[16] i16.
    #[inline]
    pub fn nodes_ptr(&self) -> *const u8 {
        unsafe { self.base.add(self.nodes_off) }
    }

    /// Pointer to the start of the vectors region (i16 SoA, per-block layout).
    /// Stride within a block: dim 0 lanes 0..7 (16 bytes), dim 1 lanes 0..7, etc.
    #[inline]
    pub fn vectors_ptr(&self) -> *const i16 {
        unsafe { self.base.add(self.vectors_off) as *const i16 }
    }

    /// Pointer to the start of labels (1 byte per slot, `block_count * LANES`
    /// bytes total).
    #[inline]
    pub fn labels_ptr(&self) -> *const u8 {
        unsafe { self.base.add(self.labels_off) }
    }

    /// Look up the i16-scaled MCC risk for an MCC code. Hash by `mcc %
    /// MCC_TABLE_SIZE` to match the builder's index choice.
    #[inline]
    pub fn mcc_risk(&self, mcc: u32) -> i16 {
        let idx = (mcc as usize) % MCC_TABLE_SIZE;
        unsafe {
            let p = self.base.add(self.mcc_table_off + idx * 2);
            i16::from_le_bytes(*(p as *const [u8; 2]))
        }
    }
}
