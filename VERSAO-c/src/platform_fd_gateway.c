#define _GNU_SOURCE
#include "http.h"
#include "index.h"
#include "ingest.h"
#include "tier_score.h"

#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/mman.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

#define MAX_FDS      65536
#define MAX_EVENTS   512
#define BUF_CAP      8192
#define EPOLL_TIMEOUT_MS 1

#define CLIENT_EVENTS      (EPOLLIN | EPOLLRDHUP | EPOLLET)
#define CLIENT_WRITE_EVENTS (EPOLLIN | EPOLLOUT | EPOLLRDHUP | EPOLLET)
#define CTRL_EVENTS        (EPOLLIN | EPOLLRDHUP | EPOLLET)

extern int scm_recv_fd_nonblock(int control_fd);

static const index_t *g_idx;
static int g_epfd = -1;

typedef struct {
    uint8_t buf[BUF_CAP];
    size_t buf_len;
    const uint8_t *send_ptr;
    size_t send_len;
    size_t send_off;
} conn_t;

static conn_t *g_conns[MAX_FDS];
static uint8_t g_is_ctrl[MAX_FDS];

static inline void conn_reset(conn_t *c)
{
    c->buf_len = 0;
    c->send_ptr = NULL;
    c->send_len = 0;
    c->send_off = 0;
}

static conn_t *get_conn(int fd)
{
    if (fd < 0 || fd >= MAX_FDS) return NULL;
    if (!g_conns[fd]) {
        g_conns[fd] = (conn_t *)calloc(1, sizeof(conn_t));
        if (!g_conns[fd]) return NULL;
    }
    conn_reset(g_conns[fd]);
    return g_conns[fd];
}

static void drop_conn(int fd)
{
    epoll_ctl(g_epfd, EPOLL_CTL_DEL, fd, NULL);
    close(fd);
}

/* --- HTTP processing (inline, no thread) --- */

static inline const uint8_t *find_double_crlf(const uint8_t *buf, size_t len, size_t *out)
{
    if (len < 4) return NULL;
    for (size_t i = 0; i + 4 <= len; i++) {
        if (buf[i] == '\r' && buf[i+1] == '\n' && buf[i+2] == '\r' && buf[i+3] == '\n') {
            *out = i + 4;
            return buf;
        }
    }
    return NULL;
}

static inline int parse_content_length(const uint8_t *hdr, size_t hlen, int *cl)
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

static inline const uint8_t *fraud_response(const uint8_t *body, size_t blen, size_t *resp_len)
{
    raw_payload_t p;
    uint8_t fc;
    if (!extract_json(body, blen, &p)) {
        fc = 5;
    } else {
        fc = tier_fraud_count(&p);
    }
    *resp_len = resp_len_for_count(fc);
    return resp_for_count(fc);
}

/* Returns 1 if connection fully handled, 0 if need more data */
static int try_process_requests(int fd, conn_t *c)
{
    for (;;) {
        size_t header_end;
        if (!find_double_crlf(c->buf, c->buf_len, &header_end)) return 0;

        /* POST /fraud-score */
        if (c->buf_len >= 5 && memcmp(c->buf, "POST ", 5) == 0) {
            int cl;
            if (!parse_content_length(c->buf, header_end, &cl)) {
                send(fd, RESP_DENIED_S10, RESP_DENIED_S10_LEN, MSG_NOSIGNAL);
                return 1;
            }
            if (c->buf_len < header_end + (size_t)cl) return 0; /* need more body */

            size_t resp_len;
            const uint8_t *resp = fraud_response(c->buf + header_end, (size_t)cl, &resp_len);
            size_t consumed = header_end + (size_t)cl;

            ssize_t sent = send(fd, resp, resp_len, MSG_NOSIGNAL);
            if ((size_t)sent == resp_len) {
                size_t leftover = c->buf_len - consumed;
                if (leftover > 0) {
                    memmove(c->buf, c->buf + consumed, leftover);
                    c->buf_len = leftover;
                    continue; /* pipeline */
                }
                c->buf_len = 0;
                /* Keep connection alive, ET will re-fire on next data */
                return 1;
            } else if (sent < 0) {
                if (errno == EAGAIN || errno == EWOULDBLOCK) {
                    c->send_ptr = resp;
                    c->send_len = resp_len;
                    c->send_off = 0;
                    size_t leftover = c->buf_len - consumed;
                    if (leftover > 0) memmove(c->buf, c->buf + consumed, leftover);
                    c->buf_len = leftover;
                    struct epoll_event ev = { .events = CLIENT_WRITE_EVENTS, .data.fd = fd };
                    epoll_ctl(g_epfd, EPOLL_CTL_MOD, fd, &ev);
                    return 1;
                }
                drop_conn(fd);
                return 1;
            } else {
                /* partial send */
                c->send_ptr = resp;
                c->send_len = resp_len;
                c->send_off = (size_t)sent;
                size_t leftover = c->buf_len - consumed;
                if (leftover > 0) memmove(c->buf, c->buf + consumed, leftover);
                c->buf_len = leftover;
                struct epoll_event ev = { .events = CLIENT_WRITE_EVENTS, .data.fd = fd };
                epoll_ctl(g_epfd, EPOLL_CTL_MOD, fd, &ev);
                return 1;
            }
        }

        /* GET /ready */
        if (c->buf_len >= 3 && memcmp(c->buf, "GET", 3) == 0) {
            const uint8_t *r = resp_ready(g_idx && g_idx->ready);
            size_t rlen = resp_ready_len(g_idx && g_idx->ready);
            send(fd, r, rlen, MSG_NOSIGNAL);
            size_t leftover = c->buf_len - header_end;
            if (leftover > 0) {
                memmove(c->buf, c->buf + header_end, leftover);
                c->buf_len = leftover;
                continue;
            }
            c->buf_len = 0;
            return 1;
        }

        send(fd, RESP_NOT_FOUND, RESP_NOT_FOUND_LEN, MSG_NOSIGNAL);
        drop_conn(fd);
        return 1;
    }
}

/* --- Epoll event handlers --- */

static void handle_client_read(int fd)
{
    if (fd < 0 || fd >= MAX_FDS || !g_conns[fd]) { drop_conn(fd); return; }
    conn_t *c = g_conns[fd];

    /* ET mode: drain socket fully */
    for (;;) {
        size_t room = BUF_CAP - c->buf_len;
        if (room == 0) break;
        ssize_t n = recv(fd, c->buf + c->buf_len, room, 0);
        if (n > 0) {
            c->buf_len += (size_t)n;
        } else if (n == 0) {
            drop_conn(fd);
            return;
        } else {
            if (errno == EAGAIN || errno == EWOULDBLOCK) break;
            drop_conn(fd);
            return;
        }
    }

    try_process_requests(fd, c);
}

static void handle_client_write(int fd)
{
    if (fd < 0 || fd >= MAX_FDS || !g_conns[fd]) { drop_conn(fd); return; }
    conn_t *c = g_conns[fd];

    if (!c->send_ptr) {
        struct epoll_event ev = { .events = CLIENT_EVENTS, .data.fd = fd };
        epoll_ctl(g_epfd, EPOLL_CTL_MOD, fd, &ev);
        return;
    }

    /* ET mode: drain send buffer fully */
    for (;;) {
        size_t remaining = c->send_len - c->send_off;
        if (remaining == 0) break;
        ssize_t n = send(fd, c->send_ptr + c->send_off, remaining, MSG_NOSIGNAL);
        if (n > 0) {
            c->send_off += (size_t)n;
        } else {
            if (errno == EAGAIN || errno == EWOULDBLOCK) return;
            drop_conn(fd);
            return;
        }
    }

    /* Done sending, switch back to read-only ET */
    c->send_ptr = NULL;
    c->send_off = 0;
    c->send_len = 0;
    struct epoll_event ev = { .events = CLIENT_EVENTS, .data.fd = fd };
    epoll_ctl(g_epfd, EPOLL_CTL_MOD, fd, &ev);
}

static void accept_ctrl_conn(int ctrl_listen_fd)
{
    for (;;) {
        int cfd = accept4(ctrl_listen_fd, NULL, NULL, SOCK_NONBLOCK | SOCK_CLOEXEC);
        if (cfd < 0) return;
        if (cfd >= MAX_FDS) { close(cfd); continue; }
        g_is_ctrl[cfd] = 1;
        struct epoll_event ev = { .events = CTRL_EVENTS, .data.fd = cfd };
        epoll_ctl(g_epfd, EPOLL_CTL_ADD, cfd, &ev);
    }
}

static void accept_from_lb(int ctrl_fd)
{
    for (int i = 0; i < 64; i++) {
        int client_fd = scm_recv_fd_nonblock(ctrl_fd);
        if (client_fd < 0) return;
        if (client_fd >= MAX_FDS) { close(client_fd); continue; }

        /* Set non-blocking */
        int flags = fcntl(client_fd, F_GETFL, 0);
        if (flags >= 0) fcntl(client_fd, F_SETFL, flags | O_NONBLOCK);

        /* TCP_NODELAY + TCP_QUICKACK */
        int one = 1;
        setsockopt(client_fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
        setsockopt(client_fd, IPPROTO_TCP, TCP_QUICKACK, &one, sizeof(one));

        conn_t *c = get_conn(client_fd);
        if (!c) { close(client_fd); continue; }

        /* Greedy read: try to read and process immediately */
        ssize_t n = recv(client_fd, c->buf, BUF_CAP, 0);
        if (n > 0) {
            c->buf_len = (size_t)n;
            if (try_process_requests(client_fd, c)) {
                /* If fully handled but connection not closed, register for next */
                if (c->send_ptr == NULL && c->buf_len == 0) {
                    struct epoll_event ev = { .events = CLIENT_EVENTS, .data.fd = client_fd };
                    epoll_ctl(g_epfd, EPOLL_CTL_ADD, client_fd, &ev);
                }
                continue;
            }
        } else if (n == 0) {
            close(client_fd);
            continue;
        }
        /* EAGAIN or need more data: register with epoll */

        struct epoll_event ev = { .events = CLIENT_EVENTS, .data.fd = client_fd };
        epoll_ctl(g_epfd, EPOLL_CTL_ADD, client_fd, &ev);
    }
}

/* --- Main event loop --- */

static void event_loop(int ctrl_listen_fd)
{
    struct epoll_event events[MAX_EVENTS];

    for (;;) {
        int nfds = epoll_wait(g_epfd, events, MAX_EVENTS, EPOLL_TIMEOUT_MS);
        if (nfds < 0) {
            if (errno == EINTR) continue;
            continue;
        }

        for (int i = 0; i < nfds; i++) {
            int fd = events[i].data.fd;
            uint32_t revents = events[i].events;

            /* Ctrl listener */
            if (fd == ctrl_listen_fd) {
                accept_ctrl_conn(ctrl_listen_fd);
                continue;
            }

            /* Ctrl (LB) connections */
            if (fd >= 0 && fd < MAX_FDS && g_is_ctrl[fd]) {
                if (revents & (EPOLLHUP | EPOLLERR | EPOLLRDHUP)) {
                    epoll_ctl(g_epfd, EPOLL_CTL_DEL, fd, NULL);
                    g_is_ctrl[fd] = 0;
                    close(fd);
                } else if (revents & EPOLLIN) {
                    accept_from_lb(fd);
                }
                continue;
            }

            /* Client connections */
            if (revents & (EPOLLHUP | EPOLLERR)) {
                drop_conn(fd);
                continue;
            }
            if (revents & EPOLLIN) {
                handle_client_read(fd);
            }
            if (revents & EPOLLOUT) {
                handle_client_write(fd);
            }
            if (revents & EPOLLRDHUP) {
                drop_conn(fd);
            }
        }
    }
}

int fd_gateway_run(const index_t *idx, const char *sock_path)
{
    g_idx = idx;

    /* mlockall for memory pinning */
    mlockall(MCL_CURRENT | MCL_FUTURE);

    signal(SIGPIPE, SIG_IGN);

    /* Create parent directory */
    char parent[512];
    strncpy(parent, sock_path, sizeof(parent) - 1);
    parent[sizeof(parent) - 1] = '\0';
    char *slash = strrchr(parent, '/');
    if (slash && slash != parent) {
        *slash = '\0';
        mkdir(parent, 0755);
    }
    unlink(sock_path);

    /* Create unix listener (non-blocking) */
    int listener = socket(AF_UNIX, SOCK_STREAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
    if (listener < 0) return -1;
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    size_t plen = strlen(sock_path);
    if (plen >= sizeof(addr.sun_path)) { close(listener); return -1; }
    memcpy(addr.sun_path, sock_path, plen + 1);
    if (bind(listener, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(listener);
        return -1;
    }
    chmod(sock_path, 0666);
    if (listen(listener, 1024) < 0) {
        close(listener);
        return -1;
    }

    fprintf(stderr, "listening uds fd-pass %s (epoll ET)\n", sock_path);

    /* Create epoll */
    g_epfd = epoll_create1(EPOLL_CLOEXEC);
    if (g_epfd < 0) return -1;

    /* Register ctrl listener */
    struct epoll_event ev = { .events = EPOLLIN | EPOLLET, .data.fd = listener };
    epoll_ctl(g_epfd, EPOLL_CTL_ADD, listener, &ev);

    event_loop(listener);
    return 0; /* never reached */
}