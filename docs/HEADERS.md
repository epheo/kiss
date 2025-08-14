# HTTP Headers Implementation

KISS implements comprehensive HTTP caching headers with file metadata caching for optimal performance.

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
  - Security headers (CSP, X-Frame-Options, etc.)

### Performance Benefits
- **Zero filesystem metadata calls** during request handling
- **Pre-compiled headers** eliminate string formatting overhead
- **O(1) cache lookups** via HashMap for instant header retrieval
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

All responses include comprehensive security headers:
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY` 
- `Content-Security-Policy: default-src 'self'; ...`

## Implementation Details

### Cache Structure
```rust
HashMap<String, FileMetadata> {
    "/index.html" -> FileMetadata {
        headers: "HTTP/1.1 200 OK\r\n...",
        size: 1234,
        last_modified: SystemTime,
        etag: "W/\"1234-1445412480\"",
    }
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

Comprehensive test suite covers:
- **Unit Tests**: ETag generation, conditional logic, cache building (`cache_tests.rs`)
- **Integration Tests**: HTTP conditional requests, header validation (`integration_tests.rs`)  
- **Performance Tests**: Cache vs no-cache throughput, 304 response speed (`performance_tests.rs`)
- **Edge Cases**: Malformed headers, concurrent requests, special characters (`edge_case_tests.rs`)

## Configuration

- **Cache Control**: 1 hour max-age (`max-age=3600`)
- **ETag Format**: Weak ETags for compatibility
- **Security Policy**: Restrictive CSP with selective allowances for web assets