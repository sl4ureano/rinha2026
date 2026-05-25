#include "http.h"

const uint8_t RESP_READY[] =
    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: keep-alive\r\n\r\nOK";
static const uint8_t RESP_APPROVED_S0[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 33\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0}";
static const uint8_t RESP_APPROVED_S2[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0.2}";
static const uint8_t RESP_APPROVED_S4[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 35\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":true,\"fraud_score\":0.4}";
static const uint8_t RESP_DENIED_S6[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 36\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":0.6}";
static const uint8_t RESP_DENIED_S8[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 36\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":0.8}";
const uint8_t RESP_DENIED_S10[] =
    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 34\r\n"
    "Connection: keep-alive\r\n\r\n{\"approved\":false,\"fraud_score\":1}";
const uint8_t RESP_NOT_FOUND[] =
    "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n";

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
