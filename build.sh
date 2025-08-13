#!/bin/bash
set -euo pipefail

echo "Building minimal Rust HTTP server container..."
echo "Note: Build includes comprehensive test execution to ensure code quality"
echo ""

# Build the container image (includes testing phase)
podman build -t kiss:latest .

# Show image size
echo ""
echo "Container image built successfully (all tests passed):"
podman images kiss:latest

echo ""
echo "To run the container:"
echo "  podman run -p 8080:8080 kiss:latest"

echo ""
echo "To test security (run container read-only):"
echo "  podman run -p 8080:8080 --read-only kiss:latest"

echo ""
echo "For local development, you can run tests separately:"
echo "  cargo test                    # Run all tests"
echo "  cargo test unit_tests        # Run unit tests only"
echo "  cargo test security_tests    # Run security tests only"