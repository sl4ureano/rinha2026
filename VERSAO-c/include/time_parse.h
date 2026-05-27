#ifndef TIME_PARSE_H
#define TIME_PARSE_H

#include <stddef.h>
#include <stdint.h>

typedef struct {
    uint8_t hour;
    uint8_t weekday_monday0;
    int64_t epoch_seconds;
} iso8601_utc_t;

/* RFC3339-style UTC instant: YYYY-MM-DDTHH:MM:SS… (seconds at positions 17–18). */
int iso8601_parse_utc(const uint8_t *ts, size_t len, iso8601_utc_t *out);

/* Civil minutes since epoch-0 day in ingest_features (seconds ignored). */
int iso8601_to_minutes_total(const uint8_t *ts, size_t len, int64_t *out);

#endif
