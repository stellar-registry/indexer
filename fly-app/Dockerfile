FROM rust:1.92 AS builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./

# Cache dependency build
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Build real application
COPY src ./src
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/fly-app /usr/local/bin/fly-app
EXPOSE 8080
CMD ["fly-app"]
