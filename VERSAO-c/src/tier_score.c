#define _GNU_SOURCE
#include "tier_score.h"
#include "decision_tree.h"

#include <string.h>

#define MAX_AMOUNT 10000.f
#define MAX_INSTALLMENTS 12.f
#define AMOUNT_VS_AVG_RATIO 10.f
#define MAX_MINUTES 1440.f
#define MAX_KM 1000.f
#define MAX_TX24H 20.f
#define MAX_MERCHANT_AVG 10000.f
#define RATIO_FRAUD_THRESHOLD 0.06951915f
#define LEGIT_RATIO_CAP 0.50001f

#define MCC5411 0x35343131u
#define MCC5812 0x35383132u
#define MCC5912 0x35393132u
#define MCC5311 0x35333131u
#define MCC7995 0x37393935u
#define MCC7801 0x37383031u
#define MCC7802 0x37383032u

typedef struct {
    uint8_t hour;
    uint8_t weekday_monday0;
    int64_t epoch_seconds;
} parsed_time_t;

typedef struct {
    float safe_avg;
    int known;
    uint32_t mcc;
    int has_requested;
    parsed_time_t requested;
} tier_ctx_t;

static float clamp01(float x)
{
    if (x < 0.f) return 0.f;
    if (x > 1.f) return 1.f;
    return x;
}

static int digit2(uint8_t a, uint8_t b, uint32_t *out)
{
    if (a < '0' || a > '9' || b < '0' || b > '9') return 0;
    *out = (uint32_t)(a - '0') * 10u + (uint32_t)(b - '0');
    return 1;
}

static int digit4(uint8_t a, uint8_t b, uint8_t c, uint8_t d, int64_t *out)
{
    uint32_t hi, lo;
    if (!digit2(a, b, &hi) || !digit2(c, d, &lo)) return 0;
    *out = (int64_t)(hi * 100u + lo);
    return 1;
}

static int64_t days_from_civil(int64_t y, int64_t m, int64_t d)
{
    int64_t year = y;
    int64_t month = m;
    if (month <= 2) year -= 1;
    int64_t era = (year >= 0 ? year : year - 399) / 400;
    int64_t yoe = year - era * 400;
    int64_t month_adj = month > 2 ? month - 3 : month + 9;
    int64_t doy = (153 * month_adj + 2) / 5 + d - 1;
    int64_t doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    return era * 146097 + doe - 719468;
}

static int parse_iso(const uint8_t *ts, size_t len, parsed_time_t *out)
{
    if (len < 19) return 0;
    int64_t year;
    uint32_t month, day, hour, minute, second = 0;
    if (!digit4(ts[0], ts[1], ts[2], ts[3], &year)) return 0;
    if (ts[4] != '-' || ts[7] != '-' || ts[10] != 'T' || ts[13] != ':') return 0;
    if (!digit2(ts[5], ts[6], &month) || !digit2(ts[8], ts[9], &day)) return 0;
    if (!digit2(ts[11], ts[12], &hour) || !digit2(ts[14], ts[15], &minute)) return 0;
    if (len >= 19 && !digit2(ts[17], ts[18], &second)) return 0;
    int64_t days = days_from_civil(year, (int64_t)month, (int64_t)day);
    int64_t wd = (days + 3) % 7;
    if (wd < 0) wd += 7;
    out->hour = (uint8_t)hour;
    out->weekday_monday0 = (uint8_t)wd;
    out->epoch_seconds = days * 86400 + (int64_t)hour * 3600 + (int64_t)minute * 60 + (int64_t)second;
    return 1;
}

static uint32_t mcc4_u32(const uint8_t *mcc, size_t len)
{
    if (len != 4) return 0xffffffffu;
    return ((uint32_t)mcc[0] << 24) | ((uint32_t)mcc[1] << 16) | ((uint32_t)mcc[2] << 8) |
           (uint32_t)mcc[3];
}

static int contains_quoted(const uint8_t *hay, size_t hlen, const uint8_t *needle, size_t nlen)
{
    if (nlen == 0 || hlen < nlen + 2) return 0;
    if (nlen + 2 <= 34) {
        uint8_t pat[34];
        pat[0] = '"';
        memcpy(pat + 1, needle, nlen);
        pat[1 + nlen] = '"';
        return memmem(hay, hlen, pat, nlen + 2) != NULL;
    }
    for (size_t i = 0; i + nlen + 1 < hlen; i++) {
        if (hay[i] == '"' && memcmp(hay + i + 1, needle, nlen) == 0 && hay[i + 1 + nlen] == '"')
            return 1;
    }
    return 0;
}

static int merchant_known(const raw_payload_t *p)
{
    return contains_quoted(p->known_merchants, p->known_merchants_len, p->merchant_id,
                           p->merchant_id_len);
}

static tier_ctx_t tier_ctx_from(const raw_payload_t *p)
{
    tier_ctx_t c;
    c.safe_avg = p->customer_avg_amount > 0.f ? p->customer_avg_amount : 1.f;
    c.known = merchant_known(p);
    c.mcc = mcc4_u32(p->merchant_mcc, p->merchant_mcc_len);
    c.has_requested = parse_iso(p->requested_at, p->requested_at_len, &c.requested);
    return c;
}

static int mcc_is_safe(uint32_t mcc)
{
    return mcc == MCC5411 || mcc == MCC5812 || mcc == MCC5912 || mcc == MCC5311;
}

static int mcc_is_risky(uint32_t mcc)
{
    return mcc == MCC7995 || mcc == MCC7801 || mcc == MCC7802;
}

static float mcc_risk_table_u32(uint32_t mcc)
{
    switch (mcc) {
    case MCC5411: return 0.15f;
    case MCC5812: return 0.30f;
    case MCC5912: return 0.20f;
    case 0x35393434u: return 0.45f; /* 5944 */
    case MCC7801: return 0.80f;
    case MCC7802: return 0.75f;
    case MCC7995: return 0.85f;
    case 0x34353131u: return 0.35f; /* 4511 */
    case MCC5311: return 0.25f;
    case 0x35393939u: return 0.50f; /* 5999 */
    default: return 0.50f;
    }
}

static int obvious_legit(const raw_payload_t *p, const tier_ctx_t *c)
{
    if (p->amount > 500.f) return 0;
    if (p->amount > c->safe_avg * LEGIT_RATIO_CAP) return 0;
    if (p->installments > 3) return 0;
    if (p->tx_count_24h > 5) return 0;
    if (p->km_from_home > 50.f) return 0;
    if (!mcc_is_safe(c->mcc)) return 0;
    return c->known;
}

static int obvious_fraud(const raw_payload_t *p, const tier_ctx_t *c)
{
    return p->amount >= 5000.f && p->installments >= 5 && p->tx_count_24h >= 6 &&
           p->km_from_home >= 150.f && mcc_is_risky(c->mcc) && !c->known;
}

static uint8_t ratio_fraud_count(const raw_payload_t *p)
{
    float safe_avg = p->customer_avg_amount > 0.f ? p->customer_avg_amount : 1.f;
    float norm = clamp01((p->amount / safe_avg) / AMOUNT_VS_AVG_RATIO);
    return norm > RATIO_FRAUD_THRESHOLD ? 5 : 0;
}

static int build_tree_features(const raw_payload_t *p, const tier_ctx_t *ctx, float out[TREE_FEATURE_COUNT])
{
    if (!ctx->has_requested) return 0;
    parsed_time_t requested = ctx->requested;
    float amount_ratio = p->amount / ctx->safe_avg;

    float minutes_since_last = -1.f;
    float km_from_last = -1.f;
    float last_null = 1.f;

    if (p->last_timestamp) {
        parsed_time_t last;
        if (!parse_iso(p->last_timestamp, p->last_timestamp_len, &last)) return 0;
        int64_t delta = requested.epoch_seconds - last.epoch_seconds;
        if (delta < 0) delta = 0;
        minutes_since_last = clamp01((float)delta / 60.f / MAX_MINUTES);
        if (p->has_last_km) km_from_last = clamp01(p->last_km / MAX_KM);
        last_null = 0.f;
    }

    out[0] = clamp01(p->amount / MAX_AMOUNT);
    out[1] = clamp01((float)p->installments / MAX_INSTALLMENTS);
    out[2] = clamp01(amount_ratio / AMOUNT_VS_AVG_RATIO);
    out[3] = (float)requested.hour / 23.f;
    out[4] = (float)requested.weekday_monday0 / 6.f;
    out[5] = minutes_since_last;
    out[6] = km_from_last;
    out[7] = clamp01(p->km_from_home / MAX_KM);
    out[8] = clamp01((float)p->tx_count_24h / MAX_TX24H);
    out[9] = p->is_online ? 1.f : 0.f;
    out[10] = p->card_present ? 1.f : 0.f;
    out[11] = ctx->known ? 0.f : 1.f;
    out[12] = mcc_risk_table_u32(ctx->mcc);
    out[13] = clamp01(p->merchant_avg_amount / MAX_MERCHANT_AVG);
    out[14] = last_null;
    out[15] = p->amount;
    out[16] = p->customer_avg_amount;
    out[17] = amount_ratio;
    out[18] = (float)p->tx_count_24h;
    out[19] = p->km_from_home;
    out[20] = p->merchant_avg_amount;
    return 1;
}

uint8_t tier_fraud_count(const raw_payload_t *p)
{
    tier_ctx_t ctx = tier_ctx_from(p);
    if (obvious_legit(p, &ctx)) return 0;
    if (obvious_fraud(p, &ctx)) return 5;
    if (!ctx.has_requested) return ratio_fraud_count(p);
    float features[TREE_FEATURE_COUNT];
    if (build_tree_features(p, &ctx, features)) return tree_predict(features) ? 5 : 0;
    return ratio_fraud_count(p);
}
