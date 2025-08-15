# Multi-stage build for minimal container
FROM rust:1.75-alpine AS builder

# Install musl target for static linking
RUN apk add --no-cache musl-dev
RUN rustup target add x86_64-unknown-linux-musl

# Set working directory
WORKDIR /app

# Copy Cargo files
COPY Cargo.toml ./

# Copy source code
COPY src ./src

# Build static binary
RUN cargo build --release --target x86_64-unknown-linux-musl

# Final stage: scratch image
FROM scratch

# Copy the static binary
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/kiss /kiss

# Create content directory with proper permissions
# Using busybox temporarily to create directory
FROM busybox AS dirbuilder
RUN mkdir -p /content && chmod 755 /content

# Back to scratch for final image
FROM scratch

# Copy the static binary
COPY --from=builder /app/target/x86_64-unknown-linux-musl/release/kiss /kiss

# Copy content directory
COPY --from=dirbuilder /content /content

# Expose port
EXPOSE 8080

# Run the server
ENTRYPOINT ["/kiss"]
