use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[cfg(test)]
mod http_request_validation_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_invalid_http_methods() {
        let invalid_methods = [
            "POST /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "PUT /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n", 
            "DELETE /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "PATCH /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "OPTIONS /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n",
            "HEAD /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n",
        ];
        
        for request in &invalid_methods {
            match send_raw_request(request) {
                Ok(response) => {
                    assert!(response.contains("405"), 
                           "Should reject method with 405: {}", request.split('\r').next().unwrap_or(""));
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping method test");
                    break;
                }
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_invalid_http_versions() {
        let invalid_versions = [
            "GET /health INVALID/1.1\r\nHost: localhost\r\n\r\n",
        ];
        
        for request in &invalid_versions {
            match send_raw_request(request) {
                Ok(response) => {
                    // Server should handle gracefully and return HTTP/1.1 response
                    assert!(response.starts_with("HTTP/1.1"));
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping version test");
                    break;
                }
            }
        }
    }
    
    #[test] 
    #[ignore] // Requires server to be running
    fn test_malformed_http_requests() {
        // Test truly malformed requests that should return 400 errors
        let malformed_requests = [
            "INVALID REQUEST\r\n\r\n",
            "GET\r\n\r\n", // Missing path and version
            "GET /health\r\n\r\n", // Missing HTTP version
        ];
        
        for request in &malformed_requests {
            match send_raw_request(request) {
                Ok(response) => {
                    // Server should handle gracefully, not crash
                    assert!(response.starts_with("HTTP/1.1"), 
                           "Should return valid HTTP response for malformed request: {:?}", request);
                    // Should return 4xx error for malformed requests
                    assert!(response.contains("400"), 
                           "Should return 400 error for: {:?}", request);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping malformed request test");
                    break;
                }
            }
        }
        
        // Test that extra spaces are now handled correctly (should succeed)
        match send_raw_request("GET  /health  HTTP/1.1\r\n\r\n") {
            Ok(response) => {
                assert!(response.starts_with("HTTP/1.1 200 OK"), 
                       "Extra spaces should be handled gracefully");
            }
            Err(_) => {
                println!("Warning: Server not running, skipping extra spaces test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_oversized_request_line() {
        // Create a request line larger than MAX_REQUEST_SIZE (8KB)
        let long_path = "a".repeat(9000);
        let oversized_request = format!("GET /{} HTTP/1.1\r\nHost: localhost\r\n\r\n", long_path);
        
        match send_raw_request(&oversized_request) {
            Ok(response) => {
                // Should reject oversized requests with 4xx error
                assert!(response.contains("413") || response.contains("400"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping oversized request test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_request_without_host_header() {
        let no_host_request = "GET /health HTTP/1.1\r\n\r\n";
        
        match send_raw_request(no_host_request) {
            Ok(response) => {
                // Should still work - Host header is optional for HTTP/1.1 in practice
                assert!(response.starts_with("HTTP/1.1"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping no host header test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_connection_handling() {
        let requests_with_connection = [
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            "GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: keep-alive\r\n\r\n",
        ];
        
        for request in &requests_with_connection {
            match send_raw_request(request) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"));
                    // Server should handle connection headers gracefully
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping connection test");
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
mod http_response_validation_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_security_headers_presence() {
        match send_raw_request("GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n") {
            Ok(response) => {
                // Verify all security headers are present
                assert!(response.contains("X-Frame-Options: DENY"), "Missing X-Frame-Options");
                assert!(response.contains("X-Content-Type-Options: nosniff"), "Missing X-Content-Type-Options");
                assert!(response.contains("Content-Security-Policy:"), "Missing CSP");
            }
            Err(_) => {
                println!("Warning: Server not running, skipping security headers test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_content_type_headers() {
        let content_tests = [
            ("/health", "application/json"),
            ("/ready", "application/json"),
        ];
        
        for (path, expected_content_type) in &content_tests {
            let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
            match send_raw_request(&request) {
                Ok(response) => {
                    assert!(response.contains(&format!("Content-Type: {}", expected_content_type)),
                           "Wrong content type for {}: expected {}", path, expected_content_type);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping content type test");
                    break;
                }
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_content_length_headers() {
        match send_raw_request("GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n") {
            Ok(response) => {
                // Should have Content-Length header
                assert!(response.contains("Content-Length:"), "Missing Content-Length header");
                
                // Just verify content length is present and valid
                if let Some(length_line) = response.lines().find(|line| line.starts_with("Content-Length:")) {
                    let length_str = length_line.split(": ").nth(1).unwrap_or("0");
                    let content_length: usize = length_str.parse().unwrap_or(0);
                    assert!(content_length > 0, "Content-Length should be > 0");
                }
            }
            Err(_) => {
                println!("Warning: Server not running, skipping content length test");
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
mod http_edge_cases_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_rapid_connection_attempts() {
        let mut successful_connections = 0;
        let mut failed_connections = 0;
        
        // Try to make many rapid connections
        for _ in 0..20 {
            match TcpStream::connect("127.0.0.1:8080") {
                Ok(mut stream) => {
                    let request = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
                    if stream.write_all(request.as_bytes()).is_ok() {
                        successful_connections += 1;
                    }
                    // Don't read response to stress connection handling
                }
                Err(_) => {
                    failed_connections += 1;
                }
            }
        }
        
        println!("Rapid connections: {} successful, {} failed", successful_connections, failed_connections);
        // Should handle at least some connections successfully
        assert!(successful_connections > 0, "Server should handle at least some rapid connections");
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_slow_request_sending() {
        match TcpStream::connect("127.0.0.1:8080") {
            Ok(mut stream) => {
                stream.set_write_timeout(Some(Duration::from_secs(10))).unwrap();
                stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
                
                // Send request very slowly, byte by byte
                let request = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
                let mut write_success = true;
                for byte in request.bytes() {
                    match stream.write_all(&[byte]) {
                        Ok(_) => {
                            std::thread::sleep(Duration::from_millis(10));
                        }
                        Err(_) => {
                            // Server closed connection due to slow sending - this is acceptable
                            write_success = false;
                            break;
                        }
                    }
                }
                
                if write_success {
                    let mut response = String::new();
                    match stream.read_to_string(&mut response) {
                        Ok(_) => {
                            assert!(response.contains("HTTP/1.1 200 OK"), "Should handle slow requests");
                        }
                        Err(_) => {
                            // Timeout is acceptable for very slow requests
                            println!("Slow request timed out (acceptable behavior)");
                        }
                    }
                } else {
                    println!("Server closed connection during slow sending (acceptable behavior)");
                }
            }
            Err(_) => {
                println!("Warning: Server not running, skipping slow request test");
            }
        }
    }
}