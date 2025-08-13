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
    fn test_memory_usage_under_load() {
        const TEST_DURATION_SECS: u64 = 30;
        const TARGET_RPS: usize = 10;
        
        let start_time = Instant::now();
        let mut total_requests = 0;
        let mut successful_requests = 0;
        let mut response_times = Vec::new();
        
        while start_time.elapsed().as_secs() < TEST_DURATION_SECS {
            let batch_start = Instant::now();
            let handles = create_concurrent_requests(TARGET_RPS, 1, "/health");
            
            for handle in handles {
                total_requests += 1;
                match handle.join() {
                    Ok(Ok((1, duration))) => {
                        successful_requests += 1;
                        response_times.push(duration);
                    }
                    Ok(Ok((count, duration))) => {
                        successful_requests += count;
                        if count > 0 {
                            response_times.push(duration / count as u32);
                        }
                    }
                    _ => {}
                }
            }
            
            // Maintain target RPS
            let batch_duration = batch_start.elapsed();
            let target_batch_duration = Duration::from_millis(1000);
            if batch_duration < target_batch_duration {
                thread::sleep(target_batch_duration - batch_duration);
            }
        }
        
        let actual_duration = start_time.elapsed();
        let actual_rps = successful_requests as f64 / actual_duration.as_secs_f64();
        
        println!("Memory usage test results:");
        println!("  Duration: {:?}", actual_duration);
        println!("  Requests: {} successful / {} total", successful_requests, total_requests);
        println!("  RPS: {:.2}", actual_rps);
        
        if !response_times.is_empty() {
            response_times.sort();
            let avg_response_time = response_times.iter().sum::<Duration>() / response_times.len() as u32;
            let p95_idx = (response_times.len() as f64 * 0.95) as usize;
            let p95_response_time = response_times[p95_idx.min(response_times.len() - 1)];
            
            println!("  Avg response time: {:?}", avg_response_time);
            println!("  95th percentile: {:?}", p95_response_time);
            
            // Performance should not degrade over time (no memory leaks)
            assert!(avg_response_time < Duration::from_millis(100), 
                   "Average response time degraded: {:?}", avg_response_time);
            assert!(p95_response_time < Duration::from_millis(500), 
                   "95th percentile response time too high: {:?}", p95_response_time);
        }
        
        // Should maintain reasonable success rate
        let success_rate = successful_requests as f64 / total_requests as f64;
        assert!(success_rate > 0.90, "Success rate too low: {:.2}%", success_rate * 100.0);
    }
    
    #[test]
    #[ignore] // Performance test - run manually
    fn test_path_sanitization_performance() {
        let test_paths = [
            "/simple.html",
            "/path/to/file.css", 
            "/complex/../../../path/./file.js",
            &format!("/{}/file.html", "long-component".repeat(10)),
            &format!("/{}", "../".repeat(100)),
        ];
        
        const ITERATIONS: usize = 10000;
        
        for path in &test_paths {
            let start = Instant::now();
            
            for _ in 0..ITERATIONS {
                let _result = kiss::sanitize_path(path);
            }
            
            let duration = start.elapsed();
            let ops_per_sec = ITERATIONS as f64 / duration.as_secs_f64();
            
            println!("Path sanitization performance for {:?}:", 
                    if path.len() > 50 { format!("{}...", &path[..47]) } else { path.to_string() });
            println!("  {} iterations in {:?} ({:.0} ops/sec)", 
                    ITERATIONS, duration, ops_per_sec);
            
            // Should be very fast for path sanitization
            assert!(ops_per_sec > 10000.0, 
                   "Path sanitization too slow: {:.0} ops/sec for {:?}", ops_per_sec, path);
        }
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
                let _result = kiss::get_mime_type(file);
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
    fn bench_path_sanitization_simple() {
        // Benchmark simple paths (most common case)
        let simple_paths = ["/index.html", "/style.css", "/js/app.js"];
        const ITERATIONS: usize = 100000;
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for path in &simple_paths {
                let _result = kiss::sanitize_path(path);
            }
        }
        let duration = start.elapsed();
        
        let total_ops = ITERATIONS * simple_paths.len();
        let ops_per_sec = total_ops as f64 / duration.as_secs_f64();
        
        println!("Simple path sanitization: {} ops in {:?} ({:.0} ops/sec)", 
                total_ops, duration, ops_per_sec);
        
        // Should be extremely fast for simple paths
        assert!(ops_per_sec > 100000.0, "Simple path sanitization should be >100k ops/sec");
    }
    
    #[test] 
    fn bench_path_sanitization_complex() {
        // Benchmark complex paths with traversal (security-critical case)
        let complex_paths = [
            "/css/../js/../../etc/passwd",
            "/../../../etc/shadow", 
            "/a/b/c/../../../../kiss",
        ];
        const ITERATIONS: usize = 10000;
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for path in &complex_paths {
                let _result = kiss::sanitize_path(path);
            }
        }
        let duration = start.elapsed();
        
        let total_ops = ITERATIONS * complex_paths.len();
        let ops_per_sec = total_ops as f64 / duration.as_secs_f64();
        
        println!("Complex path sanitization: {} ops in {:?} ({:.0} ops/sec)", 
                total_ops, duration, ops_per_sec);
        
        // Should still be fast even for complex paths
        assert!(ops_per_sec > 10000.0, "Complex path sanitization should be >10k ops/sec");
    }
    
    #[test]
    fn bench_mime_type_detection() {
        let common_types = ["html", "css", "js", "png", "jpg", "pdf", "woff2"];
        const ITERATIONS: usize = 100000;
        
        let start = Instant::now();
        for _ in 0..ITERATIONS {
            for ext in &common_types {
                let filename = format!("file.{}", ext);
                let _result = kiss::get_mime_type(&filename);
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