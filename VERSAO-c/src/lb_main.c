#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern int run_lb(uint16_t port, const char *upstreams_csv);

int main(void)
{
    const char *port_s = getenv("LB_PORT");
    if (!port_s || !*port_s) port_s = "9999";

    const char *upstreams = getenv("UPSTREAMS");
    if (upstreams && *upstreams) {
        return run_lb((uint16_t)atoi(port_s), upstreams);
    }

    const char *api1 = getenv("API1_SOCKET");
    if (!api1 || !*api1) api1 = "/tmp/sockets/api1.sock";
    const char *api2 = getenv("API2_SOCKET");
    if (!api2 || !*api2) api2 = "/tmp/sockets/api2.sock";

    const char *ch_s = getenv("CHANNELS_PER_API");
    int channels = (ch_s && *ch_s) ? atoi(ch_s) : 4;
    if (channels < 1) channels = 1;
    if (channels > 8) channels = 8;

    char buf[4096];
    int off = 0;
    for (int i = 0; i < channels; i++) {
        if (off > 0) buf[off++] = ',';
        off += snprintf(buf + off, sizeof(buf) - (size_t)off, "%s", api1);
    }
    for (int i = 0; i < channels; i++) {
        buf[off++] = ',';
        off += snprintf(buf + off, sizeof(buf) - (size_t)off, "%s", api2);
    }
    buf[off] = '\0';

    return run_lb((uint16_t)atoi(port_s), buf);
}
