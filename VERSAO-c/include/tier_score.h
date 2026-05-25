#ifndef TIER_SCORE_H
#define TIER_SCORE_H

#include "ingest.h"

/* 0 = aprova, 5 = nega */
uint8_t tier_fraud_count(const raw_payload_t *p);

#endif
