# Contexto: raiz do repo (docker-compose usa context: ..)

FROM --platform=linux/amd64 rust:1.84-bookworm AS index-builder

WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=haswell"

COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN printf 'fn main() {}\n' > src/main.rs \
    && printf 'fn main() {}\n' > src/lb.rs \
    && printf 'fn main() {}\n' > src/bin/healthcheck.rs

RUN cargo build --release --bin build-index


FROM --platform=linux/amd64 debian:bookworm-slim AS indexer

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates wget && rm -rf /var/lib/apt/lists/*

COPY --from=index-builder /app/target/release/build-index /app/build-index
COPY resources/ resources/

RUN if [ ! -f resources/references.json.gz ]; then \
      wget -q -O resources/references.json.gz \
        "https://github.com/zanfranceschi/rinha-de-backend-2026/raw/main/resources/references.json.gz"; \
    fi

ARG LEAF_SIZE=96
RUN mkdir -p data && \
    /app/build-index resources data/index.bin ${LEAF_SIZE}


FROM --platform=linux/amd64 debian:bookworm-slim AS c-builder

RUN apt-get update && apt-get install -y --no-install-recommends build-essential && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY VERSAO-c/ ./

RUN make clean && make all


FROM --platform=linux/amd64 debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=c-builder /build/server /app/server
COPY --from=c-builder /build/lb /app/lb
COPY --from=c-builder /build/healthcheck /app/healthcheck
COPY --from=indexer /app/data/index.bin /app/data/index.bin

ENV INDEX_PATH=/app/data/index.bin
ENV PORT=8080

EXPOSE 8080

CMD ["./server"]
