#define _GNU_SOURCE
#include <arpa/inet.h>
#include <netinet/in.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <unistd.h>

static int probe(int fd)
{
    const char req[] = "GET /ready HTTP/1.0\r\n\r\n";
    if (write(fd, req, sizeof(req) - 1) < 0) return 0;
    char buf[256];
    ssize_t n = read(fd, buf, sizeof(buf));
    if (n <= 0) return 0;
    for (ssize_t i = 0; i + 2 <= n; i++) {
        if (buf[i] == 'O' && buf[i + 1] == 'K') return 1;
    }
    return 0;
}

int main(void)
{
    const char *sock = getenv("SOCKET_PATH");
    const char *port = getenv("PORT");
    if (!port || !*port) port = "8080";

    if (sock && *sock) {
        int fd = socket(AF_UNIX, SOCK_STREAM, 0);
        struct sockaddr_un addr = {.sun_family = AF_UNIX};
        strncpy(addr.sun_path, sock, sizeof(addr.sun_path) - 1);
        if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) == 0 && probe(fd)) {
            close(fd);
            return 0;
        }
        if (fd >= 0) close(fd);
    }

    int fd = socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in addr = {
        .sin_family = AF_INET,
        .sin_port = htons((uint16_t)atoi(port)),
    };
    inet_pton(AF_INET, "127.0.0.1", &addr.sin_addr);
    if (connect(fd, (struct sockaddr *)&addr, sizeof(addr)) == 0 && probe(fd)) {
        close(fd);
        return 0;
    }
    if (fd >= 0) close(fd);
    return 1;
}
