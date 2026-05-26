/* Valida tier_fraud_count vs expected_approved (mesmo critério do verify-tier Rust). */
#define _GNU_SOURCE
#include "ingest.h"
#include "tier_score.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Leitor JSON mínimo: só extrai blocos "request" e "expected_approved" por entrada. */
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

static int read_expected_approved(const char *entry)
{
    const char *k = strstr(entry, "\"expected_approved\"");
    if (!k) return -1;
    const char *t = strchr(k, ':');
    if (!t) return -1;
    t++;
    while (*t == ' ' || *t == '\t') t++;
    if (strncmp(t, "true", 4) == 0) return 1;
    if (strncmp(t, "false", 5) == 0) return 0;
    return -1;
}

int main(int argc, char **argv)
{
    const char *path = argc > 1 ? argv[1] : "test/test-data.json";
    FILE *f = fopen(path, "rb");
    if (!f) {
        perror(path);
        return 1;
    }
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = malloc((size_t)sz + 1);
    if (!buf || fread(buf, 1, (size_t)sz, f) != (size_t)sz) {
        fclose(f);
        return 1;
    }
    fclose(f);
    buf[sz] = '\0';

    unsigned long fp = 0, fn = 0, parse_fail = 0, n = 0;
    const char *entries = strstr(buf, "\"entries\"");
    if (!entries) {
        fprintf(stderr, "no entries array\n");
        return 1;
    }
    const char *p = strchr(entries, '[');
    if (!p) return 1;
    p++;
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

        const char *req;
        size_t req_len;
        int exp = read_expected_approved(entry);
        if (exp < 0 || !extract_request_slice(entry, &req, &req_len)) {
            parse_fail++;
        } else {
            raw_payload_t payload;
            if (!extract_json((const uint8_t *)req, req_len, &payload)) {
                parse_fail++;
            } else {
                uint8_t count = tier_fraud_count(&payload);
                int approved = count <= 2;
                if (approved && !exp) fn++;
                if (!approved && exp) fp++;
            }
        }
        n++;
        free(entry);
        p = eend;
    }
    free(buf);
    fprintf(stderr, "entries=%lu fp=%lu fn=%lu parse_fail=%lu\n", n, fp, fn, parse_fail);
    return (fp > 0 || fn > 0) ? 1 : 0;
}
