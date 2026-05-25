#include "index.h"
#include "ingest.h"

#include <stdint.h>

#define MAX_AMOUNT_UNITS 10000u
#define MAX_INSTALLMENTS 12u
#define MAX_HOUR 23u
#define MAX_DOW 6u
#define MAX_MINUTES 1440u
#define MAX_KM_UNITS 1000u
#define MAX_TX_COUNT_24H 20u
#define MAX_MERCHANT_AVG 10000u

static int16_t clamp_quant_u64(uint64_t v)
{
    if (v >= (uint64_t)IDX_QUANT_SCALE) return (int16_t)IDX_QUANT_SCALE;
    return (int16_t)v;
}

static uint64_t div_round_u64(uint64_t num, uint64_t denom)
{
    return (num + denom / 2) / denom;
}

static int16_t quantize_uint_div(uint32_t value, uint32_t denominator)
{
    return clamp_quant_u64(div_round_u64(
        (uint64_t)value * (uint64_t)IDX_QUANT_SCALE, (uint64_t)denominator));
}

static int16_t quantize_milli_div(uint32_t value_milli, uint32_t denominator_units)
{
    return clamp_quant_u64(div_round_u64(
        (uint64_t)value_milli * (uint64_t)IDX_QUANT_SCALE,
        (uint64_t)denominator_units * 1000ull));
}

static int16_t quantize_amount_ratio(uint32_t amount_milli, uint32_t avg_milli)
{
    if (avg_milli == 0) return (int16_t)IDX_QUANT_SCALE;
    return clamp_quant_u64(div_round_u64((uint64_t)amount_milli * 1000ull, (uint64_t)avg_milli));
}

void vectorize_features(const raw_features_t *r, query_vec_t *out)
{
    (*out)[0] = quantize_milli_div(r->amount_milli, MAX_AMOUNT_UNITS);
    (*out)[1] = quantize_uint_div(r->installments, MAX_INSTALLMENTS);
    (*out)[2] = quantize_amount_ratio(r->amount_milli, r->customer_avg_amount_milli);
    (*out)[3] = quantize_uint_div(r->hour_of_day, MAX_HOUR);
    (*out)[4] = quantize_uint_div(r->day_of_week, MAX_DOW);
    if (r->has_minutes_since)
        (*out)[5] = quantize_uint_div(r->minutes_since_last_tx, MAX_MINUTES);
    else
        (*out)[5] = -(int16_t)IDX_QUANT_SCALE;
    if (r->has_km_from_last)
        (*out)[6] = quantize_milli_div(r->km_from_last_tx_milli, MAX_KM_UNITS);
    else
        (*out)[6] = -(int16_t)IDX_QUANT_SCALE;
    (*out)[7] = quantize_milli_div(r->km_from_home_milli, MAX_KM_UNITS);
    (*out)[8] = quantize_uint_div(r->tx_count_24h, MAX_TX_COUNT_24H);
    (*out)[9] = r->is_online ? (int16_t)IDX_QUANT_SCALE : 0;
    (*out)[10] = r->card_present ? (int16_t)IDX_QUANT_SCALE : 0;
    (*out)[11] = r->unknown_merchant ? (int16_t)IDX_QUANT_SCALE : 0;
    (*out)[12] = r->mcc_risk_q;
    (*out)[13] = quantize_milli_div(r->merchant_avg_amount_milli, MAX_MERCHANT_AVG);
}

uint32_t partition_key(const query_vec_t *v)
{
    uint32_t key = 0;
    if ((*v)[5] >= 0) key |= 1u << 0;
    if ((*v)[9] > 0) key |= 1u << 1;
    if ((*v)[10] > 0) key |= 1u << 2;
    if ((*v)[11] > 0) key |= 1u << 3;
    int16_t mr = (*v)[12];
    if (mr <= 2047) {
    } else if (mr <= 4095) {
        key |= 1u << 4;
    } else if (mr <= 6143) {
        key |= 2u << 4;
    } else {
        key |= 3u << 4;
    }
    if ((*v)[2] > 4096) key |= 1u << 6;
    if ((*v)[8] > 2048) key |= 1u << 7;
    return key;
}

static int64_t lower_bound_dim(int16_t q, int16_t lo, int16_t hi)
{
    int64_t diff;
    if (q < lo)
        diff = (int64_t)lo - (int64_t)q;
    else if (q > hi)
        diff = (int64_t)q - (int64_t)hi;
    else
        return 0;
    return diff * diff;
}

int64_t lower_bound_vec_cutoff(const query_vec_t *q, const query_vec_t *min,
                               const query_vec_t *max, int64_t cutoff)
{
    int64_t acc = 0;
    for (int d = 0; d < IDX_VECTOR_DIM; d++) {
        acc += lower_bound_dim((*q)[d], (*min)[d], (*max)[d]);
        if (acc >= cutoff) return acc;
    }
    return acc;
}
