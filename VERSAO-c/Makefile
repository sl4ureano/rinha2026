CC = gcc
CFLAGS = -O3 -march=haswell -mtune=haswell -flto -fno-plt -fno-semantic-interposition \
	-Wall -Wextra -DNDEBUG -Iinclude
LDFLAGS = -flto -pthread

LIB_SRCS = src/index_mmap.c src/index_quantize.c src/knn.c src/distance_avx2.c \
	src/ingest_json.c src/ingest_features.c src/time_parse.c src/decision_tree.c src/tier_score.c \
	src/http_handler.c src/http_response.c src/platform_scm.c src/platform_fd_gateway.c

LIB_OBJS = $(LIB_SRCS:.c=.o)

.PHONY: all clean
all: server lb healthcheck score_one verify-tier tier_one

%.o: %.c
	$(CC) $(CFLAGS) -c -o $@ $<

server: src/server.c $(LIB_OBJS)
	$(CC) $(CFLAGS) -o $@ src/server.c $(LIB_OBJS) $(LDFLAGS)

lb: src/lb_main.c src/platform_lb.c src/platform_scm.c
	$(CC) $(CFLAGS) -o $@ src/lb_main.c src/platform_lb.c src/platform_scm.c $(LDFLAGS)

healthcheck: src/healthcheck.c
	$(CC) $(CFLAGS) -o $@ src/healthcheck.c $(LDFLAGS)

score_one: src/score_one.c $(LIB_OBJS)
	$(CC) $(CFLAGS) -o $@ src/score_one.c $(LIB_OBJS) $(LDFLAGS)

verify-tier: src/verify_tier.c $(LIB_OBJS)
	$(CC) $(CFLAGS) -o $@ src/verify_tier.c $(LIB_OBJS) $(LDFLAGS)

tier_one: src/tier_one.c src/ingest_json.o src/ingest_features.o src/time_parse.o src/decision_tree.o src/tier_score.o
	$(CC) $(CFLAGS) -o $@ src/tier_one.c src/ingest_json.o src/ingest_features.o src/time_parse.o src/decision_tree.o src/tier_score.o $(LDFLAGS)

clean:
	rm -f server lb healthcheck score_one verify-tier $(LIB_OBJS)
