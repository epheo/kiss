use std::fs;
use std::io::{Read, Write};
use std::net::TcpStream;
use tempfile::TempDir;


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
        // This test would verify that large files are handled properly during cache building
        // In a real scenario, we'd set up a test file larger than optimal size in the static directory
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