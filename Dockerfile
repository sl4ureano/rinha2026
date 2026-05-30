## Stage 1: instrumented build (LLVM PGO, Haswell = Mac Mini da prova)
FROM rust:1.84-bookworm AS pgo-instrument
WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=haswell -Cprofile-generate=/tmp/pgo-data"

COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY resources/example-payloads.json resources/

RUN cargo build --release \
    --bin server --bin healthcheck --bin lb --bin build-index --bin pgo-train \
    --no-default-features --features submission

RUN ./target/release/pgo-train resources/example-payloads.json


## Stage 2: PGO-optimized release binaries
FROM rust:1.84-bookworm AS app-builder
WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=haswell -Cprofile-use=/tmp/pgo-data"

COPY --from=pgo-instrument /tmp/pgo-data /tmp/pgo-data
COPY Cargo.toml Cargo.lock ./
COPY src/ src/
COPY test/test-data.json test/

RUN cargo build --release \
    --bin server --bin healthcheck --bin lb --bin build-index --bin verify-tier \
    --no-default-features --features submission \
    && ./target/release/verify-tier test/test-data.json


## Stage 3: k-NN index
FROM app-builder AS index-builder
COPY resources/ resources/
RUN mkdir -p data && ./target/release/build-index resources data/index.bin


## Stage 4: runtime
FROM debian:bookworm-slim AS runtime
WORKDIR /app

COPY --from=app-builder /app/target/release/server /app/server
COPY --from=app-builder /app/target/release/healthcheck /app/healthcheck
COPY --from=app-builder /app/target/release/lb /app/lb
COPY --from=index-builder /app/data/index.bin /app/data/index.bin

ENV PORT=8080
EXPOSE 8080
CMD ["./server"]
