#ifndef HTTP_H
#define HTTP_H

#include <stddef.h>
#include <stdint.h>

#include "index.h"

void serve_connection(int fd, const index_t *idx);

const uint8_t *resp_for_count(uint8_t count);
size_t resp_len_for_count(uint8_t count);
extern const uint8_t RESP_READY[];
extern const uint8_t RESP_DENIED_S10[];
extern const uint8_t RESP_NOT_FOUND[];

#define RESP_READY_LEN 65
#define RESP_DENIED_S10_LEN 130
#define RESP_NOT_FOUND_LEN 63

#endif
