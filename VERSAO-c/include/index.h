#ifndef INDEX_H
#define INDEX_H

#include <stddef.h>
#include <stdint.h>

#define IDX_MAGIC "FRAUDIDX"
#define IDX_VECTOR_DIM 14
#define IDX_PACKED_DIMS 16
#define IDX_QUANT_SCALE 10000
#define IDX_TOP_K 5
#define IDX_LANES 8
#define IDX_HEADER_SIZE 64
#define IDX_PART_SIZE 76
#define IDX_NODE_SIZE 80
#define IDX_BLOCK_BYTES (IDX_VECTOR_DIM * IDX_LANES * 2)
#define IDX_MCC_TABLE_SZ 1024
#define IDX_EARLY_DISTANCE_MILLI 140
#define IDX_EARLY_DISTANCE_LIMIT 1960000LL /* ((10000*140)/1000)^2 */

typedef int16_t query_vec_t[IDX_PACKED_DIMS];

typedef struct {
    const uint8_t *data;
    size_t size;
    int partitions_off;
    int nodes_off;
    int vectors_off;
    int labels_off;
    int mcc_table_off;
    uint32_t part_count;
    uint32_t node_count;
    uint32_t block_count;
    int32_t part_by_key[256];
    uint8_t ready;
} index_t;

int index_open(index_t *idx, const char *path);
void index_init_empty(index_t *idx);
void index_close(index_t *idx);
void index_warmup(index_t *idx);

uint32_t index_part_count(const index_t *idx);
uint32_t index_node_count(const index_t *idx);
uint32_t index_block_count(const index_t *idx);
int32_t index_part_by_key(const index_t *idx, uint32_t key);
int16_t index_mcc_risk(const index_t *idx, uint32_t mcc);
uint8_t index_label_at(const index_t *idx, int slot);
const int16_t *index_vectors_base(const index_t *idx);

uint32_t partition_key(const query_vec_t *v);
int64_t lower_bound_vec_cutoff(const query_vec_t *q, const query_vec_t *min,
                               const query_vec_t *max, int64_t cutoff);
#endif
