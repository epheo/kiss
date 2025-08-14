use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

fn create_concurrent_connections(num_connections: usize, path: &str) -> Vec<thread::JoinHandle<Result<String, String>>> {
    let barrier = Arc::new(Barrier::new(num_connections + 1));
    let mut handles = Vec::new();
    
    for _ in 0..num_connections {
        let barrier_clone = Arc::clone(&barrier);
        let path_clone = path.to_string();
        
        let handle = thread::spawn(move || {
            barrier_clone.wait();
            send_get_request(&path_clone)
        });
        
        handles.push(handle);
    }
    
    barrier.wait(); // Release all threads at once
    handles
}

fn send_get_request(path: &str) -> Result<String, String> {
    let mut stream = TcpStream::connect("127.0.0.1:8080")
        .map_err(|e| format!("Connection failed: {}", e))?;
    
    stream.set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("Failed to set read timeout: {}", e))?;
    
    stream.set_write_timeout(Some(Duration::from_secs(10)))
        .map_err(|e| format!("Failed to set write timeout: {}", e))?;
    
    let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n", path);
    
    stream.write_all(request.as_bytes())
        .map_err(|e| format!("Write failed: {}", e))?;
    
    let mut response = String::new();
    stream.read_to_string(&mut response)
        .map_err(|e| format!("Read failed: {}", e))?;
    
    Ok(response)
}

#[cfg(test)]
mod worker_pool_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_concurrent_health_requests() {
        const NUM_CONCURRENT: usize = 10;
        
        let handles = create_concurrent_connections(NUM_CONCURRENT, "/health");
        let mut successful_requests = 0;
        let mut failed_requests = 0;
        
        for handle in handles {
            match handle.join() {
                Ok(Ok(response)) => {
                    if response.contains("HTTP/1.1 200 OK") && response.contains(r#""status":"healthy""#) {
                        successful_requests += 1;
                    } else {
                        failed_requests += 1;
                        println!("Unexpected response: {}", &response[..200.min(response.len())]);
                    }
                }
                Ok(Err(e)) => {
                    failed_requests += 1;
                    println!("Request failed: {}", e);
                }
                Err(_) => {
                    failed_requests += 1;
                    println!("Thread panicked");
                }
            }
        }
        
        println!("Concurrent test results: {} successful, {} failed", successful_requests, failed_requests);
        
        // At least half of the requests should succeed under normal load
        assert!(successful_requests >= NUM_CONCURRENT / 2, 
                "Too many failed requests: {}/{}", failed_requests, NUM_CONCURRENT);
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_worker_pool_saturation() {
        const NUM_CONCURRENT: usize = 50; // Test high concurrency behavior
        
        let start_time = Instant::now();
        let handles = create_concurrent_connections(NUM_CONCURRENT, "/health");
        
        let mut results = Vec::new();
        for handle in handles {
            results.push(handle.join());
        }
        
        let duration = start_time.elapsed();
        
        let mut successful_requests = 0;
        let mut connection_failures = 0;
        let mut service_unavailable = 0;
        
        for result in results {
            match result {
                Ok(Ok(response)) => {
                    if response.contains("HTTP/1.1 200 OK") {
                        successful_requests += 1;
                    } else if response.contains("HTTP/1.1 503 Service Unavailable") {
                        service_unavailable += 1;
                    } else {
                        println!("Unexpected response: {}", &response[..200.min(response.len())]);
                    }
                }
                Ok(Err(_)) => connection_failures += 1,
                Err(_) => connection_failures += 1,
            }
        }
        
        println!("Pool saturation test results:");
        println!("  Successful: {}", successful_requests);
        println!("  Service unavailable: {}", service_unavailable);
        println!("  Connection failures: {}", connection_failures);
        println!("  Duration: {:?}", duration);
        
        // Under saturation, we expect either successful responses or proper 503 responses
        let total_handled = successful_requests + service_unavailable;
        assert!(total_handled > 0, "Server should handle at least some requests");
        
        // Test should complete in reasonable time (not hang indefinitely)
        assert!(duration.as_secs() < 30, "Test took too long: {:?}", duration);
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_connection_cleanup() {
        // Test that connections are properly cleaned up after requests
        const NUM_ITERATIONS: usize = 20;
        
        for i in 0..NUM_ITERATIONS {
            let start = Instant::now();
            
            match send_get_request("/health") {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"));
                    let duration = start.elapsed();
                    
                    // Each request should complete quickly if connections are cleaned up properly
                    assert!(duration.as_millis() < 1000, 
                           "Request {} took too long: {:?}", i, duration);
                }
                Err(e) => {
                    panic!("Request {} failed: {}", i, e);
                }
            }
            
            // Small delay between requests
            thread::sleep(Duration::from_millis(50));
        }
        
        println!("All {} sequential requests completed successfully", NUM_ITERATIONS);
    }
}

#[cfg(test)]
mod performance_benchmarks {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn benchmark_health_endpoint_throughput() {
        const NUM_REQUESTS: usize = 100;
        const CONCURRENCY_LEVELS: [usize; 4] = [1, 5, 10, 20];
        
        for &concurrency in &CONCURRENCY_LEVELS {
            let batches = NUM_REQUESTS / concurrency;
            let start_time = Instant::now();
            
            let mut total_successful = 0;
            
            for _ in 0..batches {
                let handles = create_concurrent_connections(concurrency, "/health");
                
                for handle in handles {
                    if let Ok(Ok(response)) = handle.join() {
                        if response.contains("HTTP/1.1 200 OK") {
                            total_successful += 1;
                        }
                    }
                }
            }
            
            let total_duration = start_time.elapsed();
            let requests_per_second = total_successful as f64 / total_duration.as_secs_f64();
            
            println!("Concurrency {}: {}/{} requests in {:?} ({:.2} req/s)", 
                    concurrency, total_successful, NUM_REQUESTS, total_duration, requests_per_second);
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_memory_usage_stability() {
        // This test simulates sustained load to check for memory leaks
        const DURATION_SECONDS: u64 = 30;
        const REQUESTS_PER_SECOND: usize = 5;
        
        let start_time = Instant::now();
        let mut request_count = 0;
        let mut error_count = 0;
        
        while start_time.elapsed().as_secs() < DURATION_SECONDS {
            let batch_start = Instant::now();
            let handles = create_concurrent_connections(REQUESTS_PER_SECOND, "/health");
            
            for handle in handles {
                match handle.join() {
                    Ok(Ok(response)) => {
                        if response.contains("HTTP/1.1 200 OK") {
                            request_count += 1;
                        } else {
                            error_count += 1;
                        }
                    }
                    _ => error_count += 1,
                }
            }
            
            // Maintain roughly REQUESTS_PER_SECOND rate
            let elapsed = batch_start.elapsed();
            if elapsed < Duration::from_millis(1000) {
                thread::sleep(Duration::from_millis(1000) - elapsed);
            }
        }
        
        println!("Memory stability test completed:");
        println!("  Duration: {} seconds", DURATION_SECONDS);
        println!("  Successful requests: {}", request_count);
        println!("  Errors: {}", error_count);
        println!("  Success rate: {:.2}%", 
                (request_count as f64 / (request_count + error_count) as f64) * 100.0);
        
        // Should maintain high success rate throughout the test
        let success_rate = request_count as f64 / (request_count + error_count) as f64;
        assert!(success_rate > 0.95, "Success rate too low: {:.2}%", success_rate * 100.0);
    }
}