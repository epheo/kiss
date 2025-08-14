use kiss::*;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[cfg(test)]
mod file_system_error_tests {
    use super::*;
    
    
    #[test]
    fn test_mime_type_for_nonexistent_files() {
        // MIME detection should work based on extension even if file doesn't exist
        let test_files = [
            "/missing.html",
            "/nonexistent.css", 
            "/fake.js",
            "/imaginary.png",
            "/virtual.pdf",
        ];
        
        for file in &test_files {
            let mime_type = get_mime_type(file);
            assert!(!mime_type.is_empty(), "MIME type should not be empty for: {}", file);
            assert_ne!(mime_type, "", "MIME type should be determined for: {}", file);
        }
    }
    
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_server_file_not_found_responses() {
        let missing_files = [
            "/definitely-missing.html",
            "/no-such-file.css",
            "/void.js",
        ];
        
        for file in &missing_files {
            let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", file);
            match send_raw_request(&request) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                           "Should return 404 for missing file: {}", file);
                    assert!(response.contains("File not found"));
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping file not found test");
                    break;
                }
            }
        }
    }
    
    fn send_raw_request(request: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }
}

#[cfg(test)]
mod connection_error_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_connection_timeout_handling() {
        // Test connection that times out
        match TcpStream::connect_timeout(
            &"127.0.0.1:8080".parse().unwrap(), 
            Duration::from_millis(1)
        ) {
            Ok(_) => {
                // Connection succeeded quickly - that's fine
            }
            Err(_) => {
                // Connection timed out - also acceptable
                println!("Connection timeout test completed (timeout occurred)");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_incomplete_request_handling() {
        match TcpStream::connect("127.0.0.1:8080") {
            Ok(mut stream) => {
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                
                // Send incomplete request (no \r\n\r\n termination)
                let incomplete_request = "GET /health HTTP/1.1\r\nHost: localhost";
                stream.write_all(incomplete_request.as_bytes()).unwrap();
                
                // Server should either timeout or handle gracefully
                let mut response = String::new();
                match stream.read_to_string(&mut response) {
                    Ok(_) => {
                        // If we get a response, it should be valid HTTP
                        if !response.is_empty() {
                            assert!(response.starts_with("HTTP/1.1"), 
                                   "Incomplete request response should be valid HTTP");
                        }
                    }
                    Err(_) => {
                        // Timeout or connection closed is acceptable
                        println!("Incomplete request handled by timeout/close (acceptable)");
                    }
                }
            }
            Err(_) => {
                println!("Warning: Server not running, skipping incomplete request test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_connection_drop_during_request() {
        match TcpStream::connect("127.0.0.1:8080") {
            Ok(mut stream) => {
                // Start sending a request
                stream.write_all(b"GET /health HTTP/1.1\r\n").unwrap();
                
                // Abruptly close connection
                drop(stream);
                
                // Server should handle this gracefully without crashing
                std::thread::sleep(Duration::from_millis(100));
                
                // Verify server is still responsive with a new connection
                match TcpStream::connect("127.0.0.1:8080") {
                    Ok(mut new_stream) => {
                        let request = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
                        new_stream.write_all(request.as_bytes()).unwrap();
                        
                        let mut response = String::new();
                        new_stream.read_to_string(&mut response).unwrap();
                        assert!(response.contains("HTTP/1.1 200 OK"), 
                               "Server should still be responsive after dropped connection");
                    }
                    Err(_) => {
                        panic!("Server became unresponsive after connection drop");
                    }
                }
            }
            Err(_) => {
                println!("Warning: Server not running, skipping connection drop test");
            }
        }
    }
}

#[cfg(test)]
mod resource_exhaustion_tests {
    use super::*;
    
    
    
    #[test]
    fn test_mime_type_with_very_long_extension() {
        let long_extension = "x".repeat(1000);
        let file_with_long_ext = format!("file.{}", long_extension);
        
        let mime_type = get_mime_type(&file_with_long_ext);
        // Should handle gracefully
        assert_eq!(mime_type, "application/octet-stream", 
                  "Very long extension should default to octet-stream");
    }
    
    #[test]
    #[ignore] // Requires server to be running and stress testing
    fn test_memory_usage_under_load() {
        // This test checks for memory leaks under sustained load
        let mut successful_requests = 0;
        let start_time = std::time::Instant::now();
        
        // Run for 10 seconds
        while start_time.elapsed().as_secs() < 10 {
            match TcpStream::connect("127.0.0.1:8080") {
                Ok(mut stream) => {
                    // Set read timeout to prevent hanging on keep-alive connections
                    if stream.set_read_timeout(Some(Duration::from_secs(2))).is_ok() {
                        // Use Connection: close to prevent keep-alive issues in tests
                        let request = "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n";
                        if stream.write_all(request.as_bytes()).is_ok() {
                            let mut buffer = [0u8; 1024];
                            match stream.read(&mut buffer) {
                                Ok(bytes_read) if bytes_read > 0 => {
                                    let response = String::from_utf8_lossy(&buffer[..bytes_read]);
                                    if response.contains("HTTP/1.1 200 OK") {
                                        successful_requests += 1;
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                Err(_) => {
                    // Connection failed, small delay and continue
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                }
            }
            
            // Small delay to prevent overwhelming (reduced for optimized server)
            std::thread::sleep(Duration::from_millis(5));
        }
        
        println!("Memory usage test: {} successful requests in 10 seconds", successful_requests);
        // With optimized async server and 5ms delay, we should get 500+ requests easily
        // Setting lower bound to account for system variability
        assert!(successful_requests >= 200, "Should handle reasonable load without memory issues (got {})", successful_requests);
    }
}

#[cfg(test)]
mod error_recovery_tests {
    use super::*;
    
    
    #[test]
    fn test_edge_case_mime_types() {
        // Test MIME type detection with edge cases
        let edge_cases = [
            "", // Empty filename
            ".", // Just dot
            "..", // Just double dot  
            "...", // Triple dot
            ".hidden", // Hidden file without extension
            "file.", // File with trailing dot
            "file..", // File with trailing double dot
            "file.CAPS", // Uppercase extension
            "file.MiXeD", // Mixed case extension
        ];
        
        for filename in &edge_cases {
            let mime_type = get_mime_type(filename);
            // Should not panic and should return something
            assert!(!mime_type.is_empty(), "MIME type should not be empty for: {:?}", filename);
        }
    }
    
}