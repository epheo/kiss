# Multi-stage build for minimal container
FROM rust:1.75-alpine AS builder

# Install musl target for static linking
RUN apk add --no-cache musl-dev
RUN rustup target add x86_64-unknown-linux-musl

# Create app directory
WORKDIR /app

# Copy Cargo files
COPY Cargo.toml ./

# Copy source code
COPY src ./src

# Build static binary
RUN cargo build --release --target x86_64-unknown-linux-musl

# Create static directory for file serving
RUN mkdir -p /app/static

# Final stage: scratch image
FROM scratch

# Copy the static directory with OpenShift-compatible permissions
COPY --from=builder /app/static /app/static

# Copy the static binary
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/kiss /kiss

# Expose port
EXPOSE 8080

# Run the server
ENTRYPOINT ["/kiss"]