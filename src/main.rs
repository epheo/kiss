use std::collections::HashMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crossbeam_channel::{bounded, Receiver};

const PORT: u16 = 8080;
const MAX_REQUEST_SIZE: usize = 8192;
const STATIC_DIR: &str = "/app/static";
const SHUTDOWN_TIMEOUT_SECS: u64 = 10;
const MAX_WORKER_THREADS: usize = 30;
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB

static SHUTDOWN_FLAG: AtomicBool = AtomicBool::new(false);

fn main() {
    let active_connections = Arc::new(AtomicUsize::new(0));
    
    // Setup signal handlers
    setup_signal_handlers();
    
    // Create worker pool
    let (sender, receiver) = bounded(MAX_WORKER_THREADS);
    let worker_handles = start_worker_pool(receiver, Arc::clone(&active_connections));
    
    let listener = TcpListener::bind(format!("0.0.0.0:{}", PORT))
        .expect("Failed to bind to address");
    
    // Set non-blocking mode for graceful shutdown
    listener.set_nonblocking(true)
        .expect("Failed to set non-blocking mode");
    
    println!("Server running on http://0.0.0.0:{} with {} workers", PORT, MAX_WORKER_THREADS);
    
    loop {
        if SHUTDOWN_FLAG.load(Ordering::Relaxed) {
            break;
        }
        
        match listener.accept() {
            Ok((stream, addr)) => {
                // Try to send to worker pool, drop connection if pool is full
                if sender.try_send(stream).is_err() {
                    // Pool is full, send 503 Service Unavailable
                    if let Ok(mut rejected_stream) = TcpStream::connect(addr) {
                        let _ = send_response(&mut rejected_stream, 503, "Service Unavailable", "text/plain", b"Server busy, try again later");
                    }
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(_) => continue,
        }
    }
    
    println!("Shutting down gracefully...");
    drop(sender); // Signal workers to stop
    graceful_shutdown(active_connections, worker_handles);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, signal_handler as libc::sighandler_t);
        libc::signal(libc::SIGINT, signal_handler as libc::sighandler_t);
    }
}

extern "C" fn signal_handler(_: libc::c_int) {
    SHUTDOWN_FLAG.store(true, Ordering::Relaxed);
}

fn start_worker_pool(receiver: Receiver<TcpStream>, active_connections: Arc<AtomicUsize>) -> Vec<thread::JoinHandle<()>> {
    let mut handles = Vec::with_capacity(MAX_WORKER_THREADS);
    
    for _ in 0..MAX_WORKER_THREADS {
        let receiver = receiver.clone();
        let connections = Arc::clone(&active_connections);
        
        let handle = thread::spawn(move || {
            while let Ok(stream) = receiver.recv() {
                connections.fetch_add(1, Ordering::Relaxed);
                handle_connection(stream);
                connections.fetch_sub(1, Ordering::Relaxed);
            }
        });
        
        handles.push(handle);
    }
    
    handles
}

fn graceful_shutdown(active_connections: Arc<AtomicUsize>, worker_handles: Vec<thread::JoinHandle<()>>) {
    let start = std::time::Instant::now();
    
    // Wait for active connections to finish
    while active_connections.load(Ordering::Relaxed) > 0 {
        if start.elapsed().as_secs() > SHUTDOWN_TIMEOUT_SECS {
            println!("Shutdown timeout reached, forcing exit");
            break;
        }
        
        println!("Waiting for {} active connections to finish", 
                active_connections.load(Ordering::Relaxed));
        thread::sleep(Duration::from_millis(500));
    }
    
    // Wait for worker threads to finish
    for handle in worker_handles {
        let _ = handle.join();
    }
    
    println!("Server shutdown complete");
}

fn handle_connection(mut stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    
    if request_line.len() > MAX_REQUEST_SIZE {
        send_response(&mut stream, 413, "Request Entity Too Large", "text/plain", b"Request too large");
        return;
    }
    
    let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
    if parts.len() < 3 || parts[0] != "GET" {
        send_response(&mut stream, 405, "Method Not Allowed", "text/plain", b"Method not allowed");
        return;
    }
    
    let path = parts[1];
    
    // Handle health check endpoints
    if path == "/health" {
        send_health_response(&mut stream);
        return;
    }
    
    if path == "/ready" {
        send_ready_response(&mut stream);
        return;
    }
    
    serve_static_file(&mut stream, path);
}

fn serve_static_file(stream: &mut TcpStream, path: &str) {
    let sanitized_path = sanitize_path(path);
    let file_path = if sanitized_path == "/" {
        format!("{}/index.html", STATIC_DIR)
    } else {
        format!("{}{}", STATIC_DIR, sanitized_path)
    };
    
    // Check file size before reading
    match fs::metadata(&file_path) {
        Ok(metadata) => {
            if metadata.len() > MAX_FILE_SIZE {
                send_response(stream, 413, "Payload Too Large", "text/plain", b"File too large");
                return;
            }
            
            match fs::read(&file_path) {
                Ok(contents) => {
                    let mime_type = get_mime_type(&file_path);
                    send_response(stream, 200, "OK", &mime_type, &contents);
                }
                Err(_) => {
                    send_response(stream, 404, "Not Found", "text/plain", b"File not found");
                }
            }
        }
        Err(_) => {
            send_response(stream, 404, "Not Found", "text/plain", b"File not found");
        }
    }
}

fn sanitize_path(path: &str) -> String {
    let path = path.split('?').next().unwrap_or(path);
    let path = path.split('#').next().unwrap_or(path);
    
    let normalized = Path::new(path)
        .components()
        .filter_map(|component| {
            match component {
                std::path::Component::Normal(s) => s.to_str(),
                std::path::Component::RootDir => Some("/"),
                _ => None,
            }
        })
        .collect::<Vec<_>>()
        .join("/");
    
    if normalized.is_empty() || normalized == "/" {
        "/".to_string()
    } else if normalized.starts_with('/') {
        normalized
    } else {
        format!("/{}", normalized)
    }
}

fn get_mime_type(file_path: &str) -> String {
    let mime_types: HashMap<&str, &str> = [
        ("html", "text/html; charset=utf-8"),
        ("htm", "text/html; charset=utf-8"),
        ("css", "text/css; charset=utf-8"),
        ("js", "text/javascript; charset=utf-8"),
        ("json", "application/json; charset=utf-8"),
        ("xml", "application/xml; charset=utf-8"),
        ("txt", "text/plain; charset=utf-8"),
        ("ico", "image/x-icon"),
        ("png", "image/png"),
        ("jpg", "image/jpeg"),
        ("jpeg", "image/jpeg"),
        ("gif", "image/gif"),
        ("svg", "image/svg+xml"),
        ("pdf", "application/pdf"),
        ("woff", "font/woff"),
        ("woff2", "font/woff2"),
        ("ttf", "font/ttf"),
        ("eot", "application/vnd.ms-fontobject"),
    ].iter().cloned().collect();
    
    if let Some(extension) = Path::new(file_path).extension().and_then(|s| s.to_str()) {
        mime_types.get(extension.to_lowercase().as_str())
            .unwrap_or(&"application/octet-stream")
            .to_string()
    } else {
        "application/octet-stream".to_string()
    }
}

fn send_response(stream: &mut TcpStream, status_code: u16, status_text: &str, content_type: &str, body: &[u8]) {
    let security_headers = [
        "X-Frame-Options: DENY",
        "X-Content-Type-Options: nosniff",
        "X-XSS-Protection: 1; mode=block",
        "Referrer-Policy: strict-origin-when-cross-origin",
        "Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; script-src 'self' 'unsafe-inline' https://duckduckgo.com"
    ].join("\r\n");
    
    let response = format!(
        "HTTP/1.1 {} {}\r\n{}\r\nContent-Type: {}\r\nContent-Length: {}\r\n\r\n",
        status_code, status_text, security_headers, content_type, body.len()
    );
    
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn send_health_response(stream: &mut TcpStream) {
    let health_status = r#"{"status":"healthy","timestamp":"#.to_string() + 
        &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string() + 
        r#"}"#;
    
    send_response(stream, 200, "OK", "application/json", health_status.as_bytes());
}

fn send_ready_response(stream: &mut TcpStream) {
    let ready_status = r#"{"status":"ready","timestamp":"#.to_string() + 
        &std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs().to_string() + 
        r#"}"#;
    
    send_response(stream, 200, "OK", "application/json", ready_status.as_bytes());
}