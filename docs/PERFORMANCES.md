# KISS Performance Analysis

KISS implements an in-memory architecture that pre-loads all static files at startup, eliminating disk I/O during request processing.

Tests performed on AMD RYZEN AI MAX+ 395 (32 cores) with wrk (8 threads, 10s duration per test).

## Results

- Peak throughput: **1,142,853 req/s** (500 concurrent connections)
- P99 latency: **284μs** at 100 connections
- Memory footprint: **2.2MB RSS**, stable under sustained load

### Concurrency Scaling (small file)

| Concurrency | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|-------------|-------------|-------------|-------|----------|-----------|
| 10          | 742,572     | 559,918     | 1.32x | 24μs     | 35μs      |
| 50          | 881,120     | 1,030,593   | 0.85x | 181μs    | 626μs     |
| 100         | 1,055,977   | 946,983     | 1.11x | 284μs    | 573μs     |
| 200         | 1,106,015   | 906,544     | 1.22x | 380μs    | 1.46ms    |
| 500         | 1,142,853   | 917,053     | 1.24x | 1.05ms   | 3.12ms    |

KISS scales linearly up to 500 connections. P99 tail latency stays 2-3x tighter than nginx under load.

### File Size Performance (100 concurrent connections)

| File Type      | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|----------------|-------------|-------------|-------|----------|-----------|
| Small (12B)    | 1,022,786   | 966,781     | 1.05x | 301μs    | 556μs     |
| Medium (100KB) | 516,632     | 454,941     | 1.13x | 534μs    | 40.09ms   |
| Large (10MB)   | 4,557       | 7,681       | 0.59x | 132ms    | 21ms      |

nginx wins on large files because sendfile() avoids copying content through userspace.

### Cache & Health (100 concurrent connections)

| Endpoint     | KISS RPS    | nginx RPS | Ratio |
|--------------|-------------|-----------|-------|
| Cache (304)  | 1,053,972   | 952,569   | 1.10x |
| Health Check | 1,085,301   | 853,252   | 1.27x |

## Architecture

### KISS (In-Memory)

- All files pre-loaded into memory at startup
- Complete HTTP responses pre-generated (headers + content)
- Zero disk I/O during request processing
- Single write() system call per response

### nginx (Disk-Based)

- Files served from disk using sendfile()
- Metadata cached in memory, content read per request

### When to use KISS

- Static websites and single-page applications
- API documentation and asset serving
- Microservice static content in Kubernetes
- Files under 1MB where latency predictability matters

### When to use nginx

- Large file downloads (> 10MB)
- Dynamic content processing
- Mixed static and dynamic workloads

## Memory Usage

```txt
Total RAM = Sum of all file sizes + ~2MB base overhead

100 files × 10KB  = 1MB RAM
1000 files × 100KB = 100MB RAM
100 files × 1MB   = 100MB RAM
```

Target file sizes under 1MB for optimal performance. Consider nginx or CDN for large file serving.
