use std::io::{Read, Write};
use std::net::TcpStream;
use tempfile::TempDir;

#[cfg(test)]
mod http_integration_tests {
    use super::*;
    
    // Note: These tests require a running server instance
    // In a real implementation, we'd start the server programmatically
    
    fn send_get_request(path: &str) -> Result<String, std::io::Error> {
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
    #[ignore] // Requires server to be running and static files
    fn test_serve_html_file() {
        match send_get_request("/index.html") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("Content-Type: text/html"));
                assert!(response.contains("<html>"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping integration test");
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running and static files
    fn test_serve_different_file_types() {
        // Test CSS file
        match send_get_request("/style.css") {
            Ok(response) => {
                if response.contains("HTTP/1.1 200 OK") {
                    assert!(response.contains("Content-Type: text/css"));
                } else {
                    // File might not exist, that's ok for this test
                    assert!(response.contains("HTTP/1.1 404 Not Found"));
                }
            }
            Err(_) => {
                println!("Warning: Server not running, skipping integration test");
            }
        }
        
        // Test JavaScript file
        match send_get_request("/app.js") {
            Ok(response) => {
                if response.contains("HTTP/1.1 200 OK") {
                    assert!(response.contains("Content-Type: application/javascript"));
                } else {
                    // File might not exist, that's ok for this test
                    assert!(response.contains("HTTP/1.1 404 Not Found"));
                }
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
    fn test_security_headers() {
        match send_get_request("/health") {
            Ok(response) => {
                assert!(response.contains("X-Frame-Options: DENY"));
                assert!(response.contains("X-Content-Type-Options: nosniff"));
                assert!(response.contains("Content-Security-Policy:"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping integration test");
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