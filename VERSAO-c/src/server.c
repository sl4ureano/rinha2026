#define _GNU_SOURCE
#include "http.h"
#include "index.h"
#include <arpa/inet.h>
#include <netinet/tcp.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>
#include <sys/socket.h>
#include <unistd.h>

extern int fd_gateway_run(const index_t *idx, const char *sock_path);

typedef struct {
    const index_t *idx;
    int fd;
} conn_arg_t;

typedef struct {
    const index_t *idx;
    uint16_t port;
} health_arg_t;

static void *conn_thread(void *arg)
{
    conn_arg_t *a = (conn_arg_t *)arg;
    serve_connection(a->fd, a->idx);
    free(a);
    return NULL;
}

static void spawn_conn(const index_t *idx, int fd)
{
    conn_arg_t *a = malloc(sizeof(*a));
    if (!a) {
        close(fd);
        return;
    }
    a->idx = idx;
    a->fd = fd;
    pthread_t t;
    pthread_attr_t attr;
    pthread_attr_init(&attr);
    pthread_attr_setdetachstate(&attr, PTHREAD_CREATE_DETACHED);
    pthread_attr_setstacksize(&attr, 256 * 1024);
    if (pthread_create(&t, &attr, conn_thread, a) != 0) {
        close(fd);
        free(a);
    }
    pthread_attr_destroy(&attr);
}

static void *health_thread(void *arg)
{
    health_arg_t *ha = (health_arg_t *)arg;
    const index_t *idx = ha->idx;
    uint16_t port = ha->port;
    free(ha);
    int lfd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (lfd < 0) return NULL;
    int one = 1;
    setsockopt(lfd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(lfd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(lfd);
        return NULL;
    }
    listen(lfd, 128);
    for (;;) {
        int c = accept4(lfd, NULL, NULL, SOCK_CLOEXEC);
        if (c < 0) continue;
        char buf[512];
        ssize_t n = read(c, buf, sizeof(buf));
        if (n > 0) {
            for (ssize_t i = 0; i + 6 <= n; i++) {
                if (memcmp(buf + i, "/ready", 6) == 0) {
                    const uint8_t *r = resp_ready(idx && idx->ready);
                    size_t rl = resp_ready_len(idx && idx->ready);
                    write(c, r, rl);
                    break;
                }
            }
        }
        close(c);
    }
}

static const char *env_or(const char *key, const char *def)
{
    const char *v = getenv(key);
    return (v && *v) ? v : def;
}

static int env_truthy(const char *key)
{
    const char *v = getenv(key);
    return v && (strcmp(v, "1") == 0 || strcasecmp(v, "true") == 0);
}

static int tier_only_mode(void)
{
    if (env_truthy("TIER_ONLY") || env_truthy("SKIP_INDEX")) return 1;
    return env_truthy("FD_PASS");
}

int main(void)
{
    const char *index_path = env_or("INDEX_PATH", "/app/data/index.bin");

    index_t idx;
    if (tier_only_mode()) {
        index_init_empty(&idx);
        idx.ready = 1;
        fprintf(stderr, "tier-only: index mmap skipped\n");
    } else if (index_open(&idx, index_path) != 0) {
        fprintf(stderr, "index open %s failed\n", index_path);
        return 1;
    } else {
        fprintf(stderr, "index: %u partitions, %u nodes, %u blocks\n",
                index_part_count(&idx), index_node_count(&idx), index_block_count(&idx));
        fprintf(stderr, "warming up index...\n");
        index_warmup(&idx);
        fprintf(stderr, "index warm\n");
    }
    const char *ctrl = getenv("CTRL_SOCK");
    if (!ctrl || !*ctrl) ctrl = getenv("RINHA_FD_SOCK");
    if (!ctrl || !*ctrl) {
        const char *fd_pass = getenv("FD_PASS");
        if (fd_pass && (strcmp(fd_pass, "1") == 0 || strcasecmp(fd_pass, "true") == 0))
            ctrl = getenv("SOCKET_PATH");
    }

    if (ctrl && *ctrl) {
        health_arg_t *hp = malloc(sizeof(*hp));
        hp->idx = &idx;
        hp->port = (uint16_t)atoi(env_or("PORT", "8080"));
        pthread_t ht;
        pthread_create(&ht, NULL, health_thread, hp);
        pthread_detach(ht);
        int rc = fd_gateway_run(&idx, ctrl);
        index_close(&idx);
        return rc != 0 ? 1 : 0;
    }

    const char *bind_addr = getenv("BIND");
    char bind_buf[64];
    if (!bind_addr || !*bind_addr) {
        snprintf(bind_buf, sizeof(bind_buf), "0.0.0.0:%s", env_or("PORT", "8080"));
        bind_addr = bind_buf;
    }

    char host[64] = "0.0.0.0";
    uint16_t port = 8080;
    char *colon = strrchr((char *)bind_addr, ':');
    if (colon) {
        size_t hlen = (size_t)(colon - bind_addr);
        if (hlen < sizeof(host)) {
            memcpy(host, bind_addr, hlen);
            host[hlen] = '\0';
        }
        port = (uint16_t)atoi(colon + 1);
    }

    int listen_fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (strcmp(host, "0.0.0.0") != 0) inet_pton(AF_INET, host, &addr.sin_addr);
    int one = 1;
    setsockopt(listen_fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    bind(listen_fd, (struct sockaddr *)&addr, sizeof(addr));
    listen(listen_fd, 16384);
    fprintf(stderr, "listening tcp %s:%u\n", host, port);

    for (;;) {
        int c = accept4(listen_fd, NULL, NULL, SOCK_CLOEXEC);
        if (c < 0) continue;
        int nd = 1;
        setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &nd, sizeof(nd));
        spawn_conn(&idx, c);
    }
}
