## Testing

KISS includes a comprehensive test suite with over 100 tests covering security, performance, error handling, and production scenarios.

### Test Categories

The test suite is organized into 8 specialized modules:

#### Unit Tests (`tests/unit_tests.rs`)
- Path sanitization and normalization (21 tests)
- MIME type detection for various file extensions
- Health endpoint functionality and JSON responses
- Core library functions with edge cases

#### Security Tests (`tests/security_tests.rs`)  
- Directory traversal attack prevention (21 tests)
- Binary protection (blocking access to `/kiss` executable)
- Path sanitization against malicious inputs
- Security boundary validation

#### Integration Tests (`tests/integration_tests.rs`)
- HTTP request/response handling (8 tests) 
- End-to-end server functionality
- Request parsing and response formatting
- Static file serving workflows

#### Error Handling Tests (`tests/error_handling_tests.rs`)
- File system error scenarios (non-existent files, invalid paths)
- Connection error handling (timeouts, drops, incomplete requests)
- Resource exhaustion protection (very long paths, deep traversal)
- Unicode and special character handling

#### HTTP Protocol Tests (`tests/http_protocol_tests.rs`)
- Malformed request handling (invalid methods, versions, formats)
- Security header validation (CSP, X-Frame-Options, etc.)
- Content-Type and Content-Length header correctness
- HTTP edge cases and rapid connection scenarios

#### Performance Tests (`tests/performance_tests.rs`)
- Single request latency analysis with statistics
- Concurrent request throughput at different concurrency levels
- Memory usage under sustained load
- Path sanitization and MIME detection benchmarks
- Performance regression detection

#### Production Tests (`tests/production_tests.rs`)
- Graceful shutdown on SIGTERM/SIGINT signals
- Server startup and port binding validation
- Connection handling during shutdown
- Process lifecycle and reliability testing

#### Load Tests (`tests/load_tests.rs`)
- Worker pool behavior under high concurrency
- Connection queuing and bounded channel handling
- Thread pool saturation scenarios

### Running Tests

#### Basic Unit Tests
```bash
# Run all unit tests (no server required)
cargo test

# Run specific test module
cargo test unit_tests
cargo test security_tests
```

#### Integration and Performance Tests
Most integration and performance tests require a running server and are marked with `#[ignore]`:

```bash
# Start server in one terminal
cargo run --release

# Run all tests including server-dependent ones in another terminal
cargo test -- --include-ignored

# Run specific performance benchmarks
cargo test bench_ -- --include-ignored --nocapture

# Run production tests (requires release binary)
cargo build --release
cargo test production_tests -- --include-ignored
```

#### Test Categories by Requirement

**No Server Required:**
- Unit tests (path sanitization, MIME types)
- Security tests (input validation) 
- Error handling tests (most scenarios)

**Server Required:**
- Integration tests (HTTP functionality)
- Performance tests (latency, throughput)
- Production tests (graceful shutdown)
- Load tests (concurrency behavior)
