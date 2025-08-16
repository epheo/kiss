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
- **O(1) cache lookups** via FxHashMap with hash-based path resolution for instant header retrieval
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
PathTrie {
    exact_matches: FxHashMap<u32, CacheEntry>,
    index_entries: FxHashMap<u32, CacheEntry>,
}

CacheEntry {
    complete_response: Arc<[u8]>,     // Headers + content combined
    headers_only: Arc<[u8]>,          // Headers for HEAD requests
    not_modified_response: Arc<[u8]>, // Pre-generated 304 response
    last_modified_timestamp: SystemTime,
    etag: Arc<str>,
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
- **Security Policy**: Content-Type protection via X-Content-Type-Options header