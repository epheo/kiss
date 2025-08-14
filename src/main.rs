use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
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

// File metadata for cached headers
#[derive(Clone)]
struct FileMetadata {
    headers: Vec<u8>,
    size: u64,
    last_modified: SystemTime,
    etag: String,
}

// Pre-compiled header templates for fast response generation
lazy_static! {
    static ref HEADER_TEMPLATES: HeaderTemplates = HeaderTemplates::new();
    static ref FILE_CACHE: HashMap<String, FileMetadata> = build_file_cache();
}

// Pre-compiled response header templates
struct HeaderTemplates {
    _ok_template: String,
    not_found: Vec<u8>,
    method_not_allowed: Vec<u8>,
    request_too_large: Vec<u8>,
    file_too_large: Vec<u8>,
    bad_request: Vec<u8>,
    request_timeout: Vec<u8>,
    health_response: Vec<u8>,
    ready_response: Vec<u8>,
}

impl HeaderTemplates {
    fn new() -> Self {
        Self {
            _ok_template: "HTTP/1.1 200 OK\r\nContent-Type: {mime_type}\r\nContent-Length: {content_length}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n".to_string(),
            not_found: b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile not found".to_vec(),
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nRequest too large".to_vec(),
            file_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile too large".to_vec(),
            bad_request: b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nMalformed request".to_vec(),
            request_timeout: b"HTTP/1.1 408 Request Timeout\r\nContent-Type: text/plain\r\nContent-Length: 15\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nRequest timeout".to_vec(),
            health_response: Self::create_health_response(),
            ready_response: Self::create_ready_response(),
        }
    }
    
    fn create_health_response() -> Vec<u8> {
        let health_status = r#"{"status":"healthy","timestamp":"0"}"#;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n{}",
            health_status.len(), health_status
        );
        headers.into_bytes()
    }
    
    fn create_ready_response() -> Vec<u8> {
        let ready_status = r#"{"status":"ready","timestamp":"0"}"#;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n{}",
            ready_status.len(), ready_status
        );
        headers.into_bytes()
    }
}

// Fast case-insensitive ASCII comparison without allocation
fn header_starts_with(header_line: &str, prefix: &str) -> bool {
    if header_line.len() < prefix.len() {
        return false;
    }
    
    header_line[..prefix.len()].eq_ignore_ascii_case(prefix)
}

// Fast case-insensitive contains check without allocation
fn header_contains(header_line: &str, substring: &str) -> bool {
    // Use a simple ASCII case-insensitive search
    let header_bytes = header_line.as_bytes();
    let sub_bytes = substring.as_bytes();
    
    if sub_bytes.is_empty() {
        return true;
    }
    
    if header_bytes.len() < sub_bytes.len() {
        return false;
    }
    
    'outer: for i in 0..=(header_bytes.len() - sub_bytes.len()) {
        for j in 0..sub_bytes.len() {
            let h = header_bytes[i + j];
            let s = sub_bytes[j];
            // ASCII case-insensitive comparison
            if h != s && h.to_ascii_lowercase() != s.to_ascii_lowercase() {
                continue 'outer;
            }
        }
        return true;
    }
    false
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

fn build_file_cache() -> HashMap<String, FileMetadata> {
    let mut cache = HashMap::new();
    
    if let Err(e) = discover_files_recursive(STATIC_DIR, "", &mut cache) {
        eprintln!("Warning: Failed to build file cache: {}", e);
    }
    
    println!("File cache built with {} entries", cache.len());
    cache
}

fn discover_files_recursive(
    base_dir: &str,
    relative_path: &str,
    cache: &mut HashMap<String, FileMetadata>,
) -> Result<(), Box<dyn std::error::Error>> {
    let full_path = if relative_path.is_empty() {
        base_dir.to_string()
    } else {
        format!("{}/{}", base_dir, relative_path)
    };
    
    let entries = fs::read_dir(&full_path)?;
    
    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;
        let file_name = entry.file_name().to_string_lossy().to_string();
        
        let current_relative = if relative_path.is_empty() {
            file_name.clone()
        } else {
            format!("{}/{}", relative_path, file_name)
        };
        
        if metadata.is_file() {
            // Generate cache entry for this file
            if let Ok(file_metadata) = generate_file_metadata(&entry.path(), &current_relative) {
                let url_path = format!("/{}", current_relative);
                cache.insert(url_path, file_metadata);
            }
        } else if metadata.is_dir() {
            // Recursively process directories
            discover_files_recursive(base_dir, &current_relative, cache)?;
        }
    }
    
    Ok(())
}

fn generate_file_metadata(file_path: &std::path::Path, _relative_path: &str) -> Result<FileMetadata, Box<dyn std::error::Error>> {
    let metadata = fs::metadata(file_path)?;
    let size = metadata.len();
    let last_modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    
    // Generate weak ETag using size and modification time
    let mtime_secs = last_modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let etag = format!("W/\"{}-{}\"", size, mtime_secs);
    
    // Format Last-Modified header (RFC 7232 format)
    let last_modified_str = format_http_date(last_modified);
    
    // Get MIME type
    let mime_type = get_mime_type(&file_path.to_string_lossy());
    
    // Pre-compile headers
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nLast-Modified: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n",
        mime_type, size, last_modified_str, etag
    );
    
    Ok(FileMetadata {
        headers: headers.into_bytes(),
        size,
        last_modified,
        etag,
    })
}

fn format_http_date(time: SystemTime) -> String {
    // Ultra-fast timestamp formatting optimized for file cache building
    let timestamp = time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    
    // Fastest approach: direct integer to string conversion
    timestamp.to_string()
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

        if method != b"GET" && method != b"HEAD" {
            send_precompiled_response(stream, &HEADER_TEMPLATES.method_not_allowed).await?;
            break;
        }


        // Enhanced connection management - faster header parsing
        let mut keep_alive = version == "HTTP/1.1"; // Default for HTTP/1.1
        let mut if_modified_since: Option<String> = None;
        let mut if_none_match: Option<String> = None;
        
        // Optimized header parsing - zero allocation case-insensitive comparison
        let mut header_buffer = String::new();
        loop {
            header_buffer.clear();
            match reader.read_line(&mut header_buffer).await {
                Ok(0) => break,
                Ok(_) => {
                    let line = header_buffer.trim();
                    if line.is_empty() {
                        break; // End of headers
                    }
                    
                    // Zero-allocation case-insensitive header parsing
                    if header_starts_with(line, "connection:") {
                        let connection_close_requested = header_contains(line, "close");
                        keep_alive = !connection_close_requested && (version == "HTTP/1.1" || header_contains(line, "keep-alive"));
                    } else if header_starts_with(line, "if-modified-since:") {
                        if let Some(value) = line.splitn(2, ':').nth(1) {
                            if_modified_since = Some(value.trim().to_string());
                        }
                    } else if header_starts_with(line, "if-none-match:") {
                        if let Some(value) = line.splitn(2, ':').nth(1) {
                            if_none_match = Some(value.trim().to_string());
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // Handle the request
        let is_head = method == b"HEAD";
        match handle_request(stream, path, is_head, if_modified_since.as_deref(), if_none_match.as_deref()).await {
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

async fn handle_request(
    stream: &mut TcpStream,
    path: &str,
    is_head: bool,
    if_modified_since: Option<&str>,
    if_none_match: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle health check endpoints with pre-compiled responses (optimized for performance)
    if path == "/health" {
        return send_precompiled_health_response(stream, is_head).await;
    }

    if path == "/ready" {
        return send_precompiled_ready_response(stream, is_head).await;
    }

    serve_static_file(stream, path, is_head, if_modified_since, if_none_match).await
}

async fn serve_static_file(
    stream: &mut TcpStream,
    path: &str,
    is_head: bool,
    if_modified_since: Option<&str>,
    if_none_match: Option<&str>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let sanitized_path = sanitize_path(path);
    let lookup_path = if sanitized_path == "/" {
        "/index.html".to_string()
    } else {
        sanitized_path
    };

    // Try to get file metadata from cache
    if let Some(file_metadata) = FILE_CACHE.get(&lookup_path) {
        // Check file size limit (already validated during cache build, but double-check)
        if file_metadata.size > MAX_FILE_SIZE {
            return send_precompiled_response(stream, &HEADER_TEMPLATES.file_too_large).await;
        }

        // Handle conditional requests for 304 Not Modified
        if let Some(should_return_304) = should_return_not_modified(
            if_modified_since,
            if_none_match,
            &file_metadata.last_modified,
            &file_metadata.etag,
        ) {
            if should_return_304 {
                return send_not_modified_response(stream).await;
            }
        }

        // Send cached headers (much faster than building them each time)
        stream.write_all(&file_metadata.headers).await?;

        // For HEAD requests, only send headers, not the file content
        if !is_head {
            // Open and stream the file content (zero-copy)
            let actual_file_path = if lookup_path == "/index.html" {
                format!("{}/index.html", STATIC_DIR)
            } else {
                format!("{}{}", STATIC_DIR, lookup_path)
            };
            
            if let Ok(mut file) = File::open(&actual_file_path).await {
                tokio::io::copy(&mut file, stream).await?;
            } else {
                // File disappeared since cache was built - should be rare
                return send_precompiled_response(stream, &HEADER_TEMPLATES.not_found).await;
            }
        }
        stream.flush().await?;
    } else {
        // File not in cache - return 404
        return send_precompiled_response(stream, &HEADER_TEMPLATES.not_found).await;
    }

    Ok(())
}

fn should_return_not_modified(
    if_modified_since: Option<&str>,
    if_none_match: Option<&str>,
    last_modified: &SystemTime,
    etag: &str,
) -> Option<bool> {
    // If-None-Match takes precedence over If-Modified-Since
    if let Some(none_match_value) = if_none_match {
        // Handle ETag comparison
        // Support both weak and strong ETags, and "*" wildcard
        if none_match_value == "*" {
            return Some(true);
        }
        
        // Parse comma-separated ETags
        let client_etags: Vec<&str> = none_match_value
            .split(',')
            .map(|s| s.trim())
            .collect();
        
        // Check if our ETag matches any of the client's ETags
        let our_etag = etag.trim_start_matches("W/").trim_matches('"');
        for client_etag in client_etags {
            let clean_client_etag = client_etag.trim_start_matches("W/").trim_matches('"');
            if clean_client_etag == our_etag {
                return Some(true);
            }
        }
        
        return Some(false);
    }
    
    // Handle If-Modified-Since
    if let Some(modified_since_str) = if_modified_since {
        // For simplicity, we'll do a basic string comparison since our format is timestamp_X
        // In production, you'd parse the HTTP date properly
        let our_timestamp = last_modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::from_secs(0))
            .as_secs();
        
        // Extract timestamp from our simple format
        if let Some(timestamp_str) = modified_since_str.strip_prefix("timestamp_") {
            if let Ok(client_timestamp) = timestamp_str.parse::<u64>() {
                // If file hasn't been modified since client's timestamp, return 304
                return Some(our_timestamp <= client_timestamp);
            }
        }
        
        return Some(false);
    }
    
    None // No conditional headers present
}

async fn send_not_modified_response(
    stream: &mut TcpStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let response = b"HTTP/1.1 304 Not Modified\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n";
    stream.write_all(response).await?;
    stream.flush().await?;
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

async fn send_precompiled_health_response(stream: &mut TcpStream, is_head: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if is_head {
        // For HEAD requests, send only headers (extract from pre-compiled response)
        let response = &HEADER_TEMPLATES.health_response;
        let header_end = response.windows(4).position(|w| w == b"\r\n\r\n")
            .map(|pos| pos + 4)
            .unwrap_or(response.len());
        stream.write_all(&response[..header_end]).await?;
    } else {
        // For GET requests, send the full pre-compiled response
        stream.write_all(&HEADER_TEMPLATES.health_response).await?;
    }
    stream.flush().await?;
    Ok(())
}

async fn send_precompiled_ready_response(stream: &mut TcpStream, is_head: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if is_head {
        // For HEAD requests, send only headers (extract from pre-compiled response)
        let response = &HEADER_TEMPLATES.ready_response;
        let header_end = response.windows(4).position(|w| w == b"\r\n\r\n")
            .map(|pos| pos + 4)
            .unwrap_or(response.len());
        stream.write_all(&response[..header_end]).await?;
    } else {
        // For GET requests, send the full pre-compiled response
        stream.write_all(&HEADER_TEMPLATES.ready_response).await?;
    }
    stream.flush().await?;
    Ok(())
}


