#define _GNU_SOURCE
#include "index.h"

#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>

void index_init_empty(index_t *idx) { memset(idx, 0, sizeof(*idx)); }

int index_open(index_t *idx, const char *path)
{
    int fd = open(path, O_RDONLY);
    if (fd < 0) return -1;

    struct stat st;
    if (fstat(fd, &st) != 0) {
        close(fd);
        return -1;
    }
    size_t size = (size_t)st.st_size;
    if (size < IDX_HEADER_SIZE) {
        close(fd);
        return -1;
    }

    void *data = mmap(NULL, size, PROT_READ, MAP_SHARED, fd, 0);
    close(fd);
    if (data == MAP_FAILED) return -1;

    const uint8_t *d = (const uint8_t *)data;
    if (memcmp(d, IDX_MAGIC, 8) != 0) {
        munmap(data, size);
        return -1;
    }

    uint32_t scale, dims, packed, lanes, part_count, node_count, block_count, mcc_off;
    memcpy(&scale, d + 8, 4);
    memcpy(&dims, d + 12, 4);
    memcpy(&packed, d + 16, 4);
    memcpy(&lanes, d + 20, 4);
    memcpy(&part_count, d + 28, 4);
    memcpy(&node_count, d + 32, 4);
    memcpy(&block_count, d + 36, 4);
    memcpy(&mcc_off, d + 40, 4);

    if (scale != (uint32_t)IDX_QUANT_SCALE || dims != IDX_VECTOR_DIM ||
        packed != IDX_PACKED_DIMS || lanes != IDX_LANES) {
        munmap(data, size);
        return -1;
    }

    int partitions_off = IDX_HEADER_SIZE;
    int nodes_off = partitions_off + (int)part_count * IDX_PART_SIZE;
    int vectors_off = nodes_off + (int)node_count * IDX_NODE_SIZE;
    int labels_off = vectors_off + (int)block_count * IDX_BLOCK_BYTES;
    int mcc_table_off = labels_off + (int)block_count * IDX_LANES;
    int end = mcc_table_off + IDX_MCC_TABLE_SZ * 2;
    if (end != (int)size || (int)mcc_off != mcc_table_off) {
        munmap(data, size);
        return -1;
    }

    memset(idx, 0, sizeof(*idx));
    idx->data = d;
    idx->size = size;
    idx->partitions_off = partitions_off;
    idx->nodes_off = nodes_off;
    idx->vectors_off = vectors_off;
    idx->labels_off = labels_off;
    idx->mcc_table_off = mcc_table_off;
    idx->part_count = part_count;
    idx->node_count = node_count;
    idx->block_count = block_count;

    for (uint32_t i = 0; i < part_count; i++) {
        const uint8_t *p = d + partitions_off + (int)i * IDX_PART_SIZE;
        uint32_t key;
        memcpy(&key, p, 4);
        if (key < 256) idx->part_by_key[key] = (int32_t)i;
    }

    madvise((void *)data, size, MADV_HUGEPAGE);
    madvise((void *)(d + vectors_off), size - (size_t)vectors_off, MADV_HUGEPAGE | MADV_RANDOM);
    return 0;
}

void index_close(index_t *idx)
{
    if (idx->data) {
        munmap((void *)idx->data, idx->size);
        idx->data = NULL;
    }
}

uint32_t index_part_count(const index_t *idx) { return idx->part_count; }
uint32_t index_node_count(const index_t *idx) { return idx->node_count; }
uint32_t index_block_count(const index_t *idx) { return idx->block_count; }

int32_t index_part_by_key(const index_t *idx, uint32_t key)
{
    return idx->part_by_key[key & 0xff];
}

int16_t index_mcc_risk(const index_t *idx, uint32_t mcc)
{
    int off = idx->mcc_table_off + (int)(mcc % IDX_MCC_TABLE_SZ) * 2;
    uint16_t v;
    memcpy(&v, idx->data + off, 2);
    return (int16_t)v;
}

uint8_t index_label_at(const index_t *idx, int slot)
{
    return idx->data[idx->labels_off + slot];
}

const int16_t *index_vectors_base(const index_t *idx)
{
    return (const int16_t *)(idx->data + idx->vectors_off);
}
