#define _GNU_SOURCE
#include "http.h"
#include "ingest.h"
#include "tier_score.h"

#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <unistd.h>

#define REQ_CAP 65536

static int write_all(int fd, const uint8_t *buf, size_t len)
{
    size_t off = 0;
    while (off < len) {
        ssize_t n = send(fd, buf + off, len - off, MSG_NOSIGNAL);
        if (n <= 0) return -1;
        off += (size_t)n;
    }
    return 0;
}

static inline int write_resp(int fd, const uint8_t *buf, size_t len)
{
    return write_all(fd, buf, len);
}

static int find_double_crlf(const uint8_t *buf, size_t len, size_t *out)
{
    static const char pat[] = "\r\n\r\n";
    const void *p = memmem(buf, len, pat, 4);
    if (!p) return 0;
    *out = (size_t)((const uint8_t *)p - buf) + 4;
    return 1;
}

static int content_length_fast(const uint8_t *hdr, size_t hlen, int *cl)
{
    static const char tag[] = "\r\nContent-Length: ";
    enum { TAG_LEN = sizeof(tag) - 1 };
    const void *p = memmem(hdr, hlen, tag, TAG_LEN);
    if (!p) {
        static const char tag2[] = "\r\ncontent-length: ";
        p = memmem(hdr, hlen, tag2, TAG_LEN);
    }
    if (!p) return 0;
    const uint8_t *start = (const uint8_t *)p + TAG_LEN;
    const uint8_t *end = start;
    const uint8_t *limit = hdr + hlen;
    while (end < limit && *end >= '0' && *end <= '9') end++;
    if (end == start) return 0;
    int n = 0;
    for (const uint8_t *j = start; j < end; j++) n = n * 10 + (*j - '0');
    *cl = n;
    return 1;
}

static uint8_t fraud_count_from_body(const index_t *idx, const uint8_t *body, size_t blen)
{
    raw_payload_t p;
    (void)idx;
    if (!extract_json(body, blen, &p)) return 5;
    return tier_fraud_count(&p);
}

typedef enum { NEED_MORE, CONSUMED_OK, DROP_CONN } handle_outcome_t;

static handle_outcome_t try_handle_one(int fd, const index_t *idx, const uint8_t *buf, size_t len,
                                       size_t *consumed)
{
    size_t header_end;
    if (!find_double_crlf(buf, len, &header_end)) return NEED_MORE;

    if (__builtin_expect(len >= 21 && memcmp(buf, "POST /fraud-score HTTP", 21) == 0, 1)) {
        int cl;
        if (!content_length_fast(buf, header_end, &cl)) {
            write_resp(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN);
            *consumed = header_end;
            return CONSUMED_OK;
        }
        if (len < header_end + (size_t)cl) return NEED_MORE;
        uint8_t fc = fraud_count_from_body(idx, buf + header_end, (size_t)cl);
        write_resp(fd, resp_for_count(fc), resp_len_for_count(fc));
        *consumed = header_end + (size_t)cl;
        return CONSUMED_OK;
    }

    if (len >= 10 && memcmp(buf, "GET /ready", 10) == 0) {
        write_resp(fd, RESP_READY, RESP_READY_LEN);
        *consumed = header_end;
        return CONSUMED_OK;
    }

    write_resp(fd, RESP_NOT_FOUND, RESP_NOT_FOUND_LEN);
    *consumed = header_end;
    return CONSUMED_OK;
}

void serve_connection(int fd, const index_t *idx)
{
    uint8_t stack_buf[4096];
    uint8_t *req_buf = stack_buf;
    size_t req_len = 0, req_cap = sizeof(stack_buf);
    int heap = 0;

    for (;;) {
        if (req_len == req_cap) {
            if (req_cap >= REQ_CAP) {
                write_resp(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN);
                if (heap) free(req_buf);
                close(fd);
                return;
            }
            size_t nc = req_cap * 2;
            if (nc > REQ_CAP) nc = REQ_CAP;
            if (!heap) {
                uint8_t *p = malloc(nc);
                if (!p) { close(fd); return; }
                memcpy(p, req_buf, req_len);
                req_buf = p;
                heap = 1;
            } else {
                uint8_t *p = realloc(req_buf, nc);
                if (!p) { free(req_buf); close(fd); return; }
                req_buf = p;
            }
            req_cap = nc;
        }

        ssize_t n = recv(fd, req_buf + req_len, req_cap - req_len, MSG_NOSIGNAL);
        if (n <= 0) {
            if (heap) free(req_buf);
            close(fd);
            return;
        }
        req_len += (size_t)n;

        for (;;) {
            size_t consumed = 0;
            handle_outcome_t o = try_handle_one(fd, idx, req_buf, req_len, &consumed);
            if (o == NEED_MORE) break;
            if (o == DROP_CONN) {
                write_resp(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN);
                if (heap) free(req_buf);
                close(fd);
                return;
            }
            if (consumed > req_len) {
                if (heap) free(req_buf);
                close(fd);
                return;
            }
            req_len -= consumed;
            if (req_len > 0)
                memmove(req_buf, req_buf + consumed, req_len);
            else
                break;
        }
    }
}
