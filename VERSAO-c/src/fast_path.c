#include "fast_path.h"

#include <string.h>

#define MAX_AMOUNT_LEGIT 500.0f
#define MAX_RATIO_LEGIT 0.5f
#define MAX_INSTALLMENTS_LEGIT 3u
#define MAX_TX24H_LEGIT 5u
#define MAX_KM_HOME_LEGIT 50.0f

#define MIN_AMOUNT_FRAUD 5000.0f
#define MIN_INSTALLMENTS_FRAUD 5u
#define MIN_TX24H_FRAUD 6u
#define MIN_KM_HOME_FRAUD 150.0f

static int contains_quoted(const uint8_t *hay, size_t hlen, const uint8_t *needle, size_t nlen)
{
    if (nlen == 0 || hlen < nlen + 2) return 0;
    for (size_t i = 0; i + nlen + 1 < hlen; i++) {
        if (hay[i] == '"' && memcmp(hay + i + 1, needle, nlen) == 0 &&
            hay[i + 1 + nlen] == '"') {
            return 1;
        }
    }
    return 0;
}

static int merchant_known(const raw_payload_t *p)
{
    return contains_quoted(p->known_merchants, p->known_merchants_len, p->merchant_id,
                           p->merchant_id_len);
}

static int is_safe_mcc(const uint8_t *mcc, size_t len)
{
    return (len == 4 && ((memcmp(mcc, "5411", 4) == 0) || (memcmp(mcc, "5812", 4) == 0) ||
                         (memcmp(mcc, "5912", 4) == 0) || (memcmp(mcc, "5311", 4) == 0)));
}

static uint32_t parse_mcc(const uint8_t *mcc, size_t len)
{
    uint32_t acc = 0;
    for (size_t i = 0; i < len; i++) {
        if (mcc[i] < '0' || mcc[i] > '9') return 0;
        acc = acc * 10u + (uint32_t)(mcc[i] - '0');
    }
    return acc;
}

static int is_risky_mcc(const index_t *idx, const uint8_t *mcc, size_t len)
{
    uint32_t mcc_num = parse_mcc(mcc, len);
    int risk = index_mcc_risk(idx, mcc_num);
    if (risk >= 7500) return 1;
    if (len == 4 &&
        (memcmp(mcc, "7995", 4) == 0 || memcmp(mcc, "7801", 4) == 0 || memcmp(mcc, "7802", 4) == 0))
        return 1;
    return 0;
}

static int obvious_legit(const raw_payload_t *p)
{
    if (p->amount > MAX_AMOUNT_LEGIT) return 0;
    float safe_avg = p->customer_avg_amount > 0.0f ? p->customer_avg_amount : 1.0f;
    if (p->amount / safe_avg > MAX_RATIO_LEGIT) return 0;
    if (p->installments > MAX_INSTALLMENTS_LEGIT) return 0;
    if (p->tx_count_24h > MAX_TX24H_LEGIT) return 0;
    if (!merchant_known(p)) return 0;
    if (p->km_from_home > MAX_KM_HOME_LEGIT) return 0;
    if (!is_safe_mcc(p->merchant_mcc, p->merchant_mcc_len)) return 0;
    return 1;
}

static int obvious_fraud(const index_t *idx, const raw_payload_t *p)
{
    if (p->amount < MIN_AMOUNT_FRAUD) return 0;
    if (p->installments < MIN_INSTALLMENTS_FRAUD) return 0;
    if (p->tx_count_24h < MIN_TX24H_FRAUD) return 0;
    if (merchant_known(p)) return 0;
    if (p->km_from_home < MIN_KM_HOME_FRAUD) return 0;
    if (!is_risky_mcc(idx, p->merchant_mcc, p->merchant_mcc_len)) return 0;
    return 1;
}

int try_fast_fraud_count(const index_t *idx, const raw_payload_t *p)
{
    if (obvious_legit(p)) return 0;
    if (obvious_fraud(idx, p)) return 5;
    return -1;
}
