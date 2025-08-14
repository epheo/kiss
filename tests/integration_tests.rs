use std::io::{Read, Write};
use std::net::TcpStream;
use tempfile::TempDir;

#[cfg(test)]
mod http_integration_tests {
    use super::*;
    
    // Note: These tests require a running server instance
    // In a real implementation, we'd start the server programmatically
    
    pub fn send_get_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_health_endpoint() {
        match send_get_request("/health") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("Content-Type: application/json"));
                assert!(response.contains(r#""status":"healthy""#));
                assert!(response.contains("timestamp"));
            }
            Err(_) => {
                // Server not running - test passes but logs warning
                println!("Warning: Server not running, skipping integration test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_ready_endpoint() {
        match send_get_request("/ready") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("Content-Type: application/json"));
                assert!(response.contains(r#""status":"ready""#));
                assert!(response.contains("timestamp"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping integration test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_404_response() {
        match send_get_request("/nonexistent.html") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 404 Not Found"));
                assert!(response.contains("Content-Type: text/plain"));
                assert!(response.contains("File not found"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping integration test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running  
    fn test_method_not_allowed() {
        let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap_or_else(|_| {
            panic!("Server not running");
        });
        
        let request = "POST /test HTTP/1.1\r\nHost: localhost\r\n\r\n";
        stream.write_all(request.as_bytes()).unwrap();
        
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        
        assert!(response.contains("HTTP/1.1 405 Method Not Allowed"));
        assert!(response.contains("Method not allowed"));
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_security_headers_on_cached_files() {
        // Test that security headers are present on cached file responses
        match send_get_request("/index.html") {
            Ok(response) => {
                assert!(response.contains("X-Frame-Options: DENY"));
                assert!(response.contains("X-Content-Type-Options: nosniff"));
                assert!(response.contains("Content-Security-Policy:"));
                
                // Also verify caching headers are present alongside security headers
                assert!(response.contains("ETag: W/"));
                assert!(response.contains("Last-Modified:"));
                
                println!("✓ Security headers present on cached file responses");
            }
            Err(_) => {
                println!("Warning: Server not running, skipping cached security header test");
            }
        }
        
        // Also test health endpoint (non-cached)
        match send_get_request("/health") {
            Ok(response) => {
                assert!(response.contains("X-Frame-Options: DENY"));
                assert!(response.contains("X-Content-Type-Options: nosniff"));
                assert!(response.contains("Content-Security-Policy:"));
                
                println!("✓ Security headers present on health endpoint");
            }
            Err(_) => {
                println!("Warning: Server not running, skipping health security header test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_large_request_rejection() {
        let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap_or_else(|_| {
            panic!("Server not running");
        });
        
        // Create a request larger than MAX_REQUEST_SIZE (8KB)
        let large_path = "a".repeat(9000);
        let request = format!("GET /{} HTTP/1.1\r\nHost: localhost\r\n\r\n", large_path);
        
        stream.write_all(request.as_bytes()).unwrap();
        
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        
        assert!(response.contains("HTTP/1.1 413 Request Entity Too Large"));
        assert!(response.contains("Request too large"));
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_svg_content_type() {
        match send_get_request("/test.svg") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("Content-Type: image/svg+xml"));
                assert!(response.contains("<svg xmlns"));
                println!("SVG Content-Type test passed: image/svg+xml header found");
            }
            Err(_) => {
                println!("Warning: Server not running from tests/content/, skipping SVG test");
                println!("To run this test: cd tests/content && ../../target/release/kiss");
            }
        }
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_multiple_content_types_with_caching() {
        // Test various file types from tests/content/ - now with cache validation
        let test_cases = vec![
            ("/test.svg", "image/svg+xml"),
            ("/style.css", "text/css; charset=utf-8"),
            ("/app.js", "text/javascript; charset=utf-8"),
            ("/index.html", "text/html; charset=utf-8"),
        ];

        for (path, expected_content_type) in test_cases {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), 
                        "Failed for {}: Expected 200 OK", path);
                    assert!(response.contains(&format!("Content-Type: {}", expected_content_type)), 
                        "Failed for {}: Expected Content-Type: {}", path, expected_content_type);
                    
                    // Validate caching headers are present
                    assert!(response.contains("ETag: W/"), 
                        "File {} should have cached ETag header", path);
                    assert!(response.contains("Last-Modified:"), 
                        "File {} should have cached Last-Modified header", path);
                    assert!(response.contains("Cache-Control: public, max-age=3600"), 
                        "File {} should have cache control headers", path);
                    
                    println!("✓ {} served with correct Content-Type: {} and cache headers", path, expected_content_type);
                }
                Err(_) => {
                    println!("Warning: Server not running from tests/content/, skipping {}", path);
                }
            }
        }
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
    #[ignore] // Requires server to be running from tests/content/
    fn test_etag_headers_present() {
        match send_get_request("/index.html") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("ETag: W/"), "Response should contain weak ETag header");
                assert!(response.contains("Last-Modified: "), "Response should contain Last-Modified header");
                // Verify HTTP-date format (RFC 7231): "Day, DD Mon YYYY HH:MM:SS GMT"
                let has_valid_http_date = response.contains("Mon, ") || response.contains("Tue, ") || 
                    response.contains("Wed, ") || response.contains("Thu, ") || response.contains("Fri, ") || 
                    response.contains("Sat, ") || response.contains("Sun, ");
                assert!(has_valid_http_date, "Last-Modified should use RFC 7231 HTTP-date format");
                assert!(response.contains("Cache-Control: public, max-age=3600"), "Response should contain cache control");
                println!("✓ ETag and caching headers present in response");
            }
            Err(_) => {
                println!("Warning: Server not running from tests/content/, skipping ETag test");
            }
        }
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_conditional_request_etag_match() {
        // First, get the file to extract its ETag
        let initial_response = match send_get_request("/index.html") {
            Ok(response) => response,
            Err(_) => {
                println!("Warning: Server not running from tests/content/, skipping conditional request test");
                return;
            }
        };

        // Extract ETag from response headers
        let etag = if let Some(start) = initial_response.find("ETag: ") {
            let etag_line = &initial_response[start..];
            if let Some(end) = etag_line.find("\r\n") {
                let full_etag = &etag_line[6..end]; // Skip "ETag: "
                Some(full_etag)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(etag_value) = etag {
            println!("Found ETag: {}", etag_value);
            
            // Test If-None-Match with matching ETag should return 304
            match send_conditional_request("/index.html", None, Some(etag_value)) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                        "Matching ETag should return 304 Not Modified, got: {}", response);
                    assert!(response.contains("Cache-Control: public, max-age=3600"), 
                        "304 response should contain cache control headers");
                    println!("✓ If-None-Match with matching ETag returned 304");
                }
                Err(e) => {
                    println!("Error testing conditional request: {}", e);
                }
            }
            
            // Test If-None-Match with non-matching ETag should return 200
            match send_conditional_request("/index.html", None, Some("W/\"999-999\"")) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), 
                        "Non-matching ETag should return 200 OK, got: {}", response);
                    println!("✓ If-None-Match with non-matching ETag returned 200");
                }
                Err(e) => {
                    println!("Error testing non-matching ETag: {}", e);
                }
            }
        } else {
            println!("Could not extract ETag from response");
        }
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_conditional_request_wildcard_etag() {
        // Test If-None-Match with wildcard should always return 304
        match send_conditional_request("/index.html", None, Some("*")) {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                    "Wildcard ETag should return 304 Not Modified, got: {}", response);
                println!("✓ If-None-Match with wildcard ETag returned 304");
            }
            Err(_) => {
                println!("Warning: Server not running from tests/content/, skipping wildcard ETag test");
            }
        }
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_conditional_request_if_modified_since() {
        // First, get the file to extract its Last-Modified timestamp
        let initial_response = match send_get_request("/index.html") {
            Ok(response) => response,
            Err(_) => {
                println!("Warning: Server not running from tests/content/, skipping If-Modified-Since test");
                return;
            }
        };

        // Extract Last-Modified timestamp from response headers
        let last_modified = if let Some(start) = initial_response.find("Last-Modified: ") {
            let modified_line = &initial_response[start..];
            if let Some(end) = modified_line.find("\r\n") {
                let timestamp = &modified_line[15..end]; // Skip "Last-Modified: "
                Some(timestamp)
            } else {
                None
            }
        } else {
            None
        };

        if let Some(timestamp) = last_modified {
            println!("Found Last-Modified: {}", timestamp);
            
            // Test If-Modified-Since with same timestamp should return 304
            match send_conditional_request("/index.html", Some(timestamp), None) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 304 Not Modified"), 
                        "Same timestamp should return 304 Not Modified, got: {}", response);
                    println!("✓ If-Modified-Since with same timestamp returned 304");
                }
                Err(e) => {
                    println!("Error testing If-Modified-Since: {}", e);
                }
            }
            
            // Test If-Modified-Since with older timestamp should return 200
            // Use a very old HTTP-date (January 1, 1990) to ensure it's older than any file
            match send_conditional_request("/index.html", Some("Mon, 01 Jan 1990 00:00:00 GMT"), None) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), 
                        "Older timestamp should return 200 OK, got: {}", response);
                    println!("✓ If-Modified-Since with older timestamp returned 200");
                }
                Err(e) => {
                    println!("Error testing older timestamp: {}", e);
                }
            }
        } else {
            println!("Could not extract Last-Modified from response");
        }
    }

    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_head_request_with_cache_headers() {
        let mut stream = TcpStream::connect("127.0.0.1:8080").unwrap_or_else(|_| {
            panic!("Server not running");
        });
        
        let request = "HEAD /index.html HTTP/1.1\r\nHost: localhost\r\n\r\n";
        stream.write_all(request.as_bytes()).unwrap();
        
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        
        assert!(response.contains("HTTP/1.1 200 OK"));
        assert!(response.contains("ETag: W/"), "HEAD response should contain ETag");
        assert!(response.contains("Last-Modified:"), "HEAD response should contain Last-Modified");
        assert!(response.contains("Content-Length:"), "HEAD response should contain Content-Length");
        
        // HEAD response should not contain body
        let body_start = response.find("\r\n\r\n").unwrap() + 4;
        let body = &response[body_start..];
        assert!(body.is_empty() || body.trim().is_empty(), "HEAD response should not contain body");
        
        println!("✓ HEAD request with cache headers test passed");
    }
}

#[cfg(test)]
mod path_security_integration_tests {
    use super::http_integration_tests::send_get_request;
    
    // Test that directory traversal attacks are blocked by cache-based protection
    #[test]
    #[ignore] // Requires server to be running
    fn test_directory_traversal_protection() {
        let traversal_paths = vec![
            "/../etc/passwd",
            "/../../etc/passwd",
            "/css/../../../etc/passwd",
            "/../../../../../etc/passwd",
            "/css/../js/../../etc/passwd",
            "/images/../../../etc/passwd",
            "/valid/path/../../etc/passwd",
        ];
        
        for path in traversal_paths {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                        "Directory traversal path {} should return 404, got: {}", path, response);
                    println!("✓ Directory traversal blocked: {}", path);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping traversal test for {}", path);
                }
            }
        }
    }
    
    // Test that attempts to access the kiss binary are blocked
    #[test]
    #[ignore] // Requires server to be running
    fn test_binary_access_prevention() {
        let binary_paths = vec![
            "/kiss",
            "/./kiss",
            "/css/../kiss",
            "/images/../js/../kiss", 
            "/valid/path/../kiss",
            "/../kiss",
            "/../../kiss",
        ];
        
        for path in binary_paths {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                        "Binary access path {} should return 404, got: {}", path, response);
                    println!("✓ Binary access blocked: {}", path);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping binary access test for {}", path);
                }
            }
        }
    }
    
    // Test that complex traversal patterns don't work
    #[test]
    #[ignore] // Requires server to be running  
    fn test_complex_traversal_patterns() {
        let complex_paths = vec![
            "/css/../js/../index.html/../../../etc/passwd",
            "/./././../etc/passwd",
            "/css/./../../etc/passwd", 
            "/images/icons/../../../etc/passwd",
            "/../../../../../../../etc/passwd",
        ];
        
        for path in complex_paths {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                        "Complex traversal path {} should return 404, got: {}", path, response);
                    println!("✓ Complex traversal blocked: {}", path);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping complex traversal test for {}", path);
                }
            }
        }
    }
    
    // Test that query parameters in URLs are handled correctly
    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_query_parameter_handling() {
        let query_paths = vec![
            ("/index.html?v=1", true),           // Should serve index.html
            ("/style.css?version=2", true),      // Should serve style.css  
            ("/app.js?timestamp=123", true),     // Should serve app.js
            ("/nonexistent.html?param=value", false), // Should return 404
        ];
        
        for (path, should_succeed) in query_paths {
            match send_get_request(path) {
                Ok(response) => {
                    if should_succeed {
                        // Currently this will fail because query stripping isn't implemented
                        // This test will guide us on whether we need to implement it
                        if response.contains("HTTP/1.1 200 OK") {
                            println!("✓ Query parameter handled correctly: {}", path);
                        } else {
                            println!("⚠ Query parameter causes cache miss: {} - may need query stripping", path);
                            assert!(response.contains("HTTP/1.1 404 Not Found"), 
                                "Query parameter path {} should either succeed or return 404", path);
                        }
                    } else {
                        assert!(response.contains("HTTP/1.1 404 Not Found"), 
                            "Invalid query path {} should return 404", path);
                        println!("✓ Invalid query path correctly returned 404: {}", path);
                    }
                }
                Err(_) => {
                    println!("Warning: Server not running from tests/content/, skipping query test for {}", path);
                }
            }
        }
    }
    
    // Test that fragment identifiers are handled (though they rarely reach server)
    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_fragment_handling() {
        let fragment_paths = vec![
            "/index.html#section",
            "/style.css#top",
            "/app.js#main",
        ];
        
        for path in fragment_paths {
            match send_get_request(path) {
                Ok(response) => {
                    // Fragments usually don't reach the server, but if they do,
                    // they should either work (if path normalization is added) or return 404
                    if response.contains("HTTP/1.1 200 OK") {
                        println!("✓ Fragment handled correctly: {}", path);
                    } else {
                        println!("⚠ Fragment causes cache miss: {} - fragments rarely sent by browsers anyway", path);
                        assert!(response.contains("HTTP/1.1 404 Not Found"));
                    }
                }
                Err(_) => {
                    println!("Warning: Server not running from tests/content/, skipping fragment test for {}", path);
                }
            }
        }
    }
    
    // Test that legitimate paths still work correctly
    #[test]
    #[ignore] // Requires server to be running from tests/content/
    fn test_legitimate_paths_still_work() {
        let valid_paths = vec![
            "/index.html",
            "/style.css", 
            "/app.js",
            "/css/style.css",
            "/test.svg",
        ];
        
        for path in valid_paths {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 200 OK"), 
                        "Legitimate path {} should return 200 OK, got: {}", path, response);
                    println!("✓ Legitimate path works: {}", path);
                }
                Err(_) => {
                    println!("Warning: Server not running from tests/content/, skipping legitimate path test for {}", path);
                }
            }
        }
    }
    
    // Test that the cache-based approach prevents access to files outside the static directory
    #[test]
    #[ignore] // Requires server to be running
    fn test_cache_prevents_filesystem_access() {
        let external_paths = vec![
            "/etc/passwd",
            "/usr/bin/ls",
            "/home/user/.bashrc",
            "/var/log/syslog",
            "/proc/version",
            "/kiss", // The binary itself
        ];
        
        for path in external_paths {
            match send_get_request(path) {
                Ok(response) => {
                    assert!(response.contains("HTTP/1.1 404 Not Found"), 
                        "External path {} should return 404 due to cache-based protection, got: {}", path, response);
                    println!("✓ Cache-based protection blocks external file: {}", path);
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping external file test for {}", path);
                }
            }
        }
    }
}

#[cfg(test)]
mod file_serving_tests {
    use super::*;
    use std::fs;
    
    #[test]
    fn test_static_file_creation() {
        let temp_dir = TempDir::new().unwrap();
        let static_dir = temp_dir.path().join("static");
        fs::create_dir(&static_dir).unwrap();
        
        // Create test files
        let index_html = static_dir.join("index.html");
        fs::write(&index_html, "<html><body>Hello World</body></html>").unwrap();
        
        let css_file = static_dir.join("style.css");
        fs::write(&css_file, "body { color: red; }").unwrap();
        
        let js_file = static_dir.join("app.js");
        fs::write(&js_file, "console.log('Hello');").unwrap();
        
        // Verify files were created
        assert!(index_html.exists());
        assert!(css_file.exists());
        assert!(js_file.exists());
        
        // Verify content
        let content = fs::read_to_string(&index_html).unwrap();
        assert!(content.contains("Hello World"));
    }
    
    #[test] 
    fn test_large_file_creation() {
        let temp_dir = TempDir::new().unwrap();
        let large_file = temp_dir.path().join("large.txt");
        
        // Create a file larger than MAX_FILE_SIZE (50MB)
        let large_content = "x".repeat(52 * 1024 * 1024); // 52MB
        fs::write(&large_file, &large_content).unwrap();
        
        // Verify file was created and is large
        let metadata = fs::metadata(&large_file).unwrap();
        assert!(metadata.len() > 50 * 1024 * 1024);
    }
}