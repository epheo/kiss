# Testing

```bash
cargo test
```

That runs everything in about two seconds. No server needs to be running and
no test is `#[ignore]`d.

## Layout

- **Unit tests** (in `src/`): MIME type mapping and `Range` header parsing,
  next to the code they test.
- **End-to-end tests** (`tests/server.rs`): each test starts the real `kiss`
  binary (Cargo builds it and exposes its path as `CARGO_BIN_EXE_kiss`) on a
  free port, with its own temporary content directory, configured through
  `KISS_*` environment variables as in Kubernetes, and talks raw HTTP to it. Responses are read by `Content-Length`, so keep-alive connections never
  stall a test.

## What the end-to-end tests cover

- **Serving**: bodies, `Content-Type`, `Content-Length`, `ETag`,
  `Last-Modified`, `Cache-Control`, `Accept-Ranges` and `X-Content-Type-Options`;
  directory indexes with and without a trailing slash; query strings;
  absolute-form request targets; health and readiness probes.
- **Conditional requests**: `If-None-Match` (weak comparison, lists, `*`, any
  header case), `If-Modified-Since` (exact, older, newer, malformed), and
  `If-None-Match` taking precedence.
- **Ranges**: prefix, suffix and open ranges, clamping, 416, and multi-range or
  unknown units falling back to 200; `HEAD` with `Range`.
- **Errors**: 404 (including traversal attempts and a custom `404.html`), 405
  with `Allow`, 400 for malformed request lines, and 413 for oversized requests,
  including headers that never terminate.
- **Connections**: keep-alive, pipelining, `HEAD` never leaking a body,
  `Connection: close`, HTTP/1.0 with and without keep-alive, idle and
  incomplete-request timeouts, the keep-alive window restarting after each
  response, slowly sent requests, and concurrent clients.
- **Content directory**: symlinks (to files and directories) and files over the
  50 MB cache limit are never served.
- **Lifecycle**: graceful shutdown on `SIGTERM` and `SIGINT`, requests on open
  connections still answered during shutdown, and a clear failure when the port
  is taken.
- **Configuration**: `KISS_*` environment variables, the version in the
  startup log, and exit code 2 for invalid values or any argument.

## Benchmarks

`benchmark.sh` compares a running KISS against nginx with
[wrk](https://github.com/wg/wrk). Point both servers at `tests/content/`, which
holds the benchmark fixtures (`small.txt`, `medium.txt`, `large.txt`).
Results are in [PERFORMANCES.md](PERFORMANCES.md).
