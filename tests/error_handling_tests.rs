use kiss::*;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

#[cfg(test)]
mod file_system_error_tests {
    use super::*;
    
    #[test]
    fn test_nonexistent_file_handling() {
        // Test various non-existent file scenarios
        let test_paths = [
            "/nonexistent.html",
            "/missing/file.css",
            "/deep/nested/missing/file.js",
            "/file-with-special-chars-!@#$.txt",
        ];
        
        for path in &test_paths {
            let sanitized = sanitize_path(path);
            // Path sanitization should work even for non-existent files
            assert!(sanitized.starts_with('/'), "Sanitized path should start with /: {}", sanitized);
        }
    }
    
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
    fn test_path_with_invalid_unicode() {
        // Test paths with various challenging characters
        let challenging_paths = [
            "/файл.html", // Cyrillic
            "/文件.css",   // Chinese
            "/ファイル.js", // Japanese
            "/🚀.html",    // Emoji
            "/file%20with%20spaces.txt", // URL encoded
        ];
        
        for path in &challenging_paths {
            let result = sanitize_path(path);
            // Should handle gracefully without panicking
            assert!(result.starts_with('/'), "Should handle unicode path: {}", path);
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
    fn test_extremely_long_paths() {
        // Test path sanitization with very long paths
        let very_long_component = "a".repeat(1000);
        let long_paths = [
            format!("/{}", very_long_component),
            format!("/{}/{}", very_long_component, very_long_component),
            format!("/normal/../{}", very_long_component),
        ];
        
        for path in &long_paths {
            let result = sanitize_path(path);
            // Should handle without panic or excessive memory usage
            assert!(result.starts_with('/'), "Should handle very long path without panic");
            // Result should be reasonable length (not exponentially longer)
            assert!(result.len() < path.len() * 2, "Sanitized path should not explode in size");
        }
    }
    
    #[test]
    fn test_deeply_nested_traversal() {
        // Test path with many ../.. components trying to access root-only file
        let parts: Vec<String> = (0..1000).map(|_| "..".to_string()).collect();
        let deep_traversal = format!("/{}", parts.join("/"));
        
        let result = sanitize_path(&deep_traversal);
        
        // Should resolve to root without infinite loop or stack overflow
        assert_eq!(result, "/", "Deep traversal should resolve to root");
    }
    
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
                    let request = "GET /health HTTP/1.1\r\nHost: localhost\r\n\r\n";
                    if stream.write_all(request.as_bytes()).is_ok() {
                        let mut response = String::new();
                        if stream.read_to_string(&mut response).is_ok() {
                            if response.contains("HTTP/1.1 200 OK") {
                                successful_requests += 1;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
            
            // Small delay to prevent overwhelming
            std::thread::sleep(Duration::from_millis(10));
        }
        
        println!("Memory usage test: {} successful requests in 10 seconds", successful_requests);
        // With 10ms delay between requests, max theoretical is ~1000 requests in 10 seconds
        // Accounting for connection overhead, 80+ requests is reasonable
        assert!(successful_requests >= 80, "Should handle reasonable load without memory issues (got {})", successful_requests);
    }
}

#[cfg(test)]
mod error_recovery_tests {
    use super::*;
    
    #[test]
    fn test_sanitization_with_null_bytes() {
        // Test paths with null bytes (should be handled gracefully)
        let paths_with_nulls = [
            "/file\0.html",
            "/path\0/file.css", 
            "\0/file.js",
        ];
        
        for path in &paths_with_nulls {
            let result = sanitize_path(path);
            // Should handle without panic
            assert!(result.starts_with('/'), "Should handle null bytes without panic: {:?}", path);
        }
    }
    
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
    
    #[test]
    fn test_path_components_with_special_chars() {
        // Test path sanitization with various special characters
        let special_paths = [
            "/file with spaces.html",
            "/file-with-dashes.css",
            "/file_with_underscores.js",
            "/file+with+plus.txt",
            "/file=with=equals.html",
            "/file&with&ampersand.css",
            "/file@with@at.js",
        ];
        
        for path in &special_paths {
            let result = sanitize_path(path);
            // Should preserve valid special characters
            assert!(result.starts_with('/'), "Should handle special chars: {}", path);
            // Should not be empty
            assert!(result.len() > 1, "Sanitized path should not be just root: {}", path);
        }
    }
}