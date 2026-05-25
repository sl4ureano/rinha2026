#ifndef KNN_H
#define KNN_H

#include "index.h"

uint8_t fraud_count(const index_t *idx, const query_vec_t *query);

#endif
