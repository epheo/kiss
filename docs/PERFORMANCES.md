# KISS Performance Analysis

KISS implements an in-memory architecture that pre-loads all static files at startup, eliminating disk I/O during request processing.

All numbers come from one AMD RYZEN AI MAX+ 395 (16 cores, 32 threads), with the load generator and KISS on the same host over loopback.

## Results

- Peak throughput: **1,418,685 req/s** at 500 connections, using 5 cores
- P99 latency: **192us** at 100 connections
- Memory footprint: **1.3MB peak RSS** at 500 connections
- One core: **217,576 req/s**

Musl build as shipped in the image, 12-byte file, 8 load-generator threads, 10 s runs.
Measured with a closed-loop keep-alive load generator, not wrk, so compare these tables with each other rather than with the nginx comparison below.
The baseline is the previous build, commit `e2e32a9`, measured in the same session.

### Whole machine (median of 3)

| Connections | Build    | req/s     | P50   | P99     | Server CPU | Peak RSS |
|-------------|----------|-----------|-------|---------|------------|----------|
| 100         | current  | 1,009,266 | 96us  | 192us   | 3.6 cores  | 1.1MB    |
| 100         | baseline | 858,235   | 107us | 357us   | 5.7 cores  | 1.6MB    |
| 500         | current  | 1,418,685 | 318us | 822us   | 5.0 cores  | 1.3MB    |
| 500         | baseline | 1,070,147 | 430us | 1,058us | 15.1 cores | 5.1MB    |

Server CPU is the KISS process's user and system time; the kernel's network processing on its behalf comes on top.

### Per core (100 connections, median of 2)

KISS pinned to 1, 2 or 4 cores, so tokio runs one worker per core, with the load generator on 8 other cores.

| Server cores | req/s   | P50   | P99   |
|--------------|---------|-------|-------|
| 1            | 217,576 | 468us | 514us |
| 2            | 393,224 | 246us | 476us |
| 4            | 738,353 | 129us | 458us |

The baseline is within 1% at these core counts: the whole-machine gain only appears with many workers.
This table is a rough guide for Kubernetes CPU limits.

## Comparison with nginx

Measured with wrk (8 threads, 10 s per test) on an earlier build, before the changes measured above.

### Concurrency Scaling (small file)

| Concurrency | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|-------------|-------------|-------------|-------|----------|-----------|
| 10          | 742,572     | 559,918     | 1.32x | 24us     | 35us      |
| 50          | 881,120     | 1,030,593   | 0.85x | 181us    | 626us     |
| 100         | 1,055,977   | 946,983     | 1.11x | 284us    | 573us     |
| 200         | 1,106,015   | 906,544     | 1.22x | 380us    | 1.46ms    |
| 500         | 1,142,853   | 917,053     | 1.24x | 1.05ms   | 3.12ms    |

KISS scales linearly up to 500 connections. P99 tail latency stays 2-3x tighter than nginx under load.

### File Size Performance (100 concurrent connections)

| File Type      | KISS RPS    | nginx RPS   | Ratio | KISS P99 | nginx P99 |
|----------------|-------------|-------------|-------|----------|-----------|
| Small (12B)    | 1,022,786   | 966,781     | 1.05x | 301us    | 556us     |
| Medium (100KB) | 516,632     | 454,941     | 1.13x | 534us    | 40.09ms   |
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
- One read() and no heap allocation per request, so most CPU time is the kernel's TCP stack

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
Total RAM = Sum of all file sizes + ~1.5MB base overhead

100 files x 10KB  = 1MB RAM
1000 files x 100KB = 100MB RAM
100 files x 1MB   = 100MB RAM
```

Target file sizes under 1MB for optimal performance. Consider nginx or CDN for large file serving.
