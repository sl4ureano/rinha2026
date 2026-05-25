#define _GNU_SOURCE
#include <errno.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <stdint.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <sys/un.h>
#include <unistd.h>

int scm_connect_unix_retry(const char *path)
{
    for (;;) {
        int fd = socket(AF_UNIX, SOCK_STREAM | SOCK_CLOEXEC, 0);
        if (fd < 0) return -1;
        struct sockaddr_un addr = {.sun_family = AF_UNIX};
        size_t plen = strlen(path);
        if (plen >= sizeof(addr.sun_path)) {
            close(fd);
            return -1;
        }
        memcpy(addr.sun_path, path, plen + 1);
        if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) == 0) return fd;
        int e = errno;
        close(fd);
        if (e == ENOENT || e == ECONNREFUSED || e == EAGAIN) {
            usleep(100000);
            continue;
        }
        return -1;
    }
}

void scm_set_nonblocking(int fd)
{
    int fl = fcntl(fd, F_GETFL, 0);
    fcntl(fd, F_SETFL, fl | O_NONBLOCK);
}

void scm_set_tcp_nodelay(int fd)
{
    int one = 1;
    setsockopt(fd, IPPROTO_TCP, TCP_NODELAY, &one, sizeof(one));
}

int scm_send_fd(int ctrl_fd, int client_fd)
{
    char buf[1] = {0};
    struct msghdr msg = {0};
    struct iovec iov = {.iov_base = buf, .iov_len = 1};
    msg.msg_iov = &iov;
    msg.msg_iovlen = 1;
    union {
        char cbuf[CMSG_SPACE(sizeof(int))];
        struct cmsghdr align;
    } u;
    msg.msg_control = u.cbuf;
    msg.msg_controllen = CMSG_SPACE(sizeof(int));
    struct cmsghdr *c = CMSG_FIRSTHDR(&msg);
    c->cmsg_level = SOL_SOCKET;
    c->cmsg_type = SCM_RIGHTS;
    c->cmsg_len = CMSG_LEN(sizeof(int));
    memcpy(CMSG_DATA(c), &client_fd, sizeof(int));
    ssize_t n = sendmsg(ctrl_fd, &msg, MSG_NOSIGNAL);
    return n == 1;
}

void scm_write_502(int fd)
{
    static const char resp[] =
        "HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    (void)write(fd, resp, sizeof(resp) - 1);
}

static int recv_fd_flags(int control_fd, int flags, int *out_fd)
{
    char buf[1];
    union {
        char cbuf[CMSG_SPACE(sizeof(int))];
        struct cmsghdr align;
    } u;
    for (;;) {
        struct iovec iov = {.iov_base = buf, .iov_len = 1};
        struct msghdr msg = {0};
        msg.msg_iov = &iov;
        msg.msg_iovlen = 1;
        msg.msg_control = u.cbuf;
        msg.msg_controllen = sizeof(u.cbuf);
        ssize_t n = recvmsg(control_fd, &msg, flags);
        if (n < 0) {
            if (errno == EINTR) continue;
            return 0;
        }
        if (n == 0) return 0;
        for (struct cmsghdr *c = CMSG_FIRSTHDR(&msg); c; c = CMSG_NXTHDR(&msg, c)) {
            if (c->cmsg_level == SOL_SOCKET && c->cmsg_type == SCM_RIGHTS) {
                memcpy(out_fd, CMSG_DATA(c), sizeof(int));
                return 1;
            }
        }
        return 0;
    }
}

int scm_recv_fd(int control_fd)
{
    int fd = -1;
    if (recv_fd_flags(control_fd, 0, &fd)) return fd;
    return -1;
}

int scm_recv_fd_nonblock(int control_fd)
{
    int fd = -1;
    if (recv_fd_flags(control_fd, MSG_DONTWAIT, &fd)) return fd;
    return -1;
}
