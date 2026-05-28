#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <fcntl.h>
#include <time.h>
#include <unistd.h>

#define MAX_UPSTREAMS 16
#define BACKLOG 65535

extern void scm_set_tcp_nodelay(int fd);
extern void scm_write_502(int fd);

typedef struct {
    char path[256];
    int fd;
} upstream_t;

static upstream_t upstreams[MAX_UPSTREAMS];
static int upstream_count = 0;
static uint32_t rr_next = 0;

static void sleep_ms(long ms)
{
    struct timespec ts;
    ts.tv_sec = ms / 1000;
    ts.tv_nsec = (ms % 1000) * 1000000L;
    while (nanosleep(&ts, &ts) < 0 && errno == EINTR) {}
}

static int connect_once(const char *path)
{
    int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    struct sockaddr_un addr;
    memset(&addr, 0, sizeof(addr));
    addr.sun_family = AF_UNIX;
    strncpy(addr.sun_path, path, sizeof(addr.sun_path) - 1);
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) != 0) {
        close(fd);
        return -1;
    }
    return fd;
}

static int connect_wait(const char *path)
{
    for (;;) {
        int fd = connect_once(path);
        if (fd >= 0) return fd;
        sleep_ms(5);
    }
}

static int reconnect_one(int idx)
{
    if (upstreams[idx].fd >= 0) close(upstreams[idx].fd);
    upstreams[idx].fd = -1;
    for (int tries = 0; tries < 20; tries++) {
        int fd = connect_once(upstreams[idx].path);
        if (fd >= 0) {
            upstreams[idx].fd = fd;
            return 0;
        }
        sleep_ms(2);
    }
    return -1;
}

static char s_byte;
static struct iovec s_iov = {.iov_base = &s_byte, .iov_len = 1};
static union {
    char buf[CMSG_SPACE(sizeof(int))];
    struct cmsghdr align;
} s_control;
static struct msghdr s_msg;
static int s_fd_init_done;

static void init_send_fd(void)
{
    s_msg.msg_iov = &s_iov;
    s_msg.msg_iovlen = 1;
    s_msg.msg_control = s_control.buf;
    s_msg.msg_controllen = sizeof(s_control.buf);
    struct cmsghdr *cmsg = CMSG_FIRSTHDR(&s_msg);
    cmsg->cmsg_level = SOL_SOCKET;
    cmsg->cmsg_type = SCM_RIGHTS;
    cmsg->cmsg_len = CMSG_LEN(sizeof(int));
    s_fd_init_done = 1;
}

static int send_fd_once(int ctrl_fd, int client_fd)
{
    if (__builtin_expect(!s_fd_init_done, 0)) init_send_fd();
    memcpy(CMSG_DATA(CMSG_FIRSTHDR(&s_msg)), &client_fd, sizeof(client_fd));
    for (;;) {
        ssize_t n = sendmsg(ctrl_fd, &s_msg, MSG_NOSIGNAL);
        if (n == 1) return 0;
        if (n < 0 && errno == EINTR) continue;
        return -1;
    }
}

static int handoff(int idx, int client_fd)
{
    if (upstreams[idx].fd < 0 && reconnect_one(idx) != 0) return -1;
    if (send_fd_once(upstreams[idx].fd, client_fd) == 0) return 0;
    if (reconnect_one(idx) != 0) return -1;
    return send_fd_once(upstreams[idx].fd, client_fd);
}

static void add_upstream(const char *path, size_t len)
{
    while (len > 0 && (*path == ' ' || *path == '\t')) { path++; len--; }
    while (len > 0 && (path[len - 1] == ' ' || path[len - 1] == '\t' || path[len - 1] == '\n')) len--;
    if (len == 0 || upstream_count >= MAX_UPSTREAMS) return;
    upstream_t *u = &upstreams[upstream_count++];
    memset(u, 0, sizeof(*u));
    if (len >= sizeof(u->path)) len = sizeof(u->path) - 1;
    memcpy(u->path, path, len);
    u->path[len] = '\0';
    u->fd = -1;
}

static void parse_upstreams(const char *csv)
{
    const char *start = csv;
    for (const char *p = csv;; p++) {
        if (*p == ',' || *p == '\0') {
            add_upstream(start, (size_t)(p - start));
            if (*p == '\0') break;
            start = p + 1;
        }
    }
}

static int listen_tcp(uint16_t port)
{
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &one, sizeof(one));
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
    int sndbuf = 262144;
    setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &sndbuf, sizeof(sndbuf));

    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }
    if (listen(fd, BACKLOG) < 0) {
        close(fd);
        return -1;
    }
    return fd;
}

int run_lb(uint16_t port, const char *upstreams_csv)
{
    signal(SIGPIPE, SIG_IGN);

    parse_upstreams(upstreams_csv);
    if (upstream_count == 0) {
        fprintf(stderr, "lb: no upstreams configured\n");
        return 1;
    }
    fprintf(stderr, "lb: %d upstreams configured\n", upstream_count);

    for (int i = 0; i < upstream_count; i++) {
        upstreams[i].fd = connect_wait(upstreams[i].path);
        fprintf(stderr, "lb: connected upstream %d -> %s\n", i, upstreams[i].path);
    }

    int listen_fd = listen_tcp(port);
    if (listen_fd < 0) {
        fprintf(stderr, "lb: listen failed on port %u\n", port);
        return 1;
    }
    fprintf(stderr, "lb: listening on port %u (backlog=%d, upstreams=%d)\n",
            port, BACKLOG, upstream_count);

    for (;;) {
        int client = accept4(listen_fd, NULL, NULL, SOCK_CLOEXEC);
        if (client < 0) {
            if (errno == EINTR) continue;
            continue;
        }
        scm_set_tcp_nodelay(client);
        /* TCP_QUICKACK: disable delayed ACK */
        int one_q = 1;
        setsockopt(client, IPPROTO_TCP, TCP_QUICKACK, &one_q, sizeof(one_q));
        /* Set non-blocking before passing to API */
        int fl = fcntl(client, F_GETFL, 0);
        if (fl >= 0) fcntl(client, F_SETFL, fl | O_NONBLOCK);

        int first = (int)(rr_next++ % (uint32_t)upstream_count);
        if (handoff(first, client) != 0) {
            for (int offset = 1; offset < upstream_count; offset++) {
                if (handoff((first + offset) % upstream_count, client) == 0) break;
            }
        }
        close(client);
    }
    return 0;
}