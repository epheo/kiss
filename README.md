# KISS Instant Static Server

KISS (Kubernetes Instant Static Server) is an in-memory static file server written in Rust, designed as a minimalistic base image for Kubernetes deployments.

## Overview

KISS implements an in-memory architecture that pre-loads static files at startup, eliminating disk I/O during request processing. This approach is designed for static websites, single-page applications, and documentation in containerized environments.

**Characteristics:**
- In-memory file serving with zero disk I/O during requests
- Single-purpose design focused on static content delivery
- Container-optimized for Kubernetes deployments
- Minimal dependencies and attack surface

KISS serves files from the container root directory (`/`) while protecting the server binary at `/kiss`.


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

## Security Features

KISS is designed with security as a primary concern:

### Container Security
- **Scratch Base Image**: Minimal attack surface with no OS packages, shell, or utilities
- **Static Binary**: Single Rust binary (562KB) with no runtime dependencies
- **Ultra-Lightweight Container**: 706KB total container size
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

## Performance Architecture

KISS implements an in-memory architecture that pre-loads all static files and pre-generates complete HTTP responses at startup, eliminating disk I/O during request processing.

### Architecture Overview

At startup, KISS scans the static directory and builds an in-memory cache containing:
- **Complete HTTP responses**: Pre-generated headers and content combined for single-write operations
- **File content**: All files loaded entirely into memory
- **Conditional responses**: Pre-computed 304 Not Modified responses
- **Path variations**: Common URL patterns pre-computed to eliminate string operations

### Performance Characteristics

**Performance Characteristics:**
- Small files (< 1KB): 198,000 requests/second
- Medium files (100KB): 61,000 requests/second  
- Response times: 0.5-1.6ms for files under 1MB
- Zero disk I/O during request serving

**Implementation Details:**
- Single write() system call per request
- Pre-computed HTTP responses stored in memory
- HashMap-based file lookups
- Rust's zero-cost abstractions for performance

### Security Considerations

The in-memory architecture provides certain security characteristics:
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
- Performance optimized for files under 1MB
- Content is immutable during runtime (requires container restart for changes)
- Startup time correlates with file count and total size

For detailed performance analysis, see `docs/PERFORMANCES.md`.

