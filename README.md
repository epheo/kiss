Kubernetes Instant Static Server
================================

-- Keep It Simple Stupid

This is a minimalistic **base image** for serving static files behind a Kubernetes ingress controller.

KISS provides a secure, lightweight foundation that users extend with their own static content. The server serves files from the container root directory (`/`) while protecting the server binary at `/kiss`.

## What KISS Does NOT Implement (By Design)

The following features are deliberately omitted from KISS because they are handled by the Kubernetes ingress controller:

- **TLS/SSL Termination** - Ingress handles certificates, encryption, and HTTPS
- **Load Balancing** - Ingress distributes traffic across multiple KISS pods  
- **Domain Routing** - Ingress routes based on hostnames and paths
- **Rate Limiting** - Ingress can throttle requests before they reach KISS
- **Authentication** - Ingress handles OAuth, JWT validation, etc.
- **Compression** - Ingress can add gzip/brotli compression
- **HTTP/2 & HTTP/3** - Ingress provides modern protocol support
- **URL Rewriting** - Ingress handles path manipulation and redirects

This division follows cloud-native principles where each component has a single responsibility, reducing complexity and attack surface.

## Usage

KISS is designed as a **base image** that you extend with your own static content. 

### Base Image Approach

Create a `Dockerfile` that builds upon KISS:

#### Example 1: Serving Static Files
```dockerfile
FROM kiss:latest
COPY ./my-website/ /
```

Your directory structure:
```
my-project/
├── Dockerfile
└── my-website/
    ├── index.html
    ├── style.css
    ├── js/
    │   └── app.js
    └── images/
        └── logo.png
```

Build and run:
```bash
docker build -t my-website .
docker run -p 8080:8080 --read-only my-website
```

#### Example 2: Sphinx Documentation
```dockerfile
# Build documentation
FROM sphinxdoc/sphinx:latest AS builder
WORKDIR /docs
COPY . .
RUN sphinx-build -b html . _build/html

# Serve with KISS
FROM kiss:latest
COPY --from=builder /docs/_build/html/ /
```

Your documentation project:
```
docs-project/
├── Dockerfile
├── conf.py
├── index.rst
├── _static/
└── _templates/
```

Build and run:
```bash
docker build -t my-docs .
docker run -p 8080:8080 --read-only my-docs
```

**Kubernetes Deployment:**
```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: my-website
spec:
  replicas: 3
  selector:
    matchLabels:
      app: my-website
  template:
    metadata:
      labels:
        app: my-website
    spec:
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        runAsGroup: 65534
      containers:
      - name: kiss
        image: my-website:latest  # Your custom image
        ports:
        - containerPort: 8080
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop: [ALL]
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
        readinessProbe:
          httpGet:
            path: /ready
            port: 8080
```

### Health Checks

The server provides endpoints for Kubernetes probes:

```bash
curl http://localhost:8080/health   # Health check
curl http://localhost:8080/ready    # Readiness check
```

## Platform Compatibility

KISS is designed to run as a rootless container on both vanilla Kubernetes and OpenShift.

### Vanilla Kubernetes

Works with any SecurityContext that specifies a non-root user:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kiss-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: kiss-server
  template:
    metadata:
      labels:
        app: kiss-server
    spec:
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        runAsGroup: 65534
        fsGroup: 65534
      containers:
      - name: kiss
        image: kiss:latest
        ports:
        - containerPort: 8080
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop:
            - ALL
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
        readinessProbe:
          httpGet:
            path: /ready
            port: 8080
        volumeMounts:
        - name: static-files
          mountPath: /app/static
          readOnly: true
      volumes:
      - name: static-files
        configMap:
          name: static-content
```

### OpenShift

Compatible with OpenShift's default `restricted-v2` Security Context Constraint:

- **Arbitrary UID Assignment**: OpenShift assigns random UIDs (1000000000+ range) but always uses GID 0 (root group)
- **No USER Directive**: Container allows OpenShift to assign any UID
- **Default Permissions**: Standard directory permissions (755) provide sufficient read access
- **Read-Only Operations**: Server only reads files, never writes to filesystem

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: kiss-server
spec:
  replicas: 3
  selector:
    matchLabels:
      app: kiss-server
  template:
    metadata:
      labels:
        app: kiss-server
    spec:
      containers:
      - name: kiss
        image: kiss:latest
        ports:
        - containerPort: 8080
        securityContext:
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: true
          capabilities:
            drop:
            - ALL
        livenessProbe:
          httpGet:
            path: /health
            port: 8080
        readinessProbe:
          httpGet:
            path: /ready
            port: 8080
        volumeMounts:
        - name: static-files
          mountPath: /app/static
          readOnly: true
      volumes:
      - name: static-files
        configMap:
          name: static-content
```

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

### Quality Assurance

The comprehensive test suite ensures:
- **Security**: All path sanitization and access control mechanisms
- **Reliability**: Error handling and graceful degradation
- **Performance**: Latency and throughput requirements are met
- **Production Readiness**: Signal handling and lifecycle management

Tests can be run locally during development or integrated into CI/CD pipelines for continuous quality assurance.

## Security Features

KISS is designed with security as a primary concern:

### Container Security
- **Scratch Base Image**: Minimal attack surface with no OS packages, shell, or utilities
- **Static Binary**: Single Rust binary with no runtime dependencies
- **Rootless Operation**: Runs as non-privileged user on both Kubernetes and OpenShift
- **Read-Only Filesystem**: Compatible with `readOnlyRootFilesystem: true`
- **No Privilege Escalation**: Designed to run with `allowPrivilegeEscalation: false`
- **Minimal Capabilities**: Functions with all Linux capabilities dropped

### Network Security  
- **Non-Privileged Port**: Runs on port 8080 (>1024) for non-root compatibility
- **Bounded Request Size**: Maximum 8KB request size prevents memory exhaustion
- **File Size Limits**: 50MB maximum file size served
- **Path Sanitization**: Prevents access to server binary and normalizes paths
- **Binary Protection**: Blocks all access attempts to `/kiss` executable

### Operational Security
- **Graceful Shutdown**: Handles SIGTERM/SIGINT for clean container termination
- **Health Endpoints**: Separate `/health` and `/ready` endpoints for monitoring
- **Worker Pool**: Bounded connection handling prevents resource exhaustion
- **No File Writes**: Server only reads files, never modifies filesystem
