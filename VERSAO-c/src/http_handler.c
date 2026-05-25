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
        ssize_t n = write(fd, buf + off, len - off);
        if (n <= 0) return -1;
        off += (size_t)n;
    }
    return 0;
}

static inline int write_resp(int fd, const uint8_t *buf, size_t len)
{
    ssize_t n = write(fd, buf, len);
    if (n == (ssize_t)len) return 0;
    if (n < 0) return -1;
    return write_all(fd, buf + (size_t)n, len - (size_t)n);
}

static int find_double_crlf(const uint8_t *buf, size_t len, size_t *out)
{
    if (len < 4) return 0;
    for (size_t i = 0; i + 4 <= len; i++) {
        uint32_t w = (uint32_t)buf[i] | ((uint32_t)buf[i + 1] << 8) |
                     ((uint32_t)buf[i + 2] << 16) | ((uint32_t)buf[i + 3] << 24);
        if (w == 0x0a0d0a0du) {
            *out = i + 4;
            return 1;
        }
    }
    return 0;
}

static int memchr_crlf(const uint8_t *buf, size_t len, size_t *out)
{
    for (size_t i = 0; i + 1 < len; i++) {
        if (buf[i] == '\r' && buf[i + 1] == '\n') {
            *out = i;
            return 1;
        }
    }
    return 0;
}

static int content_length_fast(const uint8_t *hdr, size_t hlen, int *cl)
{
    static const char tag[] = "\r\nContent-Length: ";
    enum { TAG_LEN = sizeof(tag) - 1 };

    if (hlen < TAG_LEN + 1) return 0;
    for (size_t i = 0; i + TAG_LEN <= hlen; i++) {
        if (memcmp(hdr + i, tag, TAG_LEN) != 0) continue;
        size_t start = i + TAG_LEN;
        size_t end = start;
        while (end < hlen && hdr[end] >= '0' && hdr[end] <= '9') end++;
        if (end == start) return 0;
        int n = 0;
        for (size_t j = start; j < end; j++) n = n * 10 + (hdr[j] - '0');
        *cl = n;
        return 1;
    }
    static const char tag2[] = "\r\ncontent-length: ";
    enum { TAG2_LEN = sizeof(tag2) - 1 };
    for (size_t i = 0; i + TAG2_LEN <= hlen; i++) {
        if (memcmp(hdr + i, tag2, TAG2_LEN) != 0) continue;
        size_t start = i + TAG2_LEN;
        size_t end = start;
        while (end < hlen && hdr[end] >= '0' && hdr[end] <= '9') end++;
        if (end == start) return 0;
        int n = 0;
        for (size_t j = start; j < end; j++) n = n * 10 + (hdr[j] - '0');
        *cl = n;
        return 1;
    }
    return 0;
}

static int parse_request_line(const uint8_t *line, size_t len, size_t *m_end, size_t *p_end)
{
    size_t sp1 = (size_t)-1;
    for (size_t i = 0; i < len; i++) {
        if (line[i] == ' ') {
            sp1 = i;
            break;
        }
    }
    if (sp1 == (size_t)-1) return 0;
    const uint8_t *rest = line + sp1 + 1;
    size_t rlen = len - sp1 - 1;
    size_t sp2 = (size_t)-1;
    for (size_t i = 0; i < rlen; i++) {
        if (rest[i] == ' ') {
            sp2 = i;
            break;
        }
    }
    if (sp2 == (size_t)-1) return 0;
    *m_end = sp1;
    *p_end = sp2;
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

    if (len >= 21 && memcmp(buf, "POST /fraud-score HTTP", 21) == 0) {
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

    size_t req_line_end;
    if (!memchr_crlf(buf, len, &req_line_end)) return NEED_MORE;
    size_t m_end, p_end;
    if (!parse_request_line(buf, req_line_end, &m_end, &p_end)) {
        write_resp(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN);
        *consumed = header_end;
        return CONSUMED_OK;
    }

    int is_get = (m_end == 3 && memcmp(buf, "GET", 3) == 0);
    int is_post = (m_end == 4 && memcmp(buf, "POST", 4) == 0);
    const uint8_t *path = buf + m_end + 1;
    size_t plen = p_end;

    if (is_get && plen == 6 && memcmp(path, "/ready", 6) == 0) {
        write_resp(fd, RESP_READY, RESP_READY_LEN);
        *consumed = header_end;
        return CONSUMED_OK;
    }

    if (is_post && plen == 12 && memcmp(path, "/fraud-score", 12) == 0) {
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

    write_resp(fd, RESP_NOT_FOUND, RESP_NOT_FOUND_LEN);
    *consumed = header_end;
    return CONSUMED_OK;
}

void serve_connection(int fd, const index_t *idx)
{
    int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));

    uint8_t *req_buf = NULL;
    size_t req_len = 0, req_cap = 4096;
    req_buf = malloc(req_cap);
    if (!req_buf) {
        close(fd);
        return;
    }
    uint8_t read_buf[8192];

    for (;;) {
        ssize_t n = read(fd, read_buf, sizeof(read_buf));
        if (n <= 0) {
            free(req_buf);
            close(fd);
            return;
        }
        if (req_len + (size_t)n > req_cap) {
            if (req_len + (size_t)n > REQ_CAP) {
                free(req_buf);
                close(fd);
                return;
            }
            size_t nc = req_cap;
            while (nc < req_len + (size_t)n) {
                nc *= 2;
                if (nc > REQ_CAP) nc = REQ_CAP;
            }
            uint8_t *p = realloc(req_buf, nc);
            if (!p) {
                free(req_buf);
                close(fd);
                return;
            }
            req_buf = p;
            req_cap = nc;
        }
        memcpy(req_buf + req_len, read_buf, (size_t)n);
        req_len += (size_t)n;

        for (;;) {
            size_t consumed = 0;
            handle_outcome_t o = try_handle_one(fd, idx, req_buf, req_len, &consumed);
            if (o == NEED_MORE) break;
            if (o == DROP_CONN) {
                write_resp(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN);
                free(req_buf);
                close(fd);
                return;
            }
            if (consumed > req_len) {
                free(req_buf);
                close(fd);
                return;
            }
            memmove(req_buf, req_buf + consumed, req_len - consumed);
            req_len -= consumed;
            if (req_len == 0) break;
        }
    }
}
