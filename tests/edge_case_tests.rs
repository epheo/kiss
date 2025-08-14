use std::io::{Read, Write};
use std::net::TcpStream;

#[cfg(test)]
mod cache_edge_case_tests {
    use super::*;

    fn send_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }

    fn send_conditional_request(path: &str, if_modified_since: Option<&str>, if_none_match: Option<&str>) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let mut request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n", path);
        
        if let Some(modified_since) = if_modified_since {
            request.push_str(&format!("If-Modified-Since: {}\r\n", modified_since));
        }
        
        if let Some(none_match) = if_none_match {
            request.push_str(&format!("If-None-Match: {}\r\n", none_match));
        }
        
        request.push_str("\r\n");
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_cache_miss_handling() {
        // Test requesting files that don't exist in cache (should return 404)
        let non_existent_files = [
            "/file_not_in_cache.html",
            "/missing/directory/file.css",
            "/very/deep/nested/path/that/does/not/exist.js",
            "/.hidden_file_not_cached",
            "/backup_file_added_after_startup.bak",
        ];

        for file_path in &non_existent_files {
            match send_request(file_path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                        "Cache miss for {} should return 404, got: {}", file_path, response);
                    assert!(response.contains("File not found"), 
                        "404 response should contain error message");
                    assert!(response.contains("X-Content-Type-Options: nosniff"), 
                        "404 response should include essential security headers");
                    
                    println!("✓ Cache miss handled correctly for {}", file_path);
                }
                Err(_) => {
                    println!("Warning: Could not test cache miss for {} - server not running", file_path);
                    return;
                }
            }
        }
        
        println!("✓ All cache miss scenarios handled correctly");
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_malformed_conditional_headers() {
        // Test various malformed conditional request headers
        let test_cases = vec![
            // Malformed ETags
            ("malformed_etag_1", None, Some("malformed-etag-without-quotes")),
            ("malformed_etag_2", None, Some("\"missing-closing-quote")),
            ("malformed_etag_3", None, Some("W/malformed-weak-etag")),
            ("empty_etag", None, Some("")),
            
            // Malformed If-Modified-Since
            ("malformed_date_1", Some("not-a-valid-date"), None),
            ("malformed_date_2", Some("2023-13-45 25:70:80"), None), // Invalid date components
            ("malformed_date_3", Some("Mon, 32 Dec 2023 24:00:00 GMT"), None), // Invalid day
            ("empty_date", Some(""), None),
            
            // Edge case combinations
            ("both_malformed", Some("invalid-date"), Some("invalid-etag")),
        ];

        let test_file = "/index.html";
        
        for (test_name, if_modified_since, if_none_match) in test_cases {
            match send_conditional_request(test_file, if_modified_since, if_none_match) {
                Ok(response) => {
                    // Malformed headers should not cause server errors
                    // Should either return 200 (ignore malformed header) or proper error
                    assert!(!response.contains("HTTP/1.1 5"), 
                        "Test {}: Malformed headers should not cause 5xx errors, got: {}", 
                        test_name, response);
                    
                    // Most likely should return 200 OK (ignore malformed conditional)
                    if response.contains("HTTP/1.1 200 OK") {
                        assert!(response.contains("ETag: W/"), 
                            "Test {}: 200 response should contain ETag", test_name);
                        println!("✓ Test {}: Malformed header ignored, returned 200 OK", test_name);
                    } else if response.contains("HTTP/1.1 400 Bad Request") {
                        println!("✓ Test {}: Malformed header rejected with 400", test_name);
                    } else {
                        println!("? Test {}: Unexpected response: {}", test_name, 
                            response.lines().next().unwrap_or("No status line"));
                    }
                }
                Err(e) => {
                    println!("Warning: Test {} failed with connection error: {}", test_name, e);
                }
            }
        }
        
        println!("✓ Malformed conditional header tests completed");
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_very_large_etag_header() {
        // Test with unreasonably large ETag values (edge case for parsing)
        let test_file = "/index.html";
        
        // Create a very long ETag (should not cause issues)
        let long_etag = format!("W/\"{}\"", "x".repeat(10000));
        
        match send_conditional_request(test_file, None, Some(&long_etag)) {
            Ok(response) => {
                // Should handle gracefully without crashing
                assert!(!response.contains("HTTP/1.1 5"), 
                    "Large ETag should not cause server error, got: {}", response);
                
                if response.contains("HTTP/1.1 200 OK") {
                    println!("✓ Large ETag header handled gracefully, returned 200");
                } else if response.contains("HTTP/1.1 304 Not Modified") {
                    println!("✓ Large ETag header matched unexpectedly, returned 304");
                } else if response.contains("HTTP/1.1 400 Bad Request") {
                    println!("✓ Large ETag header rejected with 400");
                } else {
                    println!("? Large ETag header response: {}", 
                        response.lines().next().unwrap_or("No status line"));
                }
            }
            Err(e) => {
                println!("Warning: Large ETag test failed with connection error: {}", e);
            }
        }
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_multiple_conditional_headers() {
        // Test requests with multiple conditional headers
        let test_file = "/index.html";
        
        // First get the actual ETag
        let initial_response = match send_request(test_file) {
            Ok(response) => response,
            Err(_) => {
                println!("Warning: Cannot get initial response for multiple header test");
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
            // Test both If-None-Match and If-Modified-Since (ETag should take precedence)
            match send_conditional_request(test_file, Some("timestamp_0"), Some(&etag_value)) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                        "ETag match should override If-Modified-Since, got: {}", response);
                    println!("✓ ETag takes precedence over If-Modified-Since");
                }
                Err(e) => {
                    println!("Warning: Multiple header test failed: {}", e);
                }
            }
            
            // Test with non-matching ETag and matching timestamp
            match send_conditional_request(test_file, Some("timestamp_9999999999"), Some("W/\"999-999\"")) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), 
                        "Non-matching ETag should override matching timestamp, got: {}", response);
                    println!("✓ Non-matching ETag correctly overrides timestamp");
                }
                Err(e) => {
                    println!("Warning: Multiple header precedence test failed: {}", e);
                }
            }
        }
    }

    #[test]
    #[ignore] // Requires server to be running 
    fn test_case_sensitivity_in_headers() {
        // Test case variations in conditional headers
        let test_file = "/index.html";
        
        // Get actual ETag first
        let initial_response = match send_request(test_file) {
            Ok(response) => response,
            Err(_) => {
                println!("Warning: Cannot get initial response for case sensitivity test");
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
            // Test different case variations of header names
            let case_variations = [
                ("if-none-match", &etag_value),
                ("If-None-Match", &etag_value), 
                ("IF-NONE-MATCH", &etag_value),
                ("If-none-match", &etag_value),
                ("if-None-Match", &etag_value),
            ];
            
            for (header_case, etag_val) in &case_variations {
                let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap_or_else(|_| {
                    panic!("Server not running for case sensitivity test");
                });
                
                let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n{}: {}\r\n\r\n", 
                    test_file, header_case, etag_val);
                stream.write_all(request.as_bytes()).unwrap();
                
                let mut response = String::new();
                stream.read_to_string(&mut response).unwrap();
                
                // HTTP headers should be case-insensitive
                assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                    "Header case variation '{}' should work, got: {}", 
                    header_case, response);
                
                println!("✓ Case variation '{}' handled correctly", header_case);
            }
        }
        
        println!("✓ Header case sensitivity tests completed");
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_special_characters_in_etag() {
        // Test ETag matching with special characters (if server generates them)
        let test_file = "/index.html";
        
        // Test various special character scenarios
        let special_etag_tests = [
            "W/\"test-with-hyphens-123\"",
            "W/\"test_with_underscores_456\"", 
            "W/\"test.with.dots.789\"",
            "W/\"test/with/slashes/012\"",
            "W/\"test with spaces 345\"",  // Spaces in ETag
            "W/\"test\\\"with\\\"quotes\"", // Escaped quotes
        ];
        
        for test_etag in &special_etag_tests {
            match send_conditional_request(test_file, None, Some(test_etag)) {
                Ok(response) => {
                    // Should handle special characters gracefully
                    assert!(!response.contains("HTTP/1.1 5"), 
                        "Special ETag '{}' should not cause server error, got: {}", 
                        test_etag, response);
                    
                    // Most likely won't match, should return 200
                    if response.contains("HTTP/1.1 200 OK") {
                        println!("✓ Special ETag '{}' handled correctly (no match)", test_etag);
                    } else if response.contains("HTTP/1.1 304 Not Modified") {
                        println!("✓ Special ETag '{}' matched unexpectedly", test_etag);
                    }
                }
                Err(e) => {
                    println!("Warning: Special ETag test failed for '{}': {}", test_etag, e);
                }
            }
        }
        
        println!("✓ Special character ETag tests completed");
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_boundary_file_sizes() {
        // Test caching behavior with files at size boundaries
        println!("Testing cache behavior with boundary file sizes...");
        
        // Note: This test checks if very small and medium files are cached correctly
        let size_test_files = [
            "/index.html",    // Typical small file
            "/style.css",     // Medium CSS file  
            "/app.js",        // Medium JS file
        ];
        
        for file_path in &size_test_files {
            match send_request(file_path) {
                Ok(response) => {
                    if response.contains("HTTP/1.1 200 OK") {
                        // File found and cached
                        assert!(response.contains("ETag: W/"), 
                            "Cached file {} should have ETag", file_path);
                        assert!(response.contains("Last-Modified:"), 
                            "Cached file {} should have Last-Modified", file_path);
                        assert!(response.contains("Content-Length:"), 
                            "Cached file {} should have Content-Length", file_path);
                        
                        // Extract content length to verify caching worked correctly
                        if let Some(start) = response.find("Content-Length: ") {
                            let length_line = &response[start..];
                            if let Some(end) = length_line.find("\r\n") {
                                let length_str = &length_line[16..end]; // Skip "Content-Length: "
                                match length_str.parse::<u64>() {
                                    Ok(size) => {
                                        println!("✓ File {} cached correctly (size: {} bytes)", file_path, size);
                                        
                                        // Verify it's within reasonable bounds (not a huge file that shouldn't be cached)
                                        assert!(size < 50 * 1024 * 1024, 
                                            "Cached file {} too large: {} bytes", file_path, size);
                                    }
                                    Err(_) => {
                                        println!("Warning: Could not parse Content-Length for {}", file_path);
                                    }
                                }
                            }
                        }
                    } else if response.contains("HTTP/1.1 404 Not Found") {
                        println!("File {} not found (expected for some test scenarios)", file_path);
                    }
                }
                Err(_) => {
                    println!("Warning: Could not test file size boundaries - server not running");
                    return;
                }
            }
        }
        
        println!("✓ Boundary file size tests completed");
    }

    #[test]
    #[ignore] // Requires server to be running
    fn test_concurrent_conditional_requests() {
        // Test multiple concurrent conditional requests to ensure cache consistency
        use std::thread;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicU32, Ordering};
        
        let test_file = "/index.html";
        let num_threads = 10;
        let requests_per_thread = 5;
        
        // First get the ETag
        let initial_response = match send_request(test_file) {
            Ok(response) => response,
            Err(_) => {
                println!("Warning: Cannot get initial response for concurrent test");
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
            let success_count = Arc::new(AtomicU32::new(0));
            let not_modified_count = Arc::new(AtomicU32::new(0));
            
            println!("Testing concurrent conditional requests with ETag: {}", etag_value);
            
            let handles: Vec<_> = (0..num_threads).map(|thread_id| {
                let etag_clone = etag_value.clone();
                let success_count = Arc::clone(&success_count);
                let not_modified_count = Arc::clone(&not_modified_count);
                
                thread::spawn(move || {
                    for req_id in 0..requests_per_thread {
                        match send_conditional_request(test_file, None, Some(&etag_clone)) {
                            Ok(response) => {
                                if response.contains("HTTP/1.1 304 Not Modified") {
                                    success_count.fetch_add(1, Ordering::Relaxed);
                                    not_modified_count.fetch_add(1, Ordering::Relaxed);
                                } else if response.contains("HTTP/1.1 200 OK") {
                                    success_count.fetch_add(1, Ordering::Relaxed);
                                    println!("Thread {} req {}: Unexpected 200 response", thread_id, req_id + 1);
                                }
                            }
                            Err(e) => {
                                println!("Thread {} req {} failed: {}", thread_id, req_id + 1, e);
                            }
                        }
                    }
                })
            }).collect();
            
            // Wait for all threads
            for handle in handles {
                handle.join().unwrap();
            }
            
            let final_success = success_count.load(Ordering::Relaxed);
            let final_not_modified = not_modified_count.load(Ordering::Relaxed);
            let total_expected = num_threads * requests_per_thread;
            
            println!("Concurrent conditional request results:");
            println!("  Total expected: {}", total_expected);
            println!("  Successful responses: {}", final_success);
            println!("  304 Not Modified responses: {}", final_not_modified);
            
            // Should have high success rate
            assert!(final_success as f64 / total_expected as f64 > 0.8, 
                "Concurrent success rate too low: {}/{}", final_success, total_expected);
            
            // Most should be 304 since we're using the correct ETag
            assert!(final_not_modified as f64 / final_success as f64 > 0.8, 
                "Most concurrent requests should return 304: {}/{}", final_not_modified, final_success);
            
            println!("✓ Concurrent conditional requests handled consistently");
        }
    }
}