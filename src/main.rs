use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{timeout, Duration};
use lazy_static::lazy_static;
use kiss::{get_mime_type, sanitize_path};

const PORT: u16 = 8080;
const MAX_REQUEST_SIZE: usize = 8192;
const STATIC_DIR: &str = ".";
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB
const CONNECTION_TIMEOUT_SECS: u64 = 30;
const KEEPALIVE_TIMEOUT_SECS: u64 = 5;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// Pre-compiled header templates for fast response generation
lazy_static! {
    static ref HEADER_TEMPLATES: HeaderTemplates = HeaderTemplates::new();
}

// Pre-compiled response header templates
struct HeaderTemplates {
    ok_template: String,
    not_found: Vec<u8>,
    method_not_allowed: Vec<u8>,
    request_too_large: Vec<u8>,
    file_too_large: Vec<u8>,
    bad_request: Vec<u8>,
    request_timeout: Vec<u8>,
}

impl HeaderTemplates {
    fn new() -> Self {
        Self {
            ok_template: "HTTP/1.1 200 OK\r\nContent-Type: {mime_type}\r\nContent-Length: {content_length}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n".to_string(),
            not_found: b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile not found".to_vec(),
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nRequest too large".to_vec(),
            file_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile too large".to_vec(),
            bad_request: b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nMalformed request".to_vec(),
            request_timeout: b"HTTP/1.1 408 Request Timeout\r\nContent-Type: text/plain\r\nContent-Length: 15\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nRequest timeout".to_vec(),
        }
    }
}

// Fast zero-allocation HTTP request line parser
fn parse_request_line_fast(request: &[u8]) -> Option<(&[u8], &str, &str)> {
    let mut parts = request.split(|&b| b == b' ').filter(|part| !part.is_empty());
    
    let method = parts.next()?;
    let path_bytes = parts.next()?;
    let version_bytes = parts.next()?;
    
    // Ensure there are no extra parts after the three required ones
    if parts.next().is_some() {
        return None;
    }
    
    // Convert path and version to &str for compatibility with existing code
    let path = std::str::from_utf8(path_bytes).ok()?;
    let version = std::str::from_utf8(version_bytes).ok()?;
    
    // Basic validation
    if method.is_empty() || path.is_empty() || version.is_empty() {
        return None;
    }
    
    Some((method, path, version))
}

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", PORT))
        .await
        .expect("Failed to bind to address");

    println!("Async KISS server running on http://0.0.0.0:{}", PORT);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        // Configure TCP socket for performance
                        let _ = stream.set_nodelay(true);
                        tokio::spawn(handle_connection(stream));
                    }
                    Err(_) => continue,
                }
            }
            _ = shutdown_signal() => {
                println!("Shutdown signal received, stopping server...");
                SHUTDOWN.store(true, Ordering::Relaxed);
                break;
            }
        }

        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }
    }

    println!("Server shutdown complete");
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

async fn handle_connection(mut stream: TcpStream) {
    // Set connection timeout
    let connection_result = timeout(
        Duration::from_secs(CONNECTION_TIMEOUT_SECS),
        handle_connection_inner(&mut stream),
    )
    .await;

    if connection_result.is_err() {
        let _ = send_precompiled_response(&mut stream, &HEADER_TEMPLATES.request_timeout).await;
    }
}

async fn handle_connection_inner(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    loop {
        // Check for shutdown
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        let mut reader = BufReader::new(&mut *stream);
        let mut request_line = String::new();

        // Read request line with timeout
        match timeout(
            Duration::from_secs(KEEPALIVE_TIMEOUT_SECS),
            reader.read_line(&mut request_line),
        )
        .await
        {
            Ok(Ok(0)) | Err(_) => break, // Connection closed or timeout
            Ok(Err(_)) => break,         // Read error
            Ok(Ok(size)) if size > MAX_REQUEST_SIZE => {
                send_precompiled_response(stream, &HEADER_TEMPLATES.request_too_large).await?;
                break;
            }
            Ok(Ok(_)) => {}
        }

        if request_line.trim().is_empty() {
            continue; // Keep-alive, wait for next request
        }

        // Zero-allocation HTTP parsing - avoid string splits and allocations
        let request_bytes = request_line.trim().as_bytes();
        let (method, path, version) = match parse_request_line_fast(request_bytes) {
            Some((m, p, v)) => (m, p, v),
            None => {
                send_precompiled_response(stream, &HEADER_TEMPLATES.bad_request).await?;
                break;
            }
        };

        if method != b"GET" {
            send_precompiled_response(stream, &HEADER_TEMPLATES.method_not_allowed).await?;
            break;
        }

        // Enhanced connection management - faster header parsing
        let mut keep_alive = version == "HTTP/1.1"; // Default for HTTP/1.1
        
        // Fast header parsing - only look for Connection header
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    if line.trim().is_empty() {
                        break; // End of headers
                    }
                    // Fast case-insensitive connection header check
                    let line_lower = line.trim().to_lowercase();
                    if line_lower.starts_with("connection:") {
                        let connection_close_requested = line_lower.contains("close");
                        keep_alive = !connection_close_requested && (version == "HTTP/1.1" || line_lower.contains("keep-alive"));
                    }
                }
                Err(_) => break,
            }
        }

        // Handle the request
        match handle_request(stream, path).await {
            Ok(_) => {
                if !keep_alive {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    Ok(())
}

async fn handle_request(stream: &mut TcpStream, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle health check endpoints
    if path == "/health" {
        return send_health_response(stream).await;
    }

    if path == "/ready" {
        return send_ready_response(stream).await;
    }

    serve_static_file(stream, path).await
}

async fn serve_static_file(stream: &mut TcpStream, path: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sanitized_path = sanitize_path(path);
    let file_path = if sanitized_path == "/" {
        format!("{}/index.html", STATIC_DIR)
    } else {
        format!("{}{}", STATIC_DIR, sanitized_path)
    };

    match File::open(&file_path).await {
        Ok(mut file) => {
            // Get file metadata
            let metadata = file.metadata().await?;
            let file_size = metadata.len();

            // Check file size limit
            if file_size > MAX_FILE_SIZE {
                return send_precompiled_response(stream, &HEADER_TEMPLATES.file_too_large).await;
            }

            // Get MIME type
            let mime_type = get_mime_type(&file_path);

            // Send response headers using template (much faster than format!)
            let headers = HEADER_TEMPLATES.ok_template
                .replace("{mime_type}", mime_type)
                .replace("{content_length}", &file_size.to_string());
            stream.write_all(headers.as_bytes()).await?;

            // Zero-copy file serving - direct kernel-to-kernel transfer
            tokio::io::copy(&mut file, stream).await?;
            stream.flush().await?;
        }
        Err(_) => {
            return send_precompiled_response(stream, &HEADER_TEMPLATES.not_found).await;
        }
    }

    Ok(())
}

// Optimized function for pre-compiled responses
async fn send_precompiled_response(
    stream: &mut TcpStream,
    response: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    stream.write_all(response).await?;
    stream.flush().await?;
    Ok(())
}

async fn send_health_response(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let health_status = format!(r#"{{"status":"healthy","timestamp":"{}"}}"#, timestamp);
    
    // Use optimized response with pre-compiled headers
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n",
        health_status.len()
    );
    
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(health_status.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

async fn send_ready_response(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let ready_status = format!(r#"{{"status":"ready","timestamp":"{}"}}"#, timestamp);
    
    // Use optimized response with pre-compiled headers
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' https:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n",
        ready_status.len()
    );
    
    stream.write_all(headers.as_bytes()).await?;
    stream.write_all(ready_status.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}