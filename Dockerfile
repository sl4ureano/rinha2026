## Stage 1: build API + lb + build-index (hybrid: fast_path + k-NN)
FROM rust:1.84-bookworm AS app-builder

WORKDIR /app
# Haswell ≈ Mac Mini da prova (linux-amd64); submission agora inclui k-NN.
ENV RUSTFLAGS="-C target-cpu=haswell -C opt-level=3"

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release --bin server --bin healthcheck --bin lb --bin build-index --bin verify-tier --bin verify-fast-tier --bin analyze-tree-split --bin analyze-fast-expand --no-default-features --features submission


## Stage 2: build k-NN index from references
FROM app-builder AS index-builder

COPY resources/ resources/
RUN mkdir -p data && ./target/release/build-index resources data/index.bin


## Stage 3: runtime (with index for hybrid scoring)
FROM debian:bookworm-slim AS runtime

WORKDIR /app

COPY --from=app-builder /app/target/release/server /app/server
COPY --from=app-builder /app/target/release/healthcheck /app/healthcheck
COPY --from=app-builder /app/target/release/lb /app/lb
COPY --from=index-builder /app/data/index.bin /app/data/index.bin

ENV PORT=8080

EXPOSE 8080

CMD ["./server"]
