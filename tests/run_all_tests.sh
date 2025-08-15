#!/bin/bash

# KISS Test Runner - Simple tmux automation for server-dependent tests
# Build, run server from content dir, run tests in another window

set -e

echo "KISS Test Runner"
echo "=================="

# Check if tmux is available
if ! command -v tmux &> /dev/null; then
    echo "tmux is required but not installed. Please install tmux first."
    exit 1
fi

# Kill any existing test sessions
tmux kill-session -t kiss-tests 2>/dev/null || true
sleep 1

# Build release binary
echo "Building release binary..."
cargo build --release

# Create tmux session and run server from tests directory (which contains content subdir)
echo "Starting server from tests/ directory (serving tests/content/)..."
tmux new-session -d -s kiss-tests -n server
tmux send-keys -t kiss-tests:server "cd tests && ../target/release/kiss" Enter

# Wait for server to start
sleep 2

# Create second window for running tests
echo "Running tests..."
tmux new-window -t kiss-tests -n tests
tmux send-keys -t kiss-tests:tests "cargo test -- --include-ignored --nocapture" Enter


# Attach to the session
tmux attach-session -t kiss-tests