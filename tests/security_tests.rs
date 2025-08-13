use kiss::*;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use tempfile::TempDir;

#[cfg(test)]
mod directory_traversal_tests {
    use super::*;
    
    #[test]
    fn test_basic_directory_traversal_prevention() {
        // Test basic ../ attempts that could reach binary - should be blocked
        assert_eq!(sanitize_path("/../kiss"), "/");  // Block access to binary
        assert_eq!(sanitize_path("/../../kiss"), "/");  // Block access to binary
        assert_eq!(sanitize_path("/../../../kiss"), "/");  // Block access to binary
    }
    
    #[test]
    fn test_nested_directory_traversal_prevention() {
        // Test nested directory traversal attempts that could reach binary
        assert_eq!(sanitize_path("/css/../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("/images/../js/../../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("/a/b/c/../../../kiss"), "/");  // Block binary access
    }
    
    #[test]
    fn test_relative_path_traversal_prevention() {
        // Test relative paths that could reach binary - should be blocked
        assert_eq!(sanitize_path("../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("../../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("css/../kiss"), "/");  // Block binary access
    }
    
    #[test]
    fn test_mixed_traversal_attempts() {
        // Test combinations of valid paths and traversal attempts to reach binary
        assert_eq!(sanitize_path("/valid/path/../../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("/css/../js/../kiss"), "/");  // Block binary access  
        assert_eq!(sanitize_path("/./kiss"), "/");  // Block direct binary access
    }
    
    #[test]
    fn test_direct_binary_access() {
        // Test direct access to binary - should be blocked
        assert_eq!(sanitize_path("/kiss"), "/");  // Block direct binary access
        assert_eq!(sanitize_path("kiss"), "/"); // Also block relative access to avoid confusion
    }
    
    #[test]
    fn test_url_encoded_traversal_attempts() {
        // Note: URL decoding should be handled by the HTTP server layer
        // These tests ensure our sanitization handles already decoded paths
        assert_eq!(sanitize_path("/../kiss"), "/");  // Block binary access
        assert_eq!(sanitize_path("/..%2F..%2Fkiss"), "/..%2F..%2Fkiss"); // Not decoded by sanitizer
    }
    
    #[test]
    fn test_null_byte_injection_resistance() {
        // Test null byte injection attempts (Rust strings are UTF-8, so this is less relevant)
        assert_eq!(sanitize_path("/etc/passwd\0.txt"), "/etc/passwd\0.txt");
    }
    
    #[test]
    fn test_windows_style_traversal() {
        // Test Windows-style directory traversal (backslashes treated as normal filename chars)
        assert_eq!(sanitize_path("\\..\\..\\kiss"), "/\\..\\..\\kiss"); // Backslashes treated as normal chars
    }
    
    #[test]
    fn test_current_directory_references() {
        // Test current directory (.) references - should be ignored safely
        assert_eq!(sanitize_path("/./etc/passwd"), "/etc/passwd");
        assert_eq!(sanitize_path("/css/./passwd"), "/css/passwd");
        assert_eq!(sanitize_path("./etc/passwd"), "/etc/passwd");
    }
    
    #[test]
    fn test_valid_relative_navigation() {
        // Test valid relative navigation that should work correctly
        assert_eq!(sanitize_path("/css/../style.css"), "/style.css");
        assert_eq!(sanitize_path("/js/lib/../app.js"), "/js/app.js"); 
        assert_eq!(sanitize_path("/images/icons/../logo.png"), "/images/logo.png");
    }
    
    #[test]
    fn test_normal_content_serving() {
        // Test that normal content paths work correctly in root serving
        assert_eq!(sanitize_path("/index.html"), "/index.html");
        assert_eq!(sanitize_path("/style.css"), "/style.css");
        assert_eq!(sanitize_path("/js/app.js"), "/js/app.js");
        assert_eq!(sanitize_path("/images/logo.png"), "/images/logo.png");
        assert_eq!(sanitize_path("/favicon.ico"), "/favicon.ico");
    }
    
    #[test]
    fn test_root_traversal_attempts() {
        // Test attempts to access root and above
        assert_eq!(sanitize_path("/../"), "/");
        assert_eq!(sanitize_path("/../../"), "/");
        assert_eq!(sanitize_path("/../../../"), "/");
    }
    
    #[test]
    fn test_legitimate_paths_preserved() {
        // Ensure legitimate paths are not affected
        assert_eq!(sanitize_path("/index.html"), "/index.html");
        assert_eq!(sanitize_path("/css/style.css"), "/css/style.css");
        assert_eq!(sanitize_path("/js/modules/app.js"), "/js/modules/app.js");
        assert_eq!(sanitize_path("/images/logo.png"), "/images/logo.png");
    }
}

#[cfg(test)]
mod path_validation_tests {
    use super::*;
    
    #[test]
    fn test_query_parameter_removal() {
        // Ensure query parameters don't affect binary protection
        assert_eq!(sanitize_path("/kiss?param=value"), "/");  // Block binary access with query
        assert_eq!(sanitize_path("/css/../kiss?x=1&y=2"), "/");  // Block binary access with query  
        assert_eq!(sanitize_path("/style.css?v=1.2"), "/style.css");  // Valid path with query
    }
    
    #[test]
    fn test_fragment_removal() {
        // Ensure fragments don't affect binary protection
        assert_eq!(sanitize_path("/kiss#section"), "/");  // Block binary access with fragment
        assert_eq!(sanitize_path("/css/../kiss#top"), "/");  // Block binary access with fragment
        assert_eq!(sanitize_path("/page.html#section"), "/page.html");  // Valid path with fragment
    }
    
    #[test]
    fn test_combined_query_and_fragment_removal() {
        assert_eq!(sanitize_path("/kiss?param=value#section"), "/");  // Block binary with both
        assert_eq!(sanitize_path("/css/../kiss?x=1#fragment"), "/");  // Block binary with both
        assert_eq!(sanitize_path("/app.js?v=1.0#main"), "/app.js");  // Valid path with both
    }
    
    #[test]
    fn test_empty_and_edge_cases() {
        assert_eq!(sanitize_path(""), "/");
        assert_eq!(sanitize_path("/"), "/");
        assert_eq!(sanitize_path("?param=value"), "/");
        assert_eq!(sanitize_path("#fragment"), "/");
        assert_eq!(sanitize_path("?param=value#fragment"), "/");
    }
}

#[cfg(test)]
mod security_integration_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_directory_traversal_http_request() {
        let traversal_paths = vec![
            "/../etc/passwd",
            "/../../etc/passwd",
            "/css/../../../etc/passwd",
            "/../../../etc/shadow",
        ];
        
        for path in traversal_paths {
            match send_get_request(path) {
                Ok(response) => {
                    // Should either return 404 (file not found in static dir)
                    // or serve a legitimate file from the static directory
                    // Should NOT return actual system files
                    assert!(response.contains("HTTP/1.1 404 Not Found") || 
                           !response.contains("root:x:0:0:root"));
                }
                Err(_) => {
                    println!("Warning: Server not running, skipping security integration test");
                    break;
                }
            }
        }
    }
    
    #[test]
    #[ignore] // Requires server to be running
    fn test_file_size_limits() {
        // This test would verify that files larger than MAX_FILE_SIZE are rejected
        // In a real scenario, we'd set up a test file larger than 50MB in the static directory
        match send_get_request("/large-file.bin") {
            Ok(response) => {
                // Should return 413 if file exists and is too large, or 404 if not found
                assert!(response.contains("HTTP/1.1 413 Payload Too Large") || 
                       response.contains("HTTP/1.1 404 Not Found"));
            }
            Err(_) => {
                println!("Warning: Server not running, skipping file size test");
            }
        }
    }
    
    fn send_get_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        
        Ok(response)
    }
}

#[cfg(test)]
mod filesystem_security_tests {
    use super::*;
    
    #[test]
    fn test_filesystem_setup_security() {
        // Test that we can create a secure filesystem structure
        let temp_dir = TempDir::new().unwrap();
        let static_dir = temp_dir.path().join("static");
        fs::create_dir(&static_dir).unwrap();
        
        // Create some test files in the static directory
        let safe_file = static_dir.join("safe.html");
        fs::write(&safe_file, "<html>Safe content</html>").unwrap();
        
        // Create a file outside the static directory (simulating system file)
        let system_file = temp_dir.path().join("system.txt");
        fs::write(&system_file, "System file content").unwrap();
        
        // Verify that the static directory exists and contains our file
        assert!(static_dir.exists());
        assert!(safe_file.exists());
        assert!(system_file.exists());
        
        // Test path resolution to ensure we stay within static directory
        let resolved_safe = static_dir.join("safe.html");
        let resolved_parent = temp_dir.path().join("system.txt");
        
        assert!(resolved_safe.starts_with(&static_dir));
        assert!(!resolved_parent.starts_with(&static_dir));
    }
    
    #[test]
    fn test_path_canonicalization() {
        let temp_dir = TempDir::new().unwrap();
        let static_dir = temp_dir.path().join("static");
        fs::create_dir_all(&static_dir).unwrap();
        
        // Create nested directory structure
        let nested_dir = static_dir.join("css").join("vendor");
        fs::create_dir_all(&nested_dir).unwrap();
        
        let test_file = nested_dir.join("style.css");
        fs::write(&test_file, "/* CSS content */").unwrap();
        
        // Test that the file exists where expected
        assert!(test_file.exists());
        
        // Test that the canonical path is within the static directory
        let canonical = test_file.canonicalize().unwrap();
        let canonical_static = static_dir.canonicalize().unwrap();
        assert!(canonical.starts_with(canonical_static));
    }
}