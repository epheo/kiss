#!/bin/bash
set -euo pipefail

echo "Building minimal Rust HTTP server container..."

# Build the container image
podman build -t kiss:latest .

# Show image size
echo ""
echo "Container image built successfully:"
podman images kiss:latest

echo ""
echo "To run the container:"
echo "  podman run -p 8080:8080 kiss:latest"

echo ""
echo "To test security (run container read-only):"
echo "  podman run -p 8080:8080 --read-only kiss:latest"