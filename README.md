# KISS Instant Static Server

[![Container Repository on Quay](https://quay.io/repository/epheo/kiss/status "Container Repository on Quay")](https://quay.io/repository/epheo/kiss)

KISS (Kubernetes Instant Static Server) is a high-performance async static file server written in Rust, designed as a minimalistic base image for Kubernetes deployments with zero-overhead configuration.

## Overview

KISS implements an advanced lock-free caching architecture with pre-generated responses, eliminating disk I/O and memory allocations during request processing. This approach is designed for static websites, single-page applications, and documentation in containerized environments.

**Characteristics:**
- **Lock-free file caching** with zero disk I/O during requests  
- **Zero-allocation hot paths** with pre-allocated buffer reuse
- **Async Tokio runtime** with per-connection task spawning
- **CLI/Environment configuration** with zero runtime overhead
- **Single-purpose design** focused on maximum performance static content delivery
- **Container-optimized** for Kubernetes deployments
- **Minimal dependencies** and attack surface

KISS serves files from the container content directory (`/content/`) while protecting the server binary at `/kiss`.


## What KISS does NOT implement (by design)

The following features are deliberately omitted from KISS because they are handled by the Kubernetes ingress controller:

- **TLS/SSL Termination** - Ingress handles certificates, encryption, and HTTPS
- **Load Balancing** - Ingress distributes traffic across multiple KISS pods  
- **Domain Routing** - Ingress routes based on hostnames and paths
- **Rate Limiting** - Ingress can throttle requests before they reach KISS
- **Authentication** - Ingress handles OAuth, JWT validation, etc.
- **Compression** - Ingress can add gzip/brotli compression
- **HTTP/2 & HTTP/3** - Ingress provides modern protocol support
- **URL Rewriting** - Ingress handles path manipulation and redirects
- **POST/PUT** - This is a static file serving only server

This division follows cloud-native principles where each component has a single responsibility, reducing complexity and attack surface.

## What KISS implements

- **GET/HEAD**: Well; This is static file server
- **Ultra-low latency**: Async Tokio with zero-copy responses and lock-free caching
- **Memory-optimized**: Cache-line efficient structs, pre-allocated buffers, Arc-based sharing
- **Zero I/O overhead**: Complete file preloading at startup with pre-generated responses
- **Algorithmic efficiency**: FNV hashing, single-pass processing, Boyer-Moore-like matching
- **Single-write responses**: Headers + content combined for minimal syscalls
- **Lock-free concurrency**: Atomic RCU pattern for cache access without contention
- **Zero-allocation hot paths**: Buffer reuse, direct byte manipulation
- **Predictable performance**: No GC, minimal branching, cache-friendly access patterns
- **Container optimized**: Scratch image, configurable via CLI/environment variables

## Usage

KISS is designed as a **base image** that you extend with your own static content. 

### Base Image Approach

Create a `Dockerfile` that builds upon KISS:

#### Example 1: Serving Static Files
```dockerfile
FROM quay.io/epheo/kiss:latest
COPY ./my-website/ /content/
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
podman build -t my-website .
podman run -p 8080:8080 --read-only my-website
```

#### Example 2: Sphinx Documentation
```dockerfile
# Build documentation
FROM sphinxdoc/sphinx:latest AS builder
WORKDIR /docs
COPY . .
RUN sphinx-build -b html . _build/html

# Serve with KISS
FROM quay.io/epheo/kiss:latest
COPY --from=builder /docs/_build/html/ /content/
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
podman build -t my-docs .
podman run -p 8080:8080 --read-only my-docs
```

**Kubernetes Deployment with Configuration:**
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
        env:
        - name: KISS_PORT
          value: "8080"
        - name: KISS_BIND_IP
          value: "0.0.0.0"
        - name: KISS_MAX_REQUEST_SIZE
          value: "16384"        # 16KB for larger requests
        - name: KISS_CONNECTION_TIMEOUT
          value: "60"           # 60 seconds for slower clients
        - name: KISS_KEEPALIVE_TIMEOUT
          value: "10"           # 10 seconds keep-alive
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

### Configuration

KISS supports flexible configuration via CLI arguments or environment variables with zero runtime overhead:

| Setting | CLI Flag | Environment Variable | Default | Description |
|---------|----------|---------------------|---------|-------------|
| Port | `--port` `-p` | `KISS_PORT` | `8080` | Server bind port |
| Bind IP | `--bind-ip` `-b` | `KISS_BIND_IP` | `"0.0.0.0"` | IP address to bind |
| Static Directory | `--static-dir` `-s` | `KISS_STATIC_DIR` | `"./content"` | Static files directory |
| Max Request Size | `--max-request-size` `-r` | `KISS_MAX_REQUEST_SIZE` | `8192` | Request size limit (bytes) |
| Connection Timeout | `--connection-timeout-secs` `-c` | `KISS_CONNECTION_TIMEOUT` | `30` | Connection timeout (seconds) |
| Keep-alive Timeout | `--keepalive-timeout-secs` `-k` | `KISS_KEEPALIVE_TIMEOUT` | `5` | Keep-alive timeout (seconds) |

**Configuration Examples:**

Using CLI arguments:
```bash
./kiss --port 9000 --bind-ip 127.0.0.1 --max-request-size 16384
```

Using environment variables (preferred for containers):
```bash
export KISS_PORT=9000
export KISS_BIND_IP=127.0.0.1
export KISS_MAX_REQUEST_SIZE=16384
./kiss
```

Docker with environment variables:
```bash
podman run -p 9000:9000 \
  -e KISS_PORT=9000 \
  -e KISS_BIND_IP=0.0.0.0 \
  -e KISS_STATIC_DIR=/content \
  --read-only my-website
```

**Priority Order:** CLI arguments > Environment variables > Default values

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
        image: quay.io/epheo/kiss:latest
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
          mountPath: /content
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
        image: quay.io/epheo/kiss:latest
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
          mountPath: /content
          readOnly: true
      volumes:
      - name: static-files
        configMap:
          name: static-content
```

## Security Features

KISS is designed with security as a primary concern:

### Container Security
- **Scratch Base Image**: Minimal attack surface with no OS packages, shell, or utilities
- **Static Binary**: Single Rust binary (542KB) with no runtime dependencies
- **Ultra-Lightweight Container**: 692KB total container size optimized for security
- **Rootless Operation**: Designed for non-privileged execution on both Kubernetes and OpenShift
- **Read-Only Filesystem**: Compatible with `readOnlyRootFilesystem: true`
- **No Privilege Escalation**: Designed to run with `allowPrivilegeEscalation: false`
- **Minimal Capabilities**: Functions with all Linux capabilities dropped

### Network Security  
- **Non-Privileged Port**: Configurable port (default 8080, >1024) for non-root compatibility
- **Bounded Request Size**: Configurable request size limit (default 8KB) prevents memory exhaustion
- **Connection Limits**: Configurable connection and keep-alive timeouts prevent resource exhaustion
- **Path Sanitization**: Prevents access to server binary and normalizes paths via FNV hashing
- **Binary Protection**: Blocks all access attempts to `/kiss` executable

### Operational Security
- **Graceful Shutdown**: Handles SIGTERM/SIGINT for clean container termination
- **Health Endpoints**: Separate `/health` and `/ready` endpoints for monitoring
- **Async Connection Handling**: Lock-free concurrency prevents resource exhaustion
- **No File Writes**: Server only reads files, never modifies filesystem
- **Configuration Security**: No runtime configuration changes, startup-only parameter extraction

## Performance Architecture

KISS implements an advanced zero-overhead architecture with lock-free caching, pre-generated responses, and micro-optimizations for maximum performance.

### Core Architecture

**Lock-Free File Caching:**
- Atomic RCU pattern with `AtomicPtr<CacheGeneration>` for zero-contention reads
- Complete files preloaded at startup into memory-optimized cache structures
- Cache-line efficient structs (48-byte `CacheEntry` alignment)
- FNV hashing for fast path normalization and lookup

**Zero-Allocation Request Processing:**
- Pre-allocated buffers reused per connection via `.clear()` 
- Single-pass HTTP parsing combining query detection, hashing, and validation
- Direct byte manipulation avoiding UTF-8 overhead in hot paths
- Boyer-Moore-like string matching for header processing

**Response Optimization:**
- **Unified responses**: Headers + content pre-combined for single `write()` syscall
- **Conditional caching**: Pre-generated 304 Not Modified responses
- **Zero-copy sharing**: `Arc<[u8]>` slices for memory efficiency
- **Path trie**: Optimized URL matching with trailing slash handling

### Performance Characteristics

**Micro-Optimizations:**
- Memory layout optimized for L1/L2 cache efficiency
- Hot data fields placed first in structs for better cache locality
- Minimal branching and predictable code paths
- TCP_NODELAY enabled for reduced network latency

**Scalability Features:**
- Async Tokio runtime with per-connection task spawning
- Lock-free concurrent access to file cache
- Bounded request sizes prevent memory exhaustion
- Graceful degradation under load

**Configuration Performance:**
- Zero runtime overhead: CLI/environment variables extracted once at startup
- Direct variable access equivalent to compile-time constants
- No global lookups or indirection in hot paths

### Security Considerations

The file-caching architecture provides certain security characteristics:
- Only files present at startup can be served
- Cache-based lookups help prevent directory traversal attempts
- No dynamic file system access during request handling
- Reduced attack surface through single-purpose design

### Container Design

This architecture aligns with container deployment patterns:
- Suitable for immutable infrastructure where content doesn't change post-deployment
- Memory footprint determined at build time based on static file set
- Single initialization phase during container startup

### Architectural Trade-offs

**Benefits:**
- High performance for static content serving
- Consistent latency without disk I/O variance  
- Well-suited for container environments with static content
- Reduced system call overhead

**Limitations:**
- Memory usage scales with total content size
- Optimized for files under 1MB
- Content is immutable during runtime (requires container restart for changes)
- Startup time correlates with file count and total size

For detailed performance analysis and benchmark results, see `docs/PERFORMANCES.md`.

