# HTTP Headers Implementation

KISS implements comprehensive HTTP caching headers with file-caching architecture for optimal performance.

## File Header Caching

### Startup Cache Building

- **File Discovery**: Recursively scans `STATIC_DIR` at server startup
- **Metadata Collection**: Captures file size, modification time, and MIME type
- **Header Pre-compilation**: Generates complete HTTP response headers including:
  - `Content-Type` with charset
  - `Content-Length`
  - `ETag` (weak format: `W/"size-mtime"`)
  - `Last-Modified` (timestamp format)
  - `Cache-Control: public, max-age=3600`
  - Security headers (X-Content-Type-Options)

### Performance Benefits

- **Zero filesystem metadata calls** during request handling
- **Pre-compiled headers** eliminate string formatting overhead
- **O(1) cache lookups**: one hash map lookup per request
- **304 Not Modified** support reduces bandwidth usage

## Conditional Request Support

### If-None-Match (ETag)

- Supports weak ETag comparison (`W/"123-456"`)
- Handles multiple ETags in comma-separated list
- Wildcard support (`*` matches any resource)
- Takes precedence over If-Modified-Since

### If-Modified-Since

- Timestamp-based cache validation
- Returns 304 if file unchanged since client timestamp
- Fallback when no ETag provided

### Response Codes

- **200 OK**: File content with full headers
- **304 Not Modified**: Cached validation successful
- **404 Not Found**: File not in cache/doesn't exist

## Security Headers

All responses include security headers:

- `X-Content-Type-Options: nosniff`

## Implementation Details

### Cache Structure

```rust
Server {
    files: HashMap<String, Arc<File>>, // URL path -> file; "/dir" and "/dir/index.html" share one File
    health: Prebuilt,
    ready: Prebuilt,
    not_found: Prebuilt,               // From 404.html when present
    ...
}

File {
    response: Prebuilt,         // Complete 200 response: headers + content
    not_modified: Box<[u8]>,    // Complete 304 response
    content_type: &'static str, // Repeated on 206 responses, built per request
    etag: String,
    last_modified: String,      // Compared byte-for-byte before parsing If-Modified-Since
    modified: SystemTime,
}

Prebuilt {
    bytes: Box<[u8]>,  // HEAD sends bytes[..header_len], GET sends all of it
    header_len: usize,
}
```

### ETag Generation

- **Format**: `W/"<filesize>-<mtime_seconds>"`
- **Type**: Weak ETags for efficient cache validation
- **Uniqueness**: Size + modification time ensures uniqueness

### Cache Consistency

- Files never change during server runtime (immutable deployment model)
- Cache built once at startup for maximum performance
- Missing files return 404 (cache miss = file doesn't exist)

## Testing

`tests/server.rs` checks these headers end to end against the real binary,
including conditional requests, `HEAD`, and range requests. See [TESTS.md](TESTS.md).

## Configuration

- **Cache Control**: 1 hour max-age (`max-age=3600`)
- **ETag Format**: Weak ETags for compatibility
- **Security Policy**: Content-Type protection via X-Content-Type-Options header
