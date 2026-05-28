## Stage 1: API + lb (tier-only mode — no index needed)
FROM rust:1.84-bookworm AS app-builder

WORKDIR /app
ENV RUSTFLAGS="-C target-cpu=native -C opt-level=3"

COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release --bin server --bin healthcheck --bin lb


## Stage 2: runtime (lean — no 99MB index)
FROM debian:bookworm-slim AS runtime

WORKDIR /app

COPY --from=app-builder /app/target/release/server /app/server
COPY --from=app-builder /app/target/release/healthcheck /app/healthcheck
COPY --from=app-builder /app/target/release/lb /app/lb

ENV PORT=8080

EXPOSE 8080

CMD ["./server"]
