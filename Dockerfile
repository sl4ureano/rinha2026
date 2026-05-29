## Stage 1: API + lb (tier-only mode — no index needed)
FROM rust:1.84-bookworm AS app-builder

WORKDIR /app
# Haswell ≈ Mac Mini da prova (linux-amd64); submission = sem k-NN/monoio no server.
ENV RUSTFLAGS="-C target-cpu=haswell -C opt-level=3"

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release --bin server --bin healthcheck --bin lb --no-default-features --features submission


## Stage 2: runtime (lean — no 99MB index)
FROM debian:bookworm-slim AS runtime

WORKDIR /app

COPY --from=app-builder /app/target/release/server /app/server
COPY --from=app-builder /app/target/release/healthcheck /app/healthcheck
COPY --from=app-builder /app/target/release/lb /app/lb

ENV PORT=8080

EXPOSE 8080

CMD ["./server"]
