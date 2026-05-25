#include "index.h"
#include "ingest.h"
#include "knn.h"

#include <stdio.h>
#include <stdlib.h>

int main(int argc, char **argv)
{
    if (argc < 3) return 1;
    index_t idx;
    if (index_open(&idx, argv[1]) != 0) return 2;
    FILE *f = fopen(argv[2], "rb");
    if (!f) return 3;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    uint8_t *body = malloc((size_t)sz);
    fread(body, 1, (size_t)sz, f);
    fclose(f);
    raw_payload_t p;
    query_vec_t v;
    if (!extract_json(body, (size_t)sz, &p) || !vectorize_payload(&idx, &p, &v)) {
        fprintf(stderr, "parse/vectorize failed\n");
        return 4;
    }
    uint8_t count = fraud_count(&idx, &v);
    printf("fraud_count=%u vec=", count);
    for (int i = 0; i < IDX_PACKED_DIMS; i++) {
        if (i) putchar(',');
        printf("%d", (int)v[i]);
    }
    putchar('\n');
    index_close(&idx);
    free(body);
    return 0;
}
