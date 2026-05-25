#ifndef FAST_PATH_H
#define FAST_PATH_H

#include "index.h"
#include "ingest.h"

/* Retorna 0–5 se o perfil for óbvio; -1 → usar k-NN exato. */
int try_fast_fraud_count(const index_t *idx, const raw_payload_t *p);

#endif
