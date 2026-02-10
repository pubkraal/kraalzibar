FROM rust:1.88-bookworm AS builder

RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY proto/ proto/

RUN cargo build --release --bin kraalzibar-server

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

RUN groupadd -r kraalzibar && useradd -r -g kraalzibar kraalzibar

COPY --from=builder /app/target/release/kraalzibar-server /usr/local/bin/kraalzibar-server

USER kraalzibar

EXPOSE 50051 8080

ENTRYPOINT ["kraalzibar-server"]
