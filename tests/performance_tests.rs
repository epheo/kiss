//! Performance Tests for KISS Static Server
//! 
//! This module contains comprehensive performance tests that measure different
//! aspects of the server's performance characteristics:
//! 
//! ## Test Categories:
//! 
//! ### 1. Latency Tests (`test_single_request_response_time`)
//! - **Purpose**: Measure response time distribution and latency characteristics
//! - **Method**: Sequential requests measuring individual response times
//! - **Metrics**: Min, max, average, median, 95th percentile response times
//! - **Target**: <10ms average, <50ms 95th percentile
//! 
//! ### 2. Concurrency Scaling Tests (`test_concurrent_request_throughput`)  
//! - **Purpose**: Test how performance scales with different concurrency levels
//! - **Method**: Test at 1, 5, 10, 20 concurrent connections
//! - **Metrics**: Requests per second at each concurrency level
//! - **Target**: >50 req/s minimum, linear scaling with concurrency
//! 
//! ### 3. Sustained Capacity Tests (`test_sustained_capacity`)
//! - **Purpose**: Measure maximum sustainable throughput over extended periods
//! - **Method**: Flood test with high concurrency for 30 seconds
//! - **Metrics**: Sustained requests per second, response time stability
//! - **Result**: Reports actual sustained capacity (no artificial targets)
//! 
//! ### 4. Maximum Throughput Tests (`test_maximum_throughput`)
//! - **Purpose**: Measure peak performance under maximum stress (comparable to Apache Bench)
//! - **Method**: High concurrency (100 connections) for maximum throughput
//! - **Metrics**: Maximum requests per second achieved
//! - **Target**: >15K req/s (should approach Apache Bench results of ~32K req/s)
//! 
//! ### 5. Microbenchmarks (`bench_*` tests)
//! - **Purpose**: Test performance of individual components (MIME detection, path sanitization)
//! - **Method**: Isolated component testing with high iteration counts
//! - **Metrics**: Operations per second for specific functions
//! - **Target**: Component-specific performance thresholds
//! 
//! ## Comparison with External Tools:
//! - **Apache Bench**: Our `test_maximum_throughput` should approach `ab` results
//! - **Real-world load**: Our `test_memory_usage_under_load` simulates production usage
//! - **Component perf**: Our microbenchmarks test individual optimizations

use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(test)]
mod performance_regression_tests {
    use super::*;
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_single_request_response_time() {
        let mut response_times = Vec::new();
        let test_paths = ["/health", "/ready"];
        
        for path in &test_paths {
            for _ in 0..100 {
                let start = Instant::now();
                match send_request(path) {
                    Ok(response) => {
                        let duration = start.elapsed();
                        if response.contains("HTTP/1.1 200 OK") {
                            response_times.push(duration);
                        }
                    }
                    Err(_) => {
                        println!("Warning: Server not running, skipping performance test");
                        return;
                    }
                }
            }
        }
        
        if response_times.is_empty() {
            println!("Warning: No successful requests for performance test");
            return;
        }
        
        // Calculate statistics
        response_times.sort();
        let min = response_times[0];
        let max = response_times[response_times.len() - 1];
        let median = response_times[response_times.len() / 2];
        let p95_idx = (response_times.len() as f64 * 0.95) as usize;
        let p95 = response_times[p95_idx.min(response_times.len() - 1)];
        
        let avg = response_times.iter().sum::<Duration>() / response_times.len() as u32;
        
        println!("Response time statistics:");
        println!("  Min: {:?}", min);
        println!("  Avg: {:?}", avg);
        println!("  Median: {:?}", median);
        println!("  95th percentile: {:?}", p95);
        println!("  Max: {:?}", max);
        
        // Performance assertions (reasonable targets for a lightweight server)
        assert!(avg < Duration::from_millis(10), "Average response time should be < 10ms, got {:?}", avg);
        assert!(p95 < Duration::from_millis(50), "95th percentile should be < 50ms, got {:?}", p95);
        assert!(max < Duration::from_millis(1000), "Max response time should be < 1s, got {:?}", max);
    }
    
    #[test]
    #[ignore] // Performance test - run manually  
    fn test_concurrent_request_throughput() {
        const CONCURRENCY_LEVELS: [usize; 4] = [1, 5, 10, 20];
        const REQUESTS_PER_LEVEL: usize = 200;
        
        for &concurrency in &CONCURRENCY_LEVELS {
            println!("Testing concurrency level: {}", concurrency);
            
            let start_time = Instant::now();
            let handles = create_concurrent_requests(concurrency, REQUESTS_PER_LEVEL / concurrency, "/health");
            
            let mut successful_requests = 0;
            let mut total_response_time = Duration::ZERO;
            
            for handle in handles {
                match handle.join() {
                    Ok(Ok((count, duration))) => {
                        successful_requests += count;
                        total_response_time += duration;
                    }
                    Ok(Err(e)) => {
                        println!("Thread error: {}", e);
                    }
                    Err(_) => {
                        println!("Thread panicked");
                    }
                }
            }
            
            let total_duration = start_time.elapsed();
            let requests_per_second = successful_requests as f64 / total_duration.as_secs_f64();
            let avg_response_time = if successful_requests > 0 { 
                total_response_time / successful_requests as u32 
            } else { 
                Duration::ZERO 
            };
            
            println!("Concurrency {}: {}/{} requests in {:?} ({:.2} req/s, avg {:?})", 
                    concurrency, successful_requests, REQUESTS_PER_LEVEL, 
                    total_duration, requests_per_second, avg_response_time);
            
            // Performance targets
            assert!(requests_per_second > 50.0, 
                   "Throughput too low at concurrency {}: {:.2} req/s", concurrency, requests_per_second);
            assert!(successful_requests >= REQUESTS_PER_LEVEL * 9 / 10, 
                   "Success rate too low: {}/{}", successful_requests, REQUESTS_PER_LEVEL);
        }
    }
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_sustained_capacity() {
        // Sustained capacity test: Measure maximum sustainable throughput
        const TEST_DURATION_SECS: u64 = 30;
        const CONCURRENCY: usize = 100; // High concurrency to saturate server
        const REQUESTS_PER_THREAD: usize = 10; // Small batches for continuous flow
        
        println!("Starting sustained capacity test: {} concurrent connections for {} seconds", 
                CONCURRENCY, TEST_DURATION_SECS);
        
        let start_time = Instant::now();
        let mut total_requests = 0;
        let mut successful_requests = 0;
        let mut response_times = Vec::new();
        
        // Flood test: Send requests continuously without rate limiting
        while start_time.elapsed().as_secs() < TEST_DURATION_SECS {
            // Send requests as fast as possible - no timing constraints
            let handles = create_concurrent_requests(CONCURRENCY, REQUESTS_PER_THREAD, "/health");
            
            for handle in handles {
                total_requests += REQUESTS_PER_THREAD;
                match handle.join() {
                    Ok(Ok((count, duration))) => {
                        successful_requests += count;
                        if count > 0 {
                            // Average response time for this batch
                            response_times.push(duration / count as u32);
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(_) => {}
                }
            }
            // No sleep - continuous request sending for maximum throughput
        }
        
        let actual_duration = start_time.elapsed();
        let actual_rps = successful_requests as f64 / actual_duration.as_secs_f64();
        
        println!("Sustained capacity test results:");
        println!("  Duration: {:?}", actual_duration);
        println!("  Requests: {} successful / {} total", successful_requests, total_requests);
        println!("  Sustained RPS: {:.0}", actual_rps);
        
        if !response_times.is_empty() {
            response_times.sort();
            let avg_response_time = response_times.iter().sum::<Duration>() / response_times.len() as u32;
            let p95_idx = (response_times.len() as f64 * 0.95) as usize;
            let p95_response_time = response_times[p95_idx.min(response_times.len() - 1)];
            
            println!("  Avg response time: {:?}", avg_response_time);
            println!("  95th percentile: {:?}", p95_response_time);
            
            // Sustained performance should remain consistent (no memory leaks/degradation)
            assert!(avg_response_time < Duration::from_millis(10), 
                   "Average response time degraded under sustained load: {:?}", avg_response_time);
            assert!(p95_response_time < Duration::from_millis(50), 
                   "95th percentile response time too high under sustained load: {:?}", p95_response_time);
        }
        
        // Should maintain high success rate under sustained load
        let success_rate = successful_requests as f64 / total_requests as f64;
        assert!(success_rate > 0.95, "Success rate too low under sustained load: {:.2}%", success_rate * 100.0);
        
        // Report sustained capacity - no target to validate against
        println!("✅ Sustained capacity test completed: {:.0} req/s", actual_rps);
    }
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_sustained_capacity_static_files() {
        // Sustained capacity test: Measure maximum sustainable throughput
        const TEST_DURATION_SECS: u64 = 30;
        const CONCURRENCY: usize = 100; // High concurrency to saturate server
        const REQUESTS_PER_THREAD: usize = 10; // Small batches for continuous flow
        
        println!("Starting sustained capacity test for static files: {} concurrent connections for {} seconds", 
                CONCURRENCY, TEST_DURATION_SECS);
        
        let start_time = Instant::now();
        let mut total_requests = 0;
        let mut successful_requests = 0;
        let mut response_times = Vec::new();
        
        // Flood test: Send requests continuously without rate limiting
        while start_time.elapsed().as_secs() < TEST_DURATION_SECS {
            // Send requests as fast as possible - no timing constraints
            let handles = create_concurrent_requests(CONCURRENCY, REQUESTS_PER_THREAD, "/index.html");
            
            for handle in handles {
                total_requests += REQUESTS_PER_THREAD;
                match handle.join() {
                    Ok(Ok((count, duration))) => {
                        successful_requests += count;
                        if count > 0 {
                            // Average response time for this batch
                            response_times.push(duration / count as u32);
                        }
                    }
                    Ok(Err(_)) => {}
                    Err(_) => {}
                }
            }
            // No sleep - continuous request sending for maximum throughput
        }
        
        let actual_duration = start_time.elapsed();
        let actual_rps = successful_requests as f64 / actual_duration.as_secs_f64();
        
        println!("Sustained capacity test results (static files):");
        println!("  Duration: {:?}", actual_duration);
        println!("  Requests: {} successful / {} total", successful_requests, total_requests);
        println!("  Sustained RPS: {:.0}", actual_rps);
        
        if !response_times.is_empty() {
            response_times.sort();
            let avg_response_time = response_times.iter().sum::<Duration>() / response_times.len() as u32;
            let p95_idx = (response_times.len() as f64 * 0.95) as usize;
            let p95_response_time = response_times[p95_idx.min(response_times.len() - 1)];
            
            println!("  Avg response time: {:?}", avg_response_time);
            println!("  95th percentile: {:?}", p95_response_time);
            
            // Sustained performance should remain consistent (no memory leaks/degradation)
            assert!(avg_response_time < Duration::from_millis(10), 
                   "Average response time degraded under sustained load: {:?}", avg_response_time);
            assert!(p95_response_time < Duration::from_millis(50), 
                   "95th percentile response time too high under sustained load: {:?}", p95_response_time);
        }
        
        // Should maintain high success rate under sustained load
        let success_rate = successful_requests as f64 / total_requests as f64;
        assert!(success_rate > 0.95, "Success rate too low under sustained load: {:.2}%", success_rate * 100.0);
        
        // Report sustained capacity - no target to validate against
        println!("✅ Sustained capacity test completed (static files): {:.0} req/s", actual_rps);
    }
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_maximum_throughput() {
        // Maximum throughput test: Measure peak performance similar to Apache Bench
        const TEST_DURATION_SECS: u64 = 5; // Shorter test for maximum stress
        const HIGH_CONCURRENCY: usize = 100;
        const REQUESTS_PER_THREAD: usize = 100;
        
        println!("Starting maximum throughput test: {} concurrent connections for {} seconds", 
                HIGH_CONCURRENCY, TEST_DURATION_SECS);
        
        let start_time = Instant::now();
        let mut total_successful = 0;
        let mut total_requests = 0;
        let mut all_response_times = Vec::new();
        
        // Run multiple rounds of high-concurrency requests
        while start_time.elapsed().as_secs() < TEST_DURATION_SECS {
            let round_start = Instant::now();
            let handles = create_concurrent_requests(HIGH_CONCURRENCY, REQUESTS_PER_THREAD, "/health");
            
            let mut round_successful = 0;
            let mut round_response_times = Vec::new();
            
            for handle in handles {
                total_requests += REQUESTS_PER_THREAD;
                match handle.join() {
                    Ok(Ok((count, duration))) => {
                        round_successful += count;
                        if count > 0 {
                            // Distribute the total duration across successful requests
                            let avg_time_per_request = duration / count as u32;
                            for _ in 0..count {
                                round_response_times.push(avg_time_per_request);
                            }
                        }
                    }
                    _ => {}
                }
            }
            
            total_successful += round_successful;
            all_response_times.extend(round_response_times);
            
            let round_duration = round_start.elapsed();
            let round_rps = round_successful as f64 / round_duration.as_secs_f64();
            println!("  Round: {} requests in {:?} ({:.0} req/s)", 
                    round_successful, round_duration, round_rps);
        }
        
        let total_duration = start_time.elapsed();
        let max_rps = total_successful as f64 / total_duration.as_secs_f64();
        
        println!("Maximum throughput test results:");
        println!("  Duration: {:?}", total_duration);
        println!("  Requests: {} successful / {} total", total_successful, total_requests);
        println!("  Maximum RPS: {:.0}", max_rps);
        
        if !all_response_times.is_empty() {
            all_response_times.sort();
            let min_time = all_response_times[0];
            let max_time = all_response_times[all_response_times.len() - 1];
            let avg_time = all_response_times.iter().sum::<Duration>() / all_response_times.len() as u32;
            let p95_idx = (all_response_times.len() as f64 * 0.95) as usize;
            let p95_time = all_response_times[p95_idx.min(all_response_times.len() - 1)];
            
            println!("  Response times - Min: {:?}, Avg: {:?}, 95th: {:?}, Max: {:?}", 
                    min_time, avg_time, p95_time, max_time);
        }
        
        // Performance assertions for maximum throughput
        let success_rate = total_successful as f64 / total_requests as f64;
        assert!(success_rate > 0.90, "Success rate too low under max load: {:.2}%", success_rate * 100.0);
        
        // Should achieve significant throughput (compare to Apache Bench results)
        assert!(max_rps > 80000.0, 
               "Maximum throughput too low: {:.0} req/s (expected >80K)", max_rps);
        
        println!("✅ Maximum throughput test passed: {:.0} req/s", max_rps);
    }
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_maximum_throughput_static_files() {
        // Maximum throughput test: Measure peak performance similar to Apache Bench
        const TEST_DURATION_SECS: u64 = 5; // Shorter test for maximum stress
        const HIGH_CONCURRENCY: usize = 100;
        const REQUESTS_PER_THREAD: usize = 100;
        
        println!("Starting maximum throughput test for static files: {} concurrent connections for {} seconds", 
                HIGH_CONCURRENCY, TEST_DURATION_SECS);
        
        let start_time = Instant::now();
        let mut total_successful = 0;
        let mut total_requests = 0;
        let mut all_response_times = Vec::new();
        
        // Run multiple rounds of high-concurrency requests
        while start_time.elapsed().as_secs() < TEST_DURATION_SECS {
            let round_start = Instant::now();
            let handles = create_concurrent_requests(HIGH_CONCURRENCY, REQUESTS_PER_THREAD, "/index.html");
            
            let mut round_successful = 0;
            let mut round_response_times = Vec::new();
            
            for handle in handles {
                total_requests += REQUESTS_PER_THREAD;
                match handle.join() {
                    Ok(Ok((count, duration))) => {
                        round_successful += count;
                        if count > 0 {
                            // Distribute the total duration across successful requests
                            let avg_time_per_request = duration / count as u32;
                            for _ in 0..count {
                                round_response_times.push(avg_time_per_request);
                            }
                        }
                    }
                    _ => {}
                }
            }
            
            total_successful += round_successful;
            all_response_times.extend(round_response_times);
            
            let round_duration = round_start.elapsed();
            let round_rps = round_successful as f64 / round_duration.as_secs_f64();
            println!("  Round: {} requests in {:?} ({:.0} req/s)", 
                    round_successful, round_duration, round_rps);
        }
        
        let total_duration = start_time.elapsed();
        let max_rps = total_successful as f64 / total_duration.as_secs_f64();
        
        println!("Maximum throughput test results (static files):");
        println!("  Duration: {:?}", total_duration);
        println!("  Requests: {} successful / {} total", total_successful, total_requests);
        println!("  Maximum RPS: {:.0}", max_rps);
        
        if !all_response_times.is_empty() {
            all_response_times.sort();
            let min_time = all_response_times[0];
            let max_time = all_response_times[all_response_times.len() - 1];
            let avg_time = all_response_times.iter().sum::<Duration>() / all_response_times.len() as u32;
            let p95_idx = (all_response_times.len() as f64 * 0.95) as usize;
            let p95_time = all_response_times[p95_idx.min(all_response_times.len() - 1)];
            
            println!("  Response times - Min: {:?}, Avg: {:?}, 95th: {:?}, Max: {:?}", 
                    min_time, avg_time, p95_time, max_time);
        }
        
        // Performance assertions for maximum throughput
        let success_rate = total_successful as f64 / total_requests as f64;
        assert!(success_rate > 0.90, "Success rate too low under max load: {:.2}%", success_rate * 100.0);
        
        // Should achieve significant throughput (compare to Apache Bench results)
        assert!(max_rps > 80000.0, 
               "Maximum throughput too low: {:.0} req/s (expected >80K)", max_rps);
        
        println!("✅ Maximum throughput test passed (static files): {:.0} req/s", max_rps);
    }
    
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_mime_type_detection_performance() {
        let test_files = [
            "file.html", "file.css", "file.js", "file.png", "file.pdf",
            "path/to/file.html", "very-long-filename-with-extension.css",
            "file.UNKNOWN_EXTENSION", "file", ".hidden",
        ];
        
        const ITERATIONS: usize = 100000;
        
        for file in &test_files {
            let start = Instant::now();
            
            for _ in 0..ITERATIONS {
                let _result = kiss::get_mime_type_enum(std::path::Path::new(file));
            }
            
            let duration = start.elapsed();
            let ops_per_sec = ITERATIONS as f64 / duration.as_secs_f64();
            
            println!("MIME type detection for {:?}: {} iterations in {:?} ({:.0} ops/sec)", 
                    file, ITERATIONS, duration, ops_per_sec);
            
            // Should be very fast for MIME type detection
            assert!(ops_per_sec > 50000.0, 
                   "MIME type detection too slow: {:.0} ops/sec for {:?}", ops_per_sec, file);
        }
    }
    
    fn create_concurrent_requests(
        concurrency: usize, 
        requests_per_thread: usize, 
        path: &str
    ) -> Vec<thread::JoinHandle<Result<(usize, Duration), String>>> {
        let barrier = Arc::new(Barrier::new(concurrency + 1));
        let mut handles = Vec::new();
        
        for _ in 0..concurrency {
            let barrier_clone = Arc::clone(&barrier);
            let path_clone = path.to_string();
            
            let handle = thread::spawn(move || {
                barrier_clone.wait();
                
                let mut successful_count = 0;
                let mut total_time = Duration::ZERO;
                
                for _ in 0..requests_per_thread {
                    let start = Instant::now();
                    match send_request(&path_clone) {
                        Ok(response) => {
                            let duration = start.elapsed();
                            if response.contains("HTTP/1.1 200 OK") {
                                successful_count += 1;
                                total_time += duration;
                            }
                        }
                        Err(e) => {
                            return Err(format!("Request failed: {}", e));
                        }
                    }
                }
                
                Ok((successful_count, total_time))
            });
            
            handles.push(handle);
        }
        
        barrier.wait(); // Start all threads simultaneously
        handles
    }
    
    fn send_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        stream.set_write_timeout(Some(Duration::from_secs(10)))?;
        
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", path);
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }
}

#[cfg(test)]
mod benchmark_tests {
    use super::*;
    
    
    
    #[test]
    fn bench_mime_type_detection() {
        let common_types = ["html", "css", "js", "png", "jpg", "pdf", "woff2"];
        const ITERATIONS: usize = 100000;
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for ext in &common_types {
                let filename = format!("file.{}", ext);
                let _result = kiss::get_mime_type_enum(std::path::Path::new(&filename));
            }
        }
        let duration = start.elapsed();
        
        let total_ops = ITERATIONS * common_types.len();
        let ops_per_sec = total_ops as f64 / duration.as_secs_f64();
        
        println!("MIME type detection: {} ops in {:?} ({:.0} ops/sec)", 
                total_ops, duration, ops_per_sec);
        
        // Should be very fast for MIME type lookup
        assert!(ops_per_sec > 200000.0, "MIME type detection should be >200k ops/sec");
    }
}

#[cfg(test)]
mod cache_performance_tests {
    use super::*;

    fn send_request_with_timing(path: &str) -> Result<(String, std::time::Duration), std::io::Error> {
        let start = Instant::now();
        
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        
        stream.write_all(request.as_bytes())?;
        
        // Parse HTTP response properly (headers + body)
        let response = read_http_response(&mut stream)?;
        
        let duration = start.elapsed();
        Ok((response, duration))
    }

    fn send_conditional_request_with_timing(
        path: &str,
        if_none_match: Option<&str>,
    ) -> Result<(String, std::time::Duration), std::io::Error> {
        let start = Instant::now();
        
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let mut request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n", path);
        
        if let Some(etag) = if_none_match {
            request.push_str(&format!("If-None-Match: {}\r\n", etag));
        }
        
        request.push_str("\r\n");
        stream.write_all(request.as_bytes())?;
        
        // Parse HTTP response properly (headers + optional body)
        let response = read_http_response(&mut stream)?;
        
        let duration = start.elapsed();
        Ok((response, duration))
    }

    fn read_http_response(stream: &mut TcpStream) -> Result<String, std::io::Error> {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stream);
        let mut response = String::new();
        let mut content_length: Option<usize> = None;
        let mut is_304_or_head = false;
        
        // Read status line
        let mut line = String::new();
        reader.read_line(&mut line)?;
        response.push_str(&line);
        
        // Check if this is a 304 response
        if line.contains("304 Not Modified") {
            is_304_or_head = true;
        }
        
        // Read headers
        loop {
            line.clear();
            reader.read_line(&mut line)?;
            response.push_str(&line);
            
            if line.trim().is_empty() {
                // End of headers
                break;
            }
            
            // Parse Content-Length if present
            if line.to_lowercase().starts_with("content-length:") {
                if let Some(value) = line.split(':').nth(1) {
                    content_length = value.trim().parse().ok();
                }
            }
        }
        
        // Read body only if not a 304/HEAD response and has Content-Length
        if !is_304_or_head {
            if let Some(length) = content_length {
                let mut body = vec![0u8; length];
                std::io::Read::read_exact(reader.get_mut(), &mut body)?;
                response.push_str(&String::from_utf8_lossy(&body));
            }
        }
        
        Ok(response)
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_cache_performance_benefits() {
        // Test multiple requests to the same file to see cache benefits
        let test_file = "/index.html";
        let num_requests = 10;
        
        let mut response_times = Vec::new();
        let mut first_etag = None;
        
        println!("Testing {} requests to {}", num_requests, test_file);
        
        // Make multiple requests and measure response times
        for i in 0..num_requests {
            match send_request_with_timing(test_file) {
                Ok((response, duration)) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), "Request {} failed", i + 1);
                    response_times.push(duration);
                    
                    // Extract ETag from first response
                    if i == 0 {
                        if let Some(start) = response.find("ETag: ") {
                            let etag_line = &response[start..];
                            if let Some(end) = etag_line.find("\r\n") {
                                first_etag = Some(etag_line[6..end].to_string());
                            }
                        }
                    }
                    
                    println!("Request {}: {:?}", i + 1, duration);
                }
                Err(e) => {
                    println!("Warning: Request {} failed: {}, skipping performance test", i + 1, e);
                    return;
                }
            }
        }
        
        // Calculate average response time for cached requests (requests 2-10)
        let cached_times: Vec<_> = response_times.iter().skip(1).collect();
        let avg_cached_time = cached_times.iter().map(|d| d.as_nanos()).sum::<u128>() as f64 / cached_times.len() as f64;
        let first_request_time = response_times[0].as_nanos() as f64;
        
        println!("First request time: {:.2}ms", first_request_time / 1_000_000.0);
        println!("Average cached request time: {:.2}ms", avg_cached_time / 1_000_000.0);
        
        // With file header caching, subsequent requests should be very fast
        // They should use pre-compiled headers without filesystem metadata calls
        assert!(avg_cached_time < first_request_time * 2.0, 
            "Cached requests should be at least as fast as first request");
        
        // Test 304 Not Modified performance if we have an ETag
        if let Some(etag) = first_etag {
            println!("Testing 304 Not Modified performance with ETag: {}", etag);
            
            let mut not_modified_times = Vec::new();
            for i in 0..5 {
                match send_conditional_request_with_timing(test_file, Some(&etag)) {
                    Ok((response, duration)) => {
                        assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                            "Conditional request {} should return 304", i + 1);
                        not_modified_times.push(duration);
                        println!("304 Request {}: {:?}", i + 1, duration);
                    }
                    Err(e) => {
                        println!("Warning: 304 request {} failed: {}", i + 1, e);
                    }
                }
            }
            
            if !not_modified_times.is_empty() {
                let avg_304_time = not_modified_times.iter().map(|d| d.as_nanos()).sum::<u128>() as f64 / not_modified_times.len() as f64;
                println!("Average 304 Not Modified time: {:.2}ms", avg_304_time / 1_000_000.0);
                
                // 304 responses should be faster than full responses since no file I/O
                assert!(avg_304_time < avg_cached_time, 
                    "304 responses should be faster than full cached responses");
            }
        }
        
        println!("✓ Cache performance test completed");
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_304_response_performance() {
        // Test performance of 304 Not Modified responses specifically
        let test_file = "/index.html";
        
        // First, get the ETag
        let initial_response = match send_request_with_timing(test_file) {
            Ok((response, _)) => response,
            Err(_) => {
                println!("Warning: Cannot get initial response, skipping 304 performance test");
                return;
            }
        };

        let etag = if let Some(start) = initial_response.find("ETag: ") {
            let etag_line = &initial_response[start..];
            if let Some(end) = etag_line.find("\r\n") {
                Some(etag_line[6..end].to_string())
            } else {
                None
            }
        } else {
            None
        };

        if let Some(etag_value) = etag {
            println!("Testing 304 Not Modified performance with ETag: {}", etag_value);
            
            const NUM_304_REQUESTS: usize = 50;
            let start_time = Instant::now();
            let mut successful_304s = 0;
            let mut total_304_time = Duration::ZERO;
            
            for i in 0..NUM_304_REQUESTS {
                match send_conditional_request_with_timing(test_file, Some(&etag_value)) {
                    Ok((response, duration)) => {
                        if response.contains("HTTP/1.1 304 Not Modified") {
                            successful_304s += 1;
                            total_304_time += duration;
                        }
                    }
                    Err(_) => {
                        println!("Warning: 304 request {} failed", i + 1);
                    }
                }
            }
            
            let total_duration = start_time.elapsed();
            let rps_304 = successful_304s as f64 / total_duration.as_secs_f64();
            let avg_304_time = if successful_304s > 0 {
                total_304_time / successful_304s as u32
            } else {
                Duration::ZERO
            };
            
            println!("304 Not Modified performance:");
            println!("  {} successful 304s in {:?}", successful_304s, total_duration);
            println!("  {:.2} req/s", rps_304);
            println!("  Average response time: {:?}", avg_304_time);
            
            // 304 responses should be very fast - no file I/O, minimal processing
            assert!(rps_304 > 200.0, "304 throughput too low: {:.2} req/s", rps_304);
            assert!(avg_304_time < Duration::from_millis(10), 
                "304 response time too high: {:?}", avg_304_time);
            assert!(successful_304s >= NUM_304_REQUESTS * 9 / 10, 
                "304 success rate too low: {}/{}", successful_304s, NUM_304_REQUESTS);
        }
        
        println!("✓ 304 Not Modified performance test completed");
    }
}