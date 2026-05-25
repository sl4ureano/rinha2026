#ifndef INGEST_H
#define INGEST_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#include "index.h"

typedef struct {
    float amount;
    uint32_t installments;
    const uint8_t *requested_at;
    size_t requested_at_len;
    float customer_avg_amount;
    uint32_t tx_count_24h;
    const uint8_t *known_merchants;
    size_t known_merchants_len;
    const uint8_t *merchant_id;
    size_t merchant_id_len;
    const uint8_t *merchant_mcc;
    size_t merchant_mcc_len;
    float merchant_avg_amount;
    bool is_online;
    bool card_present;
    float km_from_home;
    const uint8_t *last_timestamp;
    size_t last_timestamp_len;
    float last_km;
    bool has_last_km; /* set when km_from_current is present in JSON */
} raw_payload_t;

typedef struct {
    uint32_t amount_milli;
    uint32_t installments;
    uint8_t hour_of_day;
    uint8_t day_of_week;
    uint32_t minutes_since_last_tx;
    int has_minutes_since;
    uint32_t km_from_last_tx_milli;
    int has_km_from_last;
    uint32_t km_from_home_milli;
    uint32_t customer_avg_amount_milli;
    uint32_t tx_count_24h;
    int is_online;
    int card_present;
    int unknown_merchant;
    int16_t mcc_risk_q;
    uint32_t merchant_avg_amount_milli;
} raw_features_t;

bool extract_json(const uint8_t *body, size_t len, raw_payload_t *out);
bool vectorize_payload(const index_t *idx, const raw_payload_t *p, query_vec_t *out);
void vectorize_features(const raw_features_t *r, query_vec_t *out);

#endif
