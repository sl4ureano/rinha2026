#define _GNU_SOURCE
#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/epoll.h>
#include <sys/socket.h>
#include <unistd.h>

extern int scm_connect_unix_retry(const char *path);
extern int scm_send_fd(int ctrl_fd, int client_fd);
extern void scm_set_nonblocking(int fd);
extern void scm_set_tcp_nodelay(int fd);
extern void scm_write_502(int fd);

static int tcp_listen(uint16_t port)
{
    int fd = socket(AF_INET, SOCK_STREAM | SOCK_CLOEXEC, 0);
    if (fd < 0) return -1;
    int one = 1;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof(one));
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons(port),
        .sin_addr.s_addr = htonl(INADDR_ANY),
    };
    if (bind(fd, (struct sockaddr *)&addr, sizeof(addr)) < 0) {
        close(fd);
        return -1;
    }
    if (listen(fd, 16384) < 0) {
        close(fd);
        return -1;
    }
    scm_set_nonblocking(fd);
    return fd;
}

int run_lb(uint16_t port, const char *api1_sock, const char *api2_sock)
{
    int ctrl1 = scm_connect_unix_retry(api1_sock);
    int ctrl2 = scm_connect_unix_retry(api2_sock);
    if (ctrl1 < 0 || ctrl2 < 0) {
        fprintf(stderr, "lb: failed to connect upstream uds\n");
        return 1;
    }

    int listen_fd = tcp_listen(port);
    if (listen_fd < 0) {
        fprintf(stderr, "lb: listen failed\n");
        return 1;
    }
    scm_set_nonblocking(listen_fd);

    int epfd = epoll_create1(EPOLL_CLOEXEC);
    struct epoll_event ev = {.events = EPOLLIN | EPOLLET, .data.fd = listen_fd};
    epoll_ctl(epfd, EPOLL_CTL_ADD, listen_fd, &ev);

    struct epoll_event events[256];
    int ctrl[2] = {ctrl1, ctrl2};
    int next = 0;

    for (;;) {
        int n = epoll_wait(epfd, events, 256, -1);
        if (n < 0) {
            if (errno == EINTR) continue;
            break;
        }
        for (int i = 0; i < n; i++) {
            if (!(events[i].events & EPOLLIN)) continue;
            for (;;) {
                int client = accept4(listen_fd, NULL, NULL, SOCK_CLOEXEC);
                if (client < 0) {
                    if (errno == EAGAIN || errno == EWOULDBLOCK) break;
                    if (errno == EINTR) continue;
                    break;
                }
                scm_set_tcp_nodelay(client);
                int primary = next;
                next ^= 1;
                if (!scm_send_fd(ctrl[primary], client) && !scm_send_fd(ctrl[primary ^ 1], client))
                    scm_write_502(client);
                close(client);
            }
        }
    }
    return 0;
}
