## Stage 1a: índice offline
FROM --platform=linux/amd64 rust:1.84-bookworm AS index-builder

WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=haswell"

COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > src/lb.rs \
    && printf 'fn main() {}\n' > src/bin/healthcheck.rs

RUN cargo build --release --bin build-index 2>/dev/null || cargo build --release --bin build-index


## Stage 1b: API + lb
FROM --platform=linux/amd64 rust:1.84-bookworm AS app-builder

WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=haswell"

COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN cargo build --release --bin server --bin healthcheck --bin lb


## Stage 2: gerar index.bin
FROM --platform=linux/amd64 debian:bookworm-slim AS indexer

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*

COPY --from=index-builder /app/target/release/build-index /app/build-index
COPY resources/ resources/
COPY data/ /app/data/

RUN if [ ! -f resources/references.json.gz ]; then \
      wget -q -O resources/references.json.gz \
        "https://github.com/zanfranceschi/rinha-de-backend-2026/raw/main/resources/references.json.gz"; \
    fi

# Inline (evita CRLF em .sh no Windows quebrando #!/bin/sh no Linux)
ARG LEAF_SIZE=96
RUN mkdir -p data && \
    echo "building index leaf=${LEAF_SIZE}..." && \
    /app/build-index resources data/index.bin ${LEAF_SIZE}


## Stage 3: runtime
FROM --platform=linux/amd64 debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=app-builder /app/target/release/server /app/server
COPY --from=app-builder /app/target/release/healthcheck /app/healthcheck
COPY --from=app-builder /app/target/release/lb /app/lb
COPY --from=indexer /app/data/index.bin /app/data/index.bin

ENV INDEX_PATH=/app/data/index.bin
ENV PORT=8080

EXPOSE 8080

CMD ["./server"]
