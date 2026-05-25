#define _GNU_SOURCE
#include "http.h"
#include "index.h"

#include <errno.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/un.h>
#include <unistd.h>

extern int scm_recv_fd(int control_fd);
extern int scm_recv_fd_nonblock(int control_fd);
extern void scm_set_tcp_nodelay(int fd);

typedef struct {
    const index_t *idx;
    int fd;
} conn_arg_t;

typedef struct {
    int control_fd;
    const index_t *idx;
} control_ctx_t;

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

static void recv_loop_fixed(int control_fd, const index_t *idx)
{
    for (;;) {
        int fd = scm_recv_fd(control_fd);
        if (fd < 0) {
            close(control_fd);
            return;
        }
        scm_set_tcp_nodelay(fd);
        spawn_conn(idx, fd);
        for (;;) {
            int fd2 = scm_recv_fd_nonblock(control_fd);
            if (fd2 < 0) break;
            scm_set_tcp_nodelay(fd2);
            spawn_conn(idx, fd2);
        }
    }
}

static void *control_thread(void *arg)
{
    control_ctx_t *ctx = (control_ctx_t *)arg;
    recv_loop_fixed(ctx->control_fd, ctx->idx);
    free(ctx);
    return NULL;
}

int fd_gateway_run(const index_t *idx, const char *sock_path)
{
    char parent[512];
    strncpy(parent, sock_path, sizeof(parent) - 1);
    parent[sizeof(parent) - 1] = '\0';
    char *slash = strrchr(parent, '/');
    if (slash && slash != parent) {
        *slash = '\0';
        mkdir(parent, 0755);
    }
    unlink(sock_path);

    int listener = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (listener < 0) return -1;
    struct sockaddr_un addr = {.sun_family = AF_UNIX};
    size_t plen = strlen(sock_path);
    if (plen >= sizeof(addr.sun_path)) {
        close(listener);
        return -1;
    }
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

    fprintf(stderr, "listening uds fd-pass %s\n", sock_path);

    for (;;) {
        int control = accept4(listener, NULL, NULL, SOCK_CLOEXEC);
        if (control < 0) {
            if (errno == EINTR) continue;
            continue;
        }
        control_ctx_t *ctx = malloc(sizeof(*ctx));
        if (!ctx) {
            close(control);
            continue;
        }
        ctx->control_fd = control;
        ctx->idx = idx;
        pthread_t t;
        if (pthread_create(&t, NULL, control_thread, ctx) != 0) {
            close(control);
            free(ctx);
        } else {
            pthread_detach(t);
        }
    }
}
