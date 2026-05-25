#include "ingest.h"

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

static int digit2(uint8_t a, uint8_t b, uint32_t *out)
{
    if (a >= '0' && a <= '9' && b >= '0' && b <= '9') {
        *out = (uint32_t)(a - '0') * 10u + (uint32_t)(b - '0');
        return 1;
    }
    return 0;
}

static int digit4(uint8_t a, uint8_t b, uint8_t c, uint8_t d, uint32_t *out)
{
    uint32_t ab, cd;
    if (!digit2(a, b, &ab) || !digit2(c, d, &cd)) return 0;
    *out = ab * 100u + cd;
    return 1;
}

static int64_t days_from_civil(int64_t y, uint32_t m, uint32_t d)
{
    if (m <= 2) y--;
    int64_t era = (y >= 0) ? y / 400 : (y - 399) / 400;
    uint64_t yoe = (uint64_t)(y - era * 400);
    uint64_t m_adj = (m > 2) ? (uint64_t)(m - 3) : (uint64_t)(m + 9);
    uint64_t doy = (153 * m_adj + 2) / 5 + (uint64_t)d - 1;
    uint64_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + (int64_t)doe - 719468;
}

static int parse_iso8601_minutes(const uint8_t *ts, size_t len, int64_t *out)
{
    if (len < 19) return 0;
    if (ts[4] != '-' || ts[7] != '-' || ts[10] != 'T' || ts[13] != ':') return 0;
    uint32_t year, month, day, hour, minute;
    if (!digit4(ts[0], ts[1], ts[2], ts[3], &year)) return 0;
    if (!digit2(ts[5], ts[6], &month)) return 0;
    if (!digit2(ts[8], ts[9], &day)) return 0;
    if (!digit2(ts[11], ts[12], &hour)) return 0;
    if (!digit2(ts[14], ts[15], &minute)) return 0;
    if (month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59) return 0;
    int64_t days = days_from_civil((int64_t)year, month, day);
    *out = days * 1440 + (int64_t)hour * 60 + (int64_t)minute;
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
    if (!parse_iso8601_minutes(p->requested_at, p->requested_at_len, &req_minutes)) return false;

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
        if (parse_iso8601_minutes(p->last_timestamp, p->last_timestamp_len, &last)) {
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
