FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY proto/ proto/

RUN cargo build --release --bin kraalzibar-server

FROM debian:bookworm-slim

ARG VERSION=dev

LABEL org.opencontainers.image.source="https://github.com/pubkraal/kraalzibar"
LABEL org.opencontainers.image.version="${VERSION}"
LABEL org.opencontainers.image.description="Zanzibar-inspired ReBAC authorization server"
LABEL org.opencontainers.image.licenses="MIT"

RUN apt-get update && apt-get install -y ca-certificates curl && rm -rf /var/lib/apt/lists/*

RUN groupadd -r kraalzibar && useradd -r -g kraalzibar kraalzibar

COPY --from=builder /app/target/release/kraalzibar-server /usr/local/bin/kraalzibar-server

USER kraalzibar

EXPOSE 50051 8080

HEALTHCHECK --interval=10s --timeout=3s --start-period=15s --retries=3 \
  CMD curl -sf http://localhost:8080/healthz || exit 1

ENTRYPOINT ["kraalzibar-server"]
