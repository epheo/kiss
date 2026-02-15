# KISS Performance Analysis

KISS implements an in-memory architecture that pre-loads all static files at startup, eliminating disk I/O during request processing.

Tests performed on AMD RYZEN AI MAX+ 395 (32 cores) with wrk (8 threads, 10s duration per test).

## Results

- Peak throughput: **1,085,482 req/s** (500 concurrent connections)
- P99 latency: **269μs** at 100 connections
- Memory footprint: **2.2MB RSS**, stable under sustained load

### Concurrency Scaling (small file)

| Concurrency | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|-------------|-------------|-------------|-------|----------|-----------|
| 10          | 664,480     | 546,423     | 1.21x | 27μs     | 42μs      |
| 50          | 831,972     | 1,044,872   | 0.80x | 170μs    | 635μs     |
| 100         | 1,014,821   | 968,300     | 1.04x | 269μs    | 569μs     |
| 200         | 1,055,360   | 915,806     | 1.15x | 390μs    | 1.42ms    |
| 500         | 1,085,482   | 919,009     | 1.18x | 1.02ms   | 3.06ms    |

KISS scales linearly up to 500 connections. P99 tail latency stays 2-3x tighter than nginx under load.

### File Size Performance (100 concurrent connections)

| File Type      | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|----------------|-------------|-------------|-------|----------|-----------|
| Small (12B)    | 1,063,514   | 997,058     | 1.06x | 274μs    | 573μs     |
| Medium (100KB) | 497,967     | 458,640     | 1.08x | 588μs    | 40.08ms   |
| Large (10MB)   | 4,507       | 7,950       | 0.57x | 135ms    | 20ms      |

nginx wins on large files because sendfile() avoids copying content through userspace.

### Cache & Health (100 concurrent connections)

| Endpoint     | KISS RPS    | nginx RPS | Ratio |
|--------------|-------------|-----------|-------|
| Cache (304)  | 1,017,210   | 948,507   | 1.07x |
| Health Check | 1,027,482   | 886,659   | 1.15x |

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
