# rust:alpine targets musl natively, so the release binary is fully static
FROM rust:1.99-alpine AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked && mkdir -m 755 /content

FROM scratch
COPY --from=builder /app/target/release/kiss /kiss
COPY --from=builder /content /content
EXPOSE 8080
ENTRYPOINT ["/kiss"]
