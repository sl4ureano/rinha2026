/* Lê JSON do stdin e imprime tier_fraud_count (uma linha). */
#define _GNU_SOURCE
#include "ingest.h"
#include "tier_score.h"

#include <stdio.h>
#include <stdlib.h>

int main(void)
{
    size_t cap = 65536, len = 0;
    uint8_t *buf = malloc(cap);
    if (!buf) return 1;
    for (;;) {
        size_t n = fread(buf + len, 1, cap - len, stdin);
        len += n;
        if (n == 0) break;
        if (len < cap) break;
        cap *= 2;
        uint8_t *p = realloc(buf, cap);
        if (!p) return 1;
        buf = p;
    }
    raw_payload_t p;
    if (!extract_json(buf, len, &p)) {
        fputs("parse_fail\n", stderr);
        return 2;
    }
    printf("%u\n", (unsigned)tier_fraud_count(&p));
    free(buf);
    return 0;
}
