#include "ingest.h"
#include "time_parse.h"

#include <stdint.h>
#include <string.h>

static uint32_t to_milli(float v)
{
    if (v <= 0.f) return 0;
    double scaled = (double)v * 1000.0 + 0.5;
    if (scaled >= (double)(UINT32_MAX)) return UINT32_MAX;
    return (uint32_t)scaled;
}

static int parse_ascii_u32(const uint8_t *s, size_t len, uint32_t *out)
{
    if (len == 0) return 0;
    uint32_t acc = 0;
    for (size_t i = 0; i < len; i++) {
        if (s[i] < '0' || s[i] > '9') return 0;
        acc = acc * 10u + (uint32_t)(s[i] - '0');
    }
    *out = acc;
    return 1;
}

static int64_t rem_euclid(int64_t a, int64_t b)
{
    int64_t r = a % b;
    if (r < 0) r += b;
    return r;
}

static int64_t div_euclid(int64_t a, int64_t b)
{
    int64_t q = a / b;
    int64_t r = a % b;
    if (r < 0) q--;
    return q;
}

static void hour_dow_from_minutes(int64_t total, uint8_t *hour, uint8_t *dow)
{
    int64_t mins_in_day = rem_euclid(total, 1440);
    *hour = (uint8_t)(mins_in_day / 60);
    int64_t days = div_euclid(total, 1440);
    *dow = (uint8_t)rem_euclid(days + 3, 7);
}

static int contains_quoted(const uint8_t *hay, size_t hlen, const uint8_t *needle, size_t nlen)
{
    for (size_t i = 0; i + nlen + 1 < hlen; i++) {
        if (hay[i] == '"' && i + 1 + nlen < hlen &&
            memcmp(hay + i + 1, needle, nlen) == 0 && hay[i + 1 + nlen] == '"')
            return 1;
    }
    return 0;
}

bool vectorize_payload(const index_t *idx, const raw_payload_t *p, query_vec_t *out)
{
    int64_t req_minutes;
    if (!iso8601_to_minutes_total(p->requested_at, p->requested_at_len, &req_minutes)) return false;

    uint32_t mcc;
    if (!parse_ascii_u32(p->merchant_mcc, p->merchant_mcc_len, &mcc)) return false;

    raw_features_t raw = {0};
    hour_dow_from_minutes(req_minutes, &raw.hour_of_day, &raw.day_of_week);
    raw.amount_milli = to_milli(p->amount);
    raw.installments = p->installments;
    raw.customer_avg_amount_milli = to_milli(p->customer_avg_amount);
    raw.tx_count_24h = p->tx_count_24h;
    raw.is_online = p->is_online ? 1 : 0;
    raw.card_present = p->card_present ? 1 : 0;
    raw.unknown_merchant =
        !contains_quoted(p->known_merchants, p->known_merchants_len, p->merchant_id, p->merchant_id_len);
    raw.mcc_risk_q = index_mcc_risk(idx, mcc);
    raw.km_from_home_milli = to_milli(p->km_from_home);
    raw.merchant_avg_amount_milli = to_milli(p->merchant_avg_amount);

    if (p->last_timestamp_len > 0) {
        int64_t last;
        if (iso8601_to_minutes_total(p->last_timestamp, p->last_timestamp_len, &last)) {
            int64_t m = req_minutes - last;
            if (m < 0) m = 0;
            raw.minutes_since_last_tx = (uint32_t)m;
            raw.has_minutes_since = 1;
        }
    }
    if (p->has_last_km) {
        raw.km_from_last_tx_milli = to_milli(p->last_km);
        raw.has_km_from_last = 1;
    }

    vectorize_features(&raw, out);
    return true;
}
