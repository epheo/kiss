# KISS Performance Analysis

This document provides performance analysis and benchmarking data for KISS (Kubernetes Instant Static Server), an in-memory static file server written in Rust.

## Performance Summary

KISS implements an in-memory architecture that pre-loads all static files at startup, eliminating disk I/O during request processing.

Those tests have been performed on my laptop "12th Gen Intel(R) Core(TM) i7-1280P"

**Benchmark Results (Apache Bench):**
- Peak throughput: 216,460 req/s (100,000 requests, 100 concurrent)
- Small file performance: 198,080 req/s vs nginx 157,744 req/s (10,000 requests)
- Medium file performance: 60,930 req/s vs nginx 45,086 req/s  
- Cache performance: 82,924 req/s vs nginx 77,813 req/s
- Response time: 0.462ms mean (100,000 requests), 0.5-1.6ms for files under 1MB
- Architecture: Zero disk I/O with pre-loaded content

## KISS vs nginx Comprehensive Comparison

### File Size Performance Comparison (100 concurrent connections)

| File Type | KISS RPS | nginx RPS | Performance Ratio | KISS Latency | nginx Latency |
|-----------|----------|-----------|-------------------|--------------|---------------|
| Small (12B) | 198,080 | 157,744 | 1.26x | 0.515ms | 0.634ms |
| Medium (100KB) | 60,930 | 45,086 | 1.35x | 1.641ms | 2.218ms |
| Large (10MB) | 386* | 465 | 0.83x | 258ms | 215ms |

*Large files experience performance degradation due to memory constraints

### Concurrency Scaling Performance (small files)

| Concurrency | KISS RPS | nginx RPS | Performance Ratio | 
|-------------|----------|-----------|-------------------|
| 1 | 62,861 | 47,278 | 1.33x |
| 10 | 126,283 | 167,177 | 0.76x |
| 50 | 153,726 | 121,779 | 1.26x |
| 100 | 102,455 | 73,772 | 1.39x |
| 200 | 81,268 | 77,405 | 1.05x |
| 500 | 67,149 | 71,845 | 0.93x |

### Specialized Endpoint Performance

| Endpoint | KISS RPS | nginx RPS | Performance Ratio | Notes |
|----------|----------|-----------|-------------------|-------|
| Cache (304) | 82,924 | 77,813 | 1.07x | Pre-generated 304 responses |
| Health Check | 84,271 | 82,747 | 1.02x | Optimized JSON responses |

## Architecture Comparison

### KISS (In-Memory Architecture)

KISS implements a specialized architecture optimized for static file serving:

- All files are pre-loaded into memory during startup
- Complete HTTP responses are pre-generated (headers + content)
- Zero disk I/O during request processing
- Single write() system call per request
- Trade-off: Higher memory usage for improved performance

### nginx (Disk-Based Architecture)

nginx uses a traditional disk-based approach:

- Files are served from disk using sendfile() system calls
- Only metadata is cached in memory
- Content is read from disk on each request
- Trade-off: Lower memory usage with disk I/O overhead

### Performance Characteristics

KISS demonstrates superior performance for:
- Small to medium files (< 1MB): 1.26-1.35x faster
- Moderate concurrency (50-200 connections): Up to 1.39x faster
- Cache performance: 1.07x faster for 304 responses
- Predictable latency due to elimination of disk I/O variance

nginx maintains advantages for:
- Large files (> 10MB): Better memory efficiency
- Very high concurrency (> 500 connections): More mature connection handling
- Complex routing and dynamic content serving

### Recommended Use Cases

KISS is optimal for:
- Static websites and single-page applications
- API documentation and asset serving
- Microservice static content
- Container-native deployments
- Scenarios where files remain static during runtime

nginx is recommended for:
- General-purpose web serving
- Large file downloads
- Applications requiring dynamic content processing
- Mixed static and dynamic workloads

## Large File Performance Considerations

### Performance Analysis for Large Files (10MB+)

**Observed Behavior:**
- Throughput: 386 RPS vs nginx's 465 RPS (0.83x performance ratio)
- Latency: 258ms vs nginx's 215ms
- Request failures: 8-149 failures per 10,000 requests
- Memory usage: Significant RAM consumption for large file sets

### Recommended File Size Guidelines

| File Size | Performance | Recommendation |
|-----------|-------------|----------------|
| < 1KB | 198,000 RPS | Optimal |
| 1KB - 100KB | 61,000 RPS | Recommended |
| 100KB - 1MB | Estimated 20,000+ RPS | Acceptable |
| 1MB - 10MB | Estimated 5,000 RPS | Consider alternatives |
| > 10MB | 386 RPS with failures | Not recommended |

### Memory Usage Planning

**Memory Usage Estimation:**
```
Total RAM = Sum of all file sizes + ~100MB base overhead

Example calculations:
100 files × 10KB = 1MB RAM
1000 files × 100KB = 100MB RAM  
100 files × 1MB = 100MB RAM
10 files × 10MB = 100MB RAM (with potential performance issues)
```

**Production Guidelines:**
- Target file sizes under 1MB for optimal performance
- Monitor memory usage during startup and operation
- Consider nginx or CDN solutions for large file serving

## Summary

**Performance Characteristics:**
- Peak throughput: 216,460 req/s (100,000 requests benchmark)
- Medium file performance: 60,930 req/s
- Response times: 0.462ms mean (100,000 requests), 0.5-1.6ms for files under 1MB
- Architecture: In-memory serving with zero disk I/O
- Scaling: Linear horizontal scaling in Kubernetes environments
- Optimal use cases: Static websites, documentation, containerized deployments with files under 1MB