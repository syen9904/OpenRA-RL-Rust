FROM rust:1.88-slim AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY . .
RUN cargo build --release
RUN cargo test --release

# Dev stage — for interactive development
FROM rust:1.88-slim AS dev

RUN apt-get update && apt-get install -y \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

# Install wasm-pack for browser builds
RUN cargo install wasm-pack

WORKDIR /app
VOLUME /app

CMD ["cargo", "test"]
