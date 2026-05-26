/* Compara tier_fraud_count C (bruto) com linhas exportadas pelo Rust. */
#define _GNU_SOURCE
#include "ingest.h"
#include "tier_score.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static const char *find_key(const char *s, const char *key)
{
    char pat[64];
    snprintf(pat, sizeof(pat), "\"%s\"", key);
    return strstr(s, pat);
}

static int extract_request_slice(const char *entry, const char **out, size_t *olen)
{
    const char *k = find_key(entry, "request");
    if (!k) return 0;
    const char *obj = strchr(k, '{');
    if (!obj) return 0;
    int depth = 0;
    const char *p = obj;
    for (; *p; p++) {
        if (*p == '{') depth++;
        else if (*p == '}') {
            depth--;
            if (depth == 0) {
                *out = obj;
                *olen = (size_t)(p - obj + 1);
                return 1;
            }
        }
    }
    return 0;
}

int main(int argc, char **argv)
{
    const char *data_path = argc > 1 ? argv[1] : "/repo/test/test-data.json";
    const char *counts_path = argc > 2 ? argv[2] : "/repo/test/rust_counts.txt";

    FILE *cf = fopen(counts_path, "r");
    if (!cf) {
        perror(counts_path);
        return 1;
    }

    FILE *f = fopen(data_path, "rb");
    if (!f) {
        perror(data_path);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc((size_t)sz + 1);
    fread(buf, 1, (size_t)sz, f);
    fclose(f);
    buf[sz] = '\0';

    unsigned long line = 0, diffs = 0;
    const char *entries = strstr(buf, "\"entries\"");
    const char *p = strchr(entries, '[') + 1;
    while (*p) {
        while (*p && (*p == ' ' || *p == ',' || *p == '\n' || *p == '\r')) p++;
        if (*p == ']') break;
        if (*p != '{') break;
        const char *estart = p;
        int depth = 0;
        const char *eend = p;
        for (; *eend; eend++) {
            if (*eend == '{') depth++;
            else if (*eend == '}') {
                depth--;
                if (depth == 0) {
                    eend++;
                    break;
                }
            }
        }
        size_t elen = (size_t)(eend - estart);
        char *entry = malloc(elen + 1);
        memcpy(entry, estart, elen);
        entry[elen] = '\0';

        int expect = -1;
        if (fscanf(cf, "%d", &expect) != 1) {
            fprintf(stderr, "eof counts at line %lu\n", line);
            break;
        }

        const char *req;
        size_t req_len;
        if (extract_request_slice(entry, &req, &req_len)) {
            raw_payload_t payload;
            if (extract_json((const uint8_t *)req, req_len, &payload)) {
                unsigned got = tier_fraud_count(&payload);
                if ((int)got != expect) {
                    if (diffs < 10)
                        fprintf(stderr, "line=%lu rust=%d c=%u\n", line, expect, got);
                    diffs++;
                }
            }
        }
        line++;
        free(entry);
        p = eend;
    }
    fclose(cf);
    free(buf);
    fprintf(stderr, "diffs=%lu lines=%lu\n", diffs, line);
    return diffs > 0 ? 1 : 0;
}
