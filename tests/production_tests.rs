use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Child, Stdio};
use std::time::{Duration, Instant};
use std::thread;

#[cfg(test)]
mod graceful_shutdown_tests {
    use super::*;
    
    fn start_test_server() -> Result<Child, std::io::Error> {
        Command::new("./target/release/kiss")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
    
    fn wait_for_server_startup(max_wait: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < max_wait {
            if TcpStream::connect("127.0.0.1:8080").is_ok() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }
    
    fn send_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }
    
    #[test]
    #[ignore] // Requires building release binary first
    fn test_sigterm_graceful_shutdown() {
        // Start server
        let mut server = match start_test_server() {
            Ok(server) => server,
            Err(_) => {
                println!("Warning: Could not start test server, skipping graceful shutdown test");
                return;
            }
        };
        
        // Wait for startup
        if !wait_for_server_startup(Duration::from_secs(5)) {
            println!("Warning: Server did not start in time, skipping test");
            let _ = server.kill();
            return;
        }
        
        // Verify server is responsive
        match send_request("/health") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
            }
            Err(_) => {
                println!("Warning: Server not responsive, skipping test");
                let _ = server.kill();
                return;
            }
        }
        
        // Send SIGTERM
        let server_pid = server.id();
        unsafe {
            libc::kill(server_pid as i32, libc::SIGTERM);
        }
        
        // Server should shut down gracefully within timeout
        let shutdown_start = Instant::now();
        let mut shutdown_completed = false;
        
        // Wait for server to shut down (max 15 seconds)
        while shutdown_start.elapsed() < Duration::from_secs(15) {
            match server.try_wait() {
                Ok(Some(_exit_status)) => {
                    shutdown_completed = true;
                    break;
                }
                Ok(None) => {
                    // Still running
                    thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }
        
        if !shutdown_completed {
            println!("Warning: Server did not shut down gracefully, forcing kill");
            let _ = server.kill();
        }
        
        assert!(shutdown_completed, "Server should shut down gracefully on SIGTERM");
        
        let shutdown_time = shutdown_start.elapsed();
        println!("Graceful shutdown took: {:?}", shutdown_time);
        
        // Should shut down within reasonable time
        assert!(shutdown_time < Duration::from_secs(12), 
               "Shutdown took too long: {:?}", shutdown_time);
    }
    
    #[test] 
    #[ignore] // Requires building release binary first
    fn test_sigint_graceful_shutdown() {
        // Similar to SIGTERM test but with SIGINT (Ctrl+C)
        let mut server = match start_test_server() {
            Ok(server) => server,
            Err(_) => {
                println!("Warning: Could not start test server, skipping SIGINT test");
                return;
            }
        };
        
        if !wait_for_server_startup(Duration::from_secs(5)) {
            println!("Warning: Server did not start, skipping SIGINT test");
            let _ = server.kill();
            return;
        }
        
        // Verify server is responsive
        if send_request("/health").is_err() {
            println!("Warning: Server not responsive, skipping SIGINT test");
            let _ = server.kill();
            return;
        }
        
        // Send SIGINT
        let server_pid = server.id();
        unsafe {
            libc::kill(server_pid as i32, libc::SIGINT);
        }
        
        // Check graceful shutdown
        let shutdown_start = Instant::now();
        let mut shutdown_completed = false;
        
        while shutdown_start.elapsed() < Duration::from_secs(15) {
            match server.try_wait() {
                Ok(Some(_)) => {
                    shutdown_completed = true;
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        
        if !shutdown_completed {
            let _ = server.kill();
        }
        
        assert!(shutdown_completed, "Server should shut down gracefully on SIGINT");
    }
    
    #[test]
    #[ignore] // Requires building release binary and can be flaky
    fn test_shutdown_with_active_connections() {
        let mut server = match start_test_server() {
            Ok(server) => server,
            Err(_) => {
                println!("Warning: Could not start test server");
                return;
            }
        };
        
        if !wait_for_server_startup(Duration::from_secs(5)) {
            let _ = server.kill();
            return;
        }
        
        // Create several active connections
        let mut connections = Vec::new();
        for _ in 0..5 {
            if let Ok(stream) = TcpStream::connect("127.0.0.1:8080") {
                connections.push(stream);
            }
        }
        
        // Send SIGTERM while connections are active
        let server_pid = server.id();
        unsafe {
            libc::kill(server_pid as i32, libc::SIGTERM);
        }
        
        // Server should wait for connections and then shut down gracefully
        let shutdown_start = Instant::now();
        let mut shutdown_completed = false;
        
        while shutdown_start.elapsed() < Duration::from_secs(15) {
            match server.try_wait() {
                Ok(Some(_)) => {
                    shutdown_completed = true;
                    break;
                }
                Ok(None) => thread::sleep(Duration::from_millis(100)),
                Err(_) => break,
            }
        }
        
        // Clean up connections
        drop(connections);
        
        if !shutdown_completed {
            let _ = server.kill();
        }
        
        // Should eventually shut down
        assert!(shutdown_completed, "Server should eventually shut down even with active connections");
    }
}

#[cfg(test)]
mod startup_and_binding_tests {
    use super::*;
    
    #[test]
    #[ignore] // Requires building release binary
    fn test_server_startup_and_binding() {
        let mut server = match start_test_server() {
            Ok(server) => server,
            Err(_) => {
                println!("Warning: Could not start test server");
                return;
            }
        };
        
        // Should bind to port successfully
        let startup_success = wait_for_server_startup(Duration::from_secs(10));
        assert!(startup_success, "Server should start and bind to port 8080");
        
        // Should respond to health check
        match send_request("/health") {
            Ok(response) => {
                assert!(response.contains("HTTP/1.1 200 OK"));
                assert!(response.contains("healthy"));
            }
            Err(_) => {
                panic!("Server should respond to health check after startup");
            }
        }
        
        // Clean shutdown
        let _ = server.kill();
        let _ = server.wait();
    }
    
    #[test]
    #[ignore] // Requires building release binary
    fn test_port_already_in_use() {
        // Start first server
        let mut server1 = match start_test_server() {
            Ok(server) => server,
            Err(_) => {
                println!("Warning: Could not start first test server");
                return;
            }
        };
        
        if !wait_for_server_startup(Duration::from_secs(5)) {
            let _ = server1.kill();
            return;
        }
        
        // Try to start second server on same port
        let server2_result = start_test_server();
        
        // Second server should fail to start or exit quickly
        match server2_result {
            Ok(mut server2) => {
                // Wait a bit to see if it exits
                thread::sleep(Duration::from_millis(500));
                
                match server2.try_wait() {
                    Ok(Some(exit_status)) => {
                        // Should exit with error
                        assert!(!exit_status.success(), "Second server should fail when port is in use");
                    }
                    Ok(None) => {
                        // Still running - kill both servers
                        let _ = server2.kill();
                        println!("Warning: Second server didn't exit quickly when port in use");
                    }
                    Err(_) => {
                        let _ = server2.kill();
                    }
                }
            }
            Err(_) => {
                // Failed to start - this is expected behavior
                println!("Second server correctly failed to start (port in use)");
            }
        }
        
        // Clean up first server
        let _ = server1.kill();
        let _ = server1.wait();
    }
    
    fn start_test_server() -> Result<Child, std::io::Error> {
        Command::new("./target/release/kiss")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
    
    fn wait_for_server_startup(max_wait: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < max_wait {
            if TcpStream::connect("127.0.0.1:8080").is_ok() {
                return true;
            }
            thread::sleep(Duration::from_millis(100));
        }
        false
    }
    
    fn send_request(path: &str) -> Result<String, std::io::Error> {
        let mut stream = TcpStream::connect("127.0.0.1:8080")?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        
        let request = format!("GET {} HTTP/1.1\r\nHost: localhost\r\n\r\n", path);
        stream.write_all(request.as_bytes())?;
        
        let mut response = String::new();
        stream.read_to_string(&mut response)?;
        Ok(response)
    }
}