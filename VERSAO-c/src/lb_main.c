#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern int run_lb(uint16_t port, const char *api1, const char *api2);

int main(void)
{
    const char *port_s = getenv("LB_PORT");
    if (!port_s || !*port_s) port_s = "9999";
    const char *api1 = getenv("API1_SOCKET");
    if (!api1 || !*api1) api1 = "/tmp/sockets/api1.sock";
    const char *api2 = getenv("API2_SOCKET");
    if (!api2 || !*api2) api2 = "/tmp/sockets/api2.sock";
    return run_lb((uint16_t)atoi(port_s), api1, api2);
}
