#include "time_parse.h"

static int digit2(uint8_t a, uint8_t b, uint32_t *out)
{
    if (a < '0' || a > '9' || b < '0' || b > '9') return 0;
    *out = (uint32_t)(a - '0') * 10u + (uint32_t)(b - '0');
    return 1;
}

static int digit4(uint8_t a, uint8_t b, uint8_t c, uint8_t d, uint32_t *out)
{
    uint32_t hi, lo;
    if (!digit2(a, b, &hi) || !digit2(c, d, &lo)) return 0;
    *out = hi * 100u + lo;
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

static int separators_rinha(const uint8_t *ts)
{
    return ts[4] == '-' && ts[7] == '-' && ts[10] == 'T' && ts[13] == ':' && ts[16] == ':';
}

/* Hot path: "YYYY-MM-DDTHH:MM:SSZ" (20 bytes) — matches official payloads. */
static int parse_zulu_20(const uint8_t *ts, uint32_t *year, uint32_t *month, uint32_t *day,
                         uint32_t *hour, uint32_t *minute, uint32_t *second)
{
    if (ts[19] != 'Z') return 0;
    if (!separators_rinha(ts)) return 0;
    if (!digit4(ts[0], ts[1], ts[2], ts[3], year)) return 0;
    if (!digit2(ts[5], ts[6], month) || !digit2(ts[8], ts[9], day)) return 0;
    if (!digit2(ts[11], ts[12], hour) || !digit2(ts[14], ts[15], minute)) return 0;
    if (!digit2(ts[17], ts[18], second)) return 0;
    return 1;
}

static int parse_datetime_prefix(const uint8_t *ts, size_t len, uint32_t *year, uint32_t *month,
                                 uint32_t *day, uint32_t *hour, uint32_t *minute, uint32_t *second,
                                 int need_seconds)
{
    if (len >= 20 && ts[19] == 'Z') {
        if (parse_zulu_20(ts, year, month, day, hour, minute, second)) return 1;
    }

    if (len < 19) return 0;
    if (ts[4] != '-' || ts[7] != '-' || ts[10] != 'T' || ts[13] != ':') return 0;
    if (!digit4(ts[0], ts[1], ts[2], ts[3], year)) return 0;
    if (!digit2(ts[5], ts[6], month) || !digit2(ts[8], ts[9], day)) return 0;
    if (!digit2(ts[11], ts[12], hour) || !digit2(ts[14], ts[15], minute)) return 0;
    if (need_seconds) {
        if (ts[16] != ':') return 0;
        if (!digit2(ts[17], ts[18], second)) return 0;
    } else {
        *second = 0;
    }
    return 1;
}

int iso8601_parse_utc(const uint8_t *ts, size_t len, iso8601_utc_t *out)
{
    uint32_t year, month, day, hour, minute, second = 0;
    if (!parse_datetime_prefix(ts, len, &year, &month, &day, &hour, &minute, &second, 1)) return 0;

    int64_t days = days_from_civil((int64_t)year, (int64_t)month, (int64_t)day);
    int64_t wd = (days + 3) % 7;
    if (wd < 0) wd += 7;
    out->hour = (uint8_t)hour;
    out->weekday_monday0 = (uint8_t)wd;
    out->epoch_seconds =
        days * 86400 + (int64_t)hour * 3600 + (int64_t)minute * 60 + (int64_t)second;
    return 1;
}

int iso8601_to_minutes_total(const uint8_t *ts, size_t len, int64_t *out)
{
    uint32_t year, month, day, hour, minute, second;
    if (len >= 20 && ts[19] == 'Z') {
        if (!parse_zulu_20(ts, &year, &month, &day, &hour, &minute, &second)) return 0;
    } else {
        if (len < 19) return 0;
        if (ts[4] != '-' || ts[7] != '-' || ts[10] != 'T' || ts[13] != ':') return 0;
        if (!digit4(ts[0], ts[1], ts[2], ts[3], &year)) return 0;
        if (!digit2(ts[5], ts[6], &month) || !digit2(ts[8], ts[9], &day)) return 0;
        if (!digit2(ts[11], ts[12], &hour) || !digit2(ts[14], ts[15], &minute)) return 0;
        second = 0;
    }
    if (month < 1 || month > 12 || day < 1 || day > 31 || hour > 23 || minute > 59) return 0;
    int64_t days = days_from_civil((int64_t)year, (int64_t)month, (int64_t)day);
    *out = days * 1440 + (int64_t)hour * 60 + (int64_t)minute;
    return 1;
}
