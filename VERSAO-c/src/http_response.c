#include "http.h"

/* Minimal headers: HTTP/1.1 defaults to keep-alive; k6 parses body via JSON.parse, no Content-Type needed. */
const uint8_t RESP_READY[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
static const uint8_t RESP_NOT_READY[] =
    "HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n";
static const uint8_t RESP_APPROVED_S0[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 33\r\n\r\n{\"approved\":true,\"fraud_score\":0}";
static const uint8_t RESP_APPROVED_S2[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 35\r\n\r\n{\"approved\":true,\"fraud_score\":0.2}";
static const uint8_t RESP_APPROVED_S4[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 35\r\n\r\n{\"approved\":true,\"fraud_score\":0.4}";
static const uint8_t RESP_DENIED_S6[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 36\r\n\r\n{\"approved\":false,\"fraud_score\":0.6}";
static const uint8_t RESP_DENIED_S8[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 36\r\n\r\n{\"approved\":false,\"fraud_score\":0.8}";
const uint8_t RESP_DENIED_S10[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 34\r\n\r\n{\"approved\":false,\"fraud_score\":1}";
const uint8_t RESP_NOT_FOUND[] =
    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n";

const uint8_t *resp_ready(int ready)
{
    return ready ? RESP_READY : RESP_NOT_READY;
}

size_t resp_ready_len(int ready)
{
    return ready ? (sizeof(RESP_READY) - 1) : (sizeof(RESP_NOT_READY) - 1);
}

static const size_t RESP_LEN_BY_COUNT[6] = {
    sizeof(RESP_APPROVED_S0) - 1,
    sizeof(RESP_APPROVED_S2) - 1,
    sizeof(RESP_APPROVED_S4) - 1,
    sizeof(RESP_DENIED_S6) - 1,
    sizeof(RESP_DENIED_S8) - 1,
    sizeof(RESP_DENIED_S10) - 1,
};

const uint8_t *resp_for_count(uint8_t count)
{
    switch (count) {
    case 0: return RESP_APPROVED_S0;
    case 1: return RESP_APPROVED_S2;
    case 2: return RESP_APPROVED_S4;
    case 3: return RESP_DENIED_S6;
    case 4: return RESP_DENIED_S8;
    default: return RESP_DENIED_S10;
    }
}

size_t resp_len_for_count(uint8_t count)
{
    if (count > 5) count = 5;
    return RESP_LEN_BY_COUNT[count];
}
