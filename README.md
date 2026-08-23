# KISS Instant Static Server

[![Container Repository on Quay](https://quay.io/repository/epheo/kiss/status "Container Repository on Quay")](https://quay.io/repository/epheo/kiss)

A minimalistic static file server written in Rust, designed to run behind a Kubernetes ingress controller.

## Quick Start

Create a `Dockerfile` that extends the KISS base image with your static content:

```dockerfile
FROM quay.io/epheo/kiss:latest
COPY ./my-website/ /content/
```

Build and run:

```bash
podman build -t my-website .
podman run -p 8080:8080 --read-only my-website
```

## Configuration

CLI arguments take priority over environment variables, which take priority over defaults.

| Setting | CLI Flag | Environment Variable | Default | Description |
|---------|----------|---------------------|---------|-------------|
| Port | `--port` `-p` | `KISS_PORT` | `8080` | Server bind port |
| Bind IP | `--bind-ip` `-b` | `KISS_BIND_IP` | `0.0.0.0` | IP address to bind |
| Static Directory | `--static-dir` `-s` | `KISS_STATIC_DIR` | `./content` | Static files directory |
| Max Request Size | `--max-request-size` `-r` | `KISS_MAX_REQUEST_SIZE` | `8192` | Request size limit (bytes) |
| Keep-alive Timeout | `--keepalive-timeout-secs` `-k` | `KISS_KEEPALIVE_TIMEOUT` | `5` | Keep-alive timeout (seconds) |

## Kubernetes Deployment

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
        image: my-website:latest
        ports:
        - containerPort: 8080
        env:
        - name: KISS_PORT
          value: "8080"
        - name: KISS_MAX_REQUEST_SIZE
          value: "16384"
        - name: KISS_KEEPALIVE_TIMEOUT
          value: "10"
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

On OpenShift, remove `runAsUser` and `runAsGroup` from the pod security context — OpenShift assigns arbitrary UIDs with GID 0, which KISS supports out of the box.

## Health Checks

```bash
curl http://localhost:8080/health   # Liveness probe
curl http://localhost:8080/ready    # Readiness probe
```

## Custom 404 Page

A `404.html` at the root of the content directory becomes the 404 response body,
served as `text/html`. Like everything else it is read once at startup and
pre-generated, so misses stay a single `write()`. Without it, KISS answers with
a plain text `File not found`.

## Security

- **Scratch base image** — no shell, no OS packages, minimal attack surface
- **Static musl binary** — single binary with no runtime dependencies
- **Rootless** — runs as non-root on both Kubernetes and OpenShift
- **Read-only filesystem** — compatible with `readOnlyRootFilesystem: true`
- **Path sanitization** — prevents directory traversal and access to the server binary
- **Bounded requests** — configurable request size limit prevents memory exhaustion
- **Graceful shutdown** — handles SIGTERM/SIGINT for clean container termination
