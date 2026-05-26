#include "ingest.h"

#include <stdlib.h>
#include <stdint.h>
#include <string.h>

typedef struct {
    const uint8_t *buf;
    size_t len;
    size_t i;
} scanner_t;

static int s_at_end(scanner_t *s) { return s->i >= s->len; }
static uint8_t s_peek(scanner_t *s) { return s->i < s->len ? s->buf[s->i] : 0; }
static void s_bump(scanner_t *s) { s->i++; }
static void s_advance(scanner_t *s, size_t n) { s->i += n; }

static int s_expect(scanner_t *s, uint8_t c)
{
    if (s_peek(s) == c) {
        s_bump(s);
        return 1;
    }
    return 0;
}

static void s_skip_ws(scanner_t *s)
{
    for (;;) {
        uint8_t c = s_peek(s);
        if (c == ' ' || c == '\t' || c == '\n' || c == '\r')
            s_bump(s);
        else
            return;
    }
}

static int s_peek_word(scanner_t *s, const char *w)
{
    size_t wl = strlen(w);
    if (s->i + wl > s->len) return 0;
    return memcmp(s->buf + s->i, w, wl) == 0;
}

static int s_read_string(scanner_t *s, const uint8_t **out, size_t *olen)
{
    if (!s_expect(s, '"')) return 0;
    size_t start = s->i;
    while (s->i < s->len) {
        uint8_t c = s->buf[s->i];
        if (c == '"') {
            *out = s->buf + start;
            *olen = s->i - start;
            s_bump(s);
            return 1;
        }
        if (c == '\\') s_bump(s);
        s_bump(s);
    }
    return 0;
}

static int parse_u32_bytes(const uint8_t *b, size_t len, uint32_t *out)
{
    if (len == 0) return 0;
    uint32_t n = 0;
    for (size_t i = 0; i < len; i++) {
        if (b[i] < '0' || b[i] > '9') return 0;
        n = n * 10u + (uint32_t)(b[i] - '0');
    }
    *out = n;
    return 1;
}

static int u64_mul10_add(uint64_t *acc, uint8_t digit)
{
    if (*acc > (UINT64_MAX - (uint64_t)digit) / 10u) return 0;
    *acc = *acc * 10u + (uint64_t)digit;
    return 1;
}

/* Fast path; returns 0 on overflow or non-decimal tail (caller uses strtof). */
static int parse_f32_bytes(const uint8_t *b, size_t len, float *out)
{
    if (len == 0) return 0;
    size_t i = 0;
    int neg = (b[0] == '-');
    if (neg) i++;
    uint64_t int_part = 0;
    while (i < len && b[i] >= '0' && b[i] <= '9') {
        if (!u64_mul10_add(&int_part, b[i] - '0')) return 0;
        i++;
    }
    uint64_t frac = 0, frac_div = 1;
    if (i < len && b[i] == '.') {
        i++;
        while (i < len && b[i] >= '0' && b[i] <= '9') {
            if (!u64_mul10_add(&frac, b[i] - '0')) return 0;
            if (frac_div > UINT64_MAX / 10u) return 0;
            frac_div *= 10u;
            i++;
        }
    }
    if (i != len) return 0;
    double v = (double)int_part;
    if (frac_div) v += (double)frac / (double)frac_div;
    if (neg) v = -v;
    *out = (float)v;
    return 1;
}

static int s_read_f32(scanner_t *s, float *out)
{
    size_t start = s->i;
    while (s->i < s->len) {
        uint8_t c = s->buf[s->i];
        if (c == '-' || c == '+' || c == '.' || (c >= '0' && c <= '9') || c == 'e' || c == 'E')
            s_bump(s);
        else
            break;
    }
    size_t n = s->i - start;
    if (n == 0) return 0;
    if (parse_f32_bytes(s->buf + start, n, out)) return 1;
    if (n >= 64) return 0;
    char tmp[64];
    memcpy(tmp, s->buf + start, n);
    tmp[n] = '\0';
    char *end = tmp;
    float v = strtof(tmp, &end);
    if ((size_t)(end - tmp) != n) return 0;
    *out = v;
    return 1;
}

static int s_read_u32(scanner_t *s, uint32_t *out)
{
    size_t start = s->i;
    while (s->i < s->len && s->buf[s->i] >= '0' && s->buf[s->i] <= '9') s_bump(s);
    return parse_u32_bytes(s->buf + start, s->i - start, out);
}

static int s_read_bool(scanner_t *s, int *out)
{
    if (s_peek_word(s, "true")) {
        s_advance(s, 4);
        *out = 1;
        return 1;
    }
    if (s_peek_word(s, "false")) {
        s_advance(s, 5);
        *out = 0;
        return 1;
    }
    return 0;
}

static int s_read_array_raw(scanner_t *s, const uint8_t **out, size_t *olen)
{
    if (!s_expect(s, '[')) return 0;
    size_t start = s->i;
    int depth = 1, in_str = 0;
    while (s->i < s->len) {
        uint8_t c = s->buf[s->i];
        if (in_str) {
            if (c == '\\') s_bump(s);
            else if (c == '"') in_str = 0;
            s_bump(s);
            continue;
        }
        switch (c) {
        case '"':
            in_str = 1;
            s_bump(s);
            break;
        case '[':
            depth++;
            s_bump(s);
            break;
        case ']':
            depth--;
            if (depth == 0) {
                *out = s->buf + start;
                *olen = s->i - start;
                s_bump(s);
                return 1;
            }
            s_bump(s);
            break;
        default:
            s_bump(s);
            break;
        }
    }
    return 0;
}

static int s_skip_object(scanner_t *s);
static int s_skip_value(scanner_t *s);

static int s_skip_value(scanner_t *s)
{
    s_skip_ws(s);
    uint8_t c = s_peek(s);
    if (c == '"') {
        const uint8_t *t;
        size_t tl;
        return s_read_string(s, &t, &tl);
    }
    if (c == '{') return s_skip_object(s);
    if (c == '[') {
        const uint8_t *t;
        size_t tl;
        return s_read_array_raw(s, &t, &tl);
    }
    if (c == 't' || c == 'f') {
        int b;
        return s_read_bool(s, &b);
    }
    if (c == 'n') {
        if (s_peek_word(s, "null")) {
            s_advance(s, 4);
            return 1;
        }
        return 0;
    }
    float f;
    return s_read_f32(s, &f);
}

static int s_skip_object(scanner_t *s)
{
    if (!s_expect(s, '{')) return 0;
    int depth = 1, in_str = 0;
    while (s->i < s->len) {
        uint8_t c = s->buf[s->i];
        if (in_str) {
            if (c == '\\') s_bump(s);
            else if (c == '"') in_str = 0;
            s_bump(s);
            continue;
        }
        switch (c) {
        case '"':
            in_str = 1;
            s_bump(s);
            break;
        case '{':
            depth++;
            s_bump(s);
            break;
        case '}':
            depth--;
            s_bump(s);
            if (depth == 0) return 1;
            break;
        default:
            s_bump(s);
            break;
        }
    }
    return 0;
}

static int key_eq(const uint8_t *k, size_t kl, const char *s)
{
    size_t sl = strlen(s);
    return kl == sl && memcmp(k, s, sl) == 0;
}

static int parse_transaction(scanner_t *s, raw_payload_t *p);
static int parse_customer(scanner_t *s, raw_payload_t *p);
static int parse_merchant(scanner_t *s, raw_payload_t *p);
static int parse_terminal(scanner_t *s, raw_payload_t *p);
static int parse_last_transaction(scanner_t *s, raw_payload_t *p);

static int parse_transaction(scanner_t *s, raw_payload_t *p)
{
    if (!s_expect(s, '{')) return 0;
    for (;;) {
        s_skip_ws(s);
        if (s_peek(s) == '}') {
            s_bump(s);
            return 1;
        }
        const uint8_t *k;
        size_t kl;
        if (!s_read_string(s, &k, &kl)) return 0;
        s_skip_ws(s);
        if (!s_expect(s, ':')) return 0;
        s_skip_ws(s);
        if (key_eq(k, kl, "amount")) {
            if (!s_read_f32(s, &p->amount)) return 0;
        } else if (key_eq(k, kl, "installments")) {
            if (!s_read_u32(s, &p->installments)) return 0;
        } else if (key_eq(k, kl, "requested_at")) {
            if (!s_read_string(s, &p->requested_at, &p->requested_at_len)) return 0;
        } else if (!s_skip_value(s)) {
            return 0;
        }
        s_skip_ws(s);
        if (s_peek(s) == ',') s_bump(s);
    }
}

static int parse_customer(scanner_t *s, raw_payload_t *p)
{
    if (!s_expect(s, '{')) return 0;
    for (;;) {
        s_skip_ws(s);
        if (s_peek(s) == '}') {
            s_bump(s);
            return 1;
        }
        const uint8_t *k;
        size_t kl;
        if (!s_read_string(s, &k, &kl)) return 0;
        s_skip_ws(s);
        if (!s_expect(s, ':')) return 0;
        s_skip_ws(s);
        if (key_eq(k, kl, "avg_amount")) {
            if (!s_read_f32(s, &p->customer_avg_amount)) return 0;
        } else if (key_eq(k, kl, "tx_count_24h")) {
            if (!s_read_u32(s, &p->tx_count_24h)) return 0;
        } else if (key_eq(k, kl, "known_merchants")) {
            if (!s_read_array_raw(s, &p->known_merchants, &p->known_merchants_len)) return 0;
        } else if (!s_skip_value(s)) {
            return 0;
        }
        s_skip_ws(s);
        if (s_peek(s) == ',') s_bump(s);
    }
}

static int parse_merchant(scanner_t *s, raw_payload_t *p)
{
    if (!s_expect(s, '{')) return 0;
    for (;;) {
        s_skip_ws(s);
        if (s_peek(s) == '}') {
            s_bump(s);
            return 1;
        }
        const uint8_t *k;
        size_t kl;
        if (!s_read_string(s, &k, &kl)) return 0;
        s_skip_ws(s);
        if (!s_expect(s, ':')) return 0;
        s_skip_ws(s);
        if (key_eq(k, kl, "id")) {
            if (!s_read_string(s, &p->merchant_id, &p->merchant_id_len)) return 0;
        } else if (key_eq(k, kl, "mcc")) {
            if (!s_read_string(s, &p->merchant_mcc, &p->merchant_mcc_len)) return 0;
        } else if (key_eq(k, kl, "avg_amount")) {
            if (!s_read_f32(s, &p->merchant_avg_amount)) return 0;
        } else if (!s_skip_value(s)) {
            return 0;
        }
        s_skip_ws(s);
        if (s_peek(s) == ',') s_bump(s);
    }
}

static int parse_terminal(scanner_t *s, raw_payload_t *p)
{
    if (!s_expect(s, '{')) return 0;
    for (;;) {
        s_skip_ws(s);
        if (s_peek(s) == '}') {
            s_bump(s);
            return 1;
        }
        const uint8_t *k;
        size_t kl;
        if (!s_read_string(s, &k, &kl)) return 0;
        s_skip_ws(s);
        if (!s_expect(s, ':')) return 0;
        s_skip_ws(s);
        if (key_eq(k, kl, "is_online")) {
            int b;
            if (!s_read_bool(s, &b)) return 0;
            p->is_online = b != 0;
        } else if (key_eq(k, kl, "card_present")) {
            int b;
            if (!s_read_bool(s, &b)) return 0;
            p->card_present = b != 0;
        } else if (key_eq(k, kl, "km_from_home")) {
            if (!s_read_f32(s, &p->km_from_home)) return 0;
        } else if (!s_skip_value(s)) {
            return 0;
        }
        s_skip_ws(s);
        if (s_peek(s) == ',') s_bump(s);
    }
}

static int parse_last_transaction(scanner_t *s, raw_payload_t *p)
{
    s_skip_ws(s);
    if (s_peek_word(s, "null")) {
        s_advance(s, 4);
        return 1;
    }
    if (!s_expect(s, '{')) return 0;
    for (;;) {
        s_skip_ws(s);
        if (s_peek(s) == '}') {
            s_bump(s);
            return 1;
        }
        const uint8_t *k;
        size_t kl;
        if (!s_read_string(s, &k, &kl)) return 0;
        s_skip_ws(s);
        if (!s_expect(s, ':')) return 0;
        s_skip_ws(s);
        if (key_eq(k, kl, "timestamp")) {
            if (!s_read_string(s, &p->last_timestamp, &p->last_timestamp_len)) return 0;
        } else if (key_eq(k, kl, "km_from_current")) {
            if (!s_read_f32(s, &p->last_km)) return 0;
            p->has_last_km = 1;
        } else if (!s_skip_value(s)) {
            return 0;
        }
        s_skip_ws(s);
        if (s_peek(s) == ',') s_bump(s);
    }
}

bool extract_json(const uint8_t *body, size_t len, raw_payload_t *out)
{
    memset(out, 0, sizeof(*out));
    scanner_t s = {.buf = body, .len = len, .i = 0};
    if (!s_expect(&s, '{')) return false;
    while (!s_at_end(&s)) {
        s_skip_ws(&s);
        if (s_peek(&s) == '}') {
            s_bump(&s);
            break;
        }
        const uint8_t *key;
        size_t key_len;
        if (!s_read_string(&s, &key, &key_len)) return false;
        s_skip_ws(&s);
        if (!s_expect(&s, ':')) return false;
        s_skip_ws(&s);
        if (key_eq(key, key_len, "transaction")) {
            if (!parse_transaction(&s, out)) return false;
        } else if (key_eq(key, key_len, "customer")) {
            if (!parse_customer(&s, out)) return false;
        } else if (key_eq(key, key_len, "merchant")) {
            if (!parse_merchant(&s, out)) return false;
        } else if (key_eq(key, key_len, "terminal")) {
            if (!parse_terminal(&s, out)) return false;
        } else if (key_eq(key, key_len, "last_transaction")) {
            if (!parse_last_transaction(&s, out)) return false;
        } else if (!s_skip_value(&s)) {
            return false;
        }
        s_skip_ws(&s);
        if (s_peek(&s) == ',') s_bump(&s);
    }
    return true;
}
