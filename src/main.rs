use std::collections::HashMap;
use std::fs::{read_dir, metadata};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{timeout, Duration};
use once_cell::sync::OnceCell;
use kiss::get_mime_type_enum;

const PORT: u16 = 8080;
const MAX_REQUEST_SIZE: usize = 8192;
const STATIC_DIR: &str = ".";
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB
const CONNECTION_TIMEOUT_SECS: u64 = 30;
const KEEPALIVE_TIMEOUT_SECS: u64 = 5;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// Optimized file metadata - pre-computed headers and paths for zero request overhead
#[derive(Clone, Debug)]
struct FileMetadata {
    headers: Vec<u8>,                // Pre-generated complete HTTP headers (biggest win)
    file_path: String,               // Pre-computed file path (eliminates string building)
    size: u64,                       // Keep for conditional logic and file size limits
    last_modified: SystemTime,       // Keep for conditional request logic
    etag: String,                    // Keep for conditional logic
}

// Static storage for header templates and file cache - initialized at startup
static HEADER_TEMPLATES: OnceCell<HeaderTemplates> = OnceCell::new();
static FILE_CACHE: OnceCell<HashMap<String, FileMetadata>> = OnceCell::new();

// Pre-compiled response header templates
#[derive(Debug)]
struct HeaderTemplates {
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
            not_found: b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile not found".to_vec(),
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nRequest too large".to_vec(),
            file_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\nFile too large".to_vec(),
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

// Optimized case-insensitive ASCII comparison using SIMD-friendly approach
fn header_starts_with(header_line: &[u8], prefix: &[u8]) -> bool {
    if header_line.len() < prefix.len() {
        return false;
    }
    
    // Direct byte comparison is faster for short prefixes
    for i in 0..prefix.len() {
        let h = header_line[i];
        let p = prefix[i];
        if h != p && h.to_ascii_lowercase() != p.to_ascii_lowercase() {
            return false;
        }
    }
    true
}

// Optimized case-insensitive contains check using Boyer-Moore-like approach
fn header_contains(header_line: &[u8], substring: &[u8]) -> bool {
    if substring.is_empty() {
        return true;
    }
    
    if header_line.len() < substring.len() {
        return false;
    }
    
    let first_char = substring[0].to_ascii_lowercase();
    let mut i = 0;
    
    while i <= header_line.len() - substring.len() {
        // Quick first-byte check
        if header_line[i].to_ascii_lowercase() != first_char {
            i += 1;
            continue;
        }
        
        // Check remaining bytes
        let mut matches = true;
        for j in 1..substring.len() {
            let h = header_line[i + j];
            let s = substring[j];
            if h != s && h.to_ascii_lowercase() != s.to_ascii_lowercase() {
                matches = false;
                break;
            }
        }
        
        if matches {
            return true;
        }
        i += 1;
    }
    false
}

// Helper function to read a line into a byte buffer
async fn read_line_bytes(reader: &mut BufReader<&mut TcpStream>, buffer: &mut Vec<u8>) -> Result<usize, std::io::Error> {
    let mut total_bytes = 0;
    loop {
        let bytes_read = reader.read_until(b'\n', buffer).await?;
        total_bytes += bytes_read;
        if bytes_read == 0 || buffer.ends_with(b"\n") {
            break;
        }
    }
    Ok(total_bytes)
}

// Fast header line trimming
fn trim_header_line(line: &[u8]) -> &[u8] {
    let mut start = 0;
    let mut end = line.len();
    
    // Trim trailing CRLF and whitespace
    while end > 0 {
        match line[end - 1] {
            b'\r' | b'\n' | b' ' | b'\t' => end -= 1,
            _ => break,
        }
    }
    
    // Trim leading whitespace
    while start < end {
        match line[start] {
            b' ' | b'\t' => start += 1,
            _ => break,
        }
    }
    
    &line[start..end]
}

// Extract header value without allocation
fn extract_header_value<'a>(line: &'a [u8], header_name: &[u8]) -> Option<&'a [u8]> {
    if line.len() <= header_name.len() {
        return None;
    }
    
    let value_start = header_name.len();
    let value_bytes = &line[value_start..];
    
    // Skip whitespace after colon
    let mut start = 0;
    while start < value_bytes.len() && (value_bytes[start] == b' ' || value_bytes[start] == b'\t') {
        start += 1;
    }
    
    if start >= value_bytes.len() {
        return None;
    }
    
    Some(&value_bytes[start..])
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
    // Optimized path construction using pre-allocated capacity
    let mut full_path = String::with_capacity(base_dir.len() + relative_path.len() + 1);
    full_path.push_str(base_dir);
    if !relative_path.is_empty() {
        full_path.push('/');
        full_path.push_str(relative_path);
    }
    
    let entries = read_dir(&full_path)?;
    
    for entry in entries {
        let entry = entry?;
        let metadata = entry.metadata()?;
        
        // Use OsStr to avoid unnecessary UTF-8 conversion until needed
        let file_name_os = entry.file_name();
        let file_name = file_name_os.to_string_lossy();
        
        // Optimized path joining - pre-allocate with capacity
        let current_relative = if relative_path.is_empty() {
            file_name.to_string()
        } else {
            let mut path = String::with_capacity(relative_path.len() + file_name.len() + 1);
            path.push_str(relative_path);
            path.push('/');
            path.push_str(&file_name);
            path
        };
        
        if metadata.is_file() {
            // Generate cache entry for this file
            if let Ok(file_metadata) = generate_file_metadata(&entry.path(), &current_relative) {
                // Optimized URL path construction
                let mut url_path = String::with_capacity(current_relative.len() + 1);
                url_path.push('/');
                url_path.push_str(&current_relative);
                cache.insert(url_path, file_metadata);
            }
        } else if metadata.is_dir() {
            // Recursively process directories
            discover_files_recursive(base_dir, &current_relative, cache)?;
        }
    }
    
    Ok(())
}

fn generate_file_metadata(file_path: &std::path::Path, relative_path: &str) -> Result<FileMetadata, Box<dyn std::error::Error>> {
    let file_metadata = metadata(file_path)?;
    let size = file_metadata.len();
    let last_modified_raw = file_metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    // Truncate to second precision during cache building for HTTP compliance
    let last_modified = truncate_to_seconds(&last_modified_raw);
    
    // Generate weak ETag using size and modification time
    let mtime_secs = last_modified
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs();
    let etag = format!("W/\"{}-{}\"", size, mtime_secs);
    
    // Get MIME type using fast enum lookup during cache building
    let mime_type_enum = get_mime_type_enum(file_path);
    let mime_type_str = mime_type_enum.as_str();
    
    // Format HTTP date once during cache building - RFC 7231 compliant
    let last_modified_str = format_http_date(last_modified);
    
    // Pre-generate complete HTTP headers - eliminates all runtime allocations
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nLast-Modified: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline' https:; script-src 'self' 'unsafe-inline' https:; img-src 'self' data: blob: https:; font-src 'self' data: https:; object-src 'self' data:; base-uri 'self'\r\nConnection: keep-alive\r\n\r\n",
        mime_type_str, size, last_modified_str, etag
    ).into_bytes();
    
    // Pre-compute file path - eliminates runtime string building
    let full_file_path = if relative_path == "index.html" {
        "./index.html".to_string()
    } else {
        let mut path_buf = String::with_capacity(STATIC_DIR.len() + relative_path.len() + 1);
        path_buf.push_str(STATIC_DIR);
        path_buf.push('/');
        path_buf.push_str(relative_path);
        path_buf
    };
    
    Ok(FileMetadata {
        headers,
        file_path: full_file_path,
        size,
        last_modified,
        etag,
    })
}

fn format_http_date(time: SystemTime) -> String {
    // RFC 7231 compliant HTTP-date formatting - done once during cache building
    httpdate::fmt_http_date(time)
}

#[tokio::main]
async fn main() {
    // Initialize header templates and file cache at startup - not on first request
    HEADER_TEMPLATES.set(HeaderTemplates::new())
        .expect("Failed to initialize header templates");
    
    let cache = build_file_cache();
    FILE_CACHE.set(cache)
        .expect("Failed to initialize file cache");

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
        let _ = send_precompiled_response(&mut stream, &HEADER_TEMPLATES.get().unwrap().request_timeout).await;
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
                send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().request_too_large).await?;
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
                send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().bad_request).await?;
                break;
            }
        };

        if method != b"GET" && method != b"HEAD" {
            send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().method_not_allowed).await?;
            break;
        }


        // Enhanced connection management - faster header parsing
        let mut keep_alive = version == "HTTP/1.1"; // Default for HTTP/1.1
        let mut if_modified_since: Option<String> = None;
        let mut if_none_match: Option<String> = None;
        
        // Ultra-optimized header parsing with minimal allocations
        let mut header_buffer = Vec::with_capacity(256); // Pre-allocate reasonable buffer
        loop {
            header_buffer.clear();
            
            // Read header line into byte buffer
            match read_line_bytes(&mut reader, &mut header_buffer).await {
                Ok(0) => break, // Connection closed
                Ok(_) => {
                    if header_buffer.is_empty() || (header_buffer.len() == 2 && header_buffer == b"\r\n") {
                        break; // End of headers
                    }
                    
                    // Trim CRLF and whitespace
                    let line = trim_header_line(&header_buffer);
                    if line.is_empty() {
                        break;
                    }
                    
                    // Optimized header parsing using byte slices
                    if header_starts_with(line, b"connection:") {
                        let connection_close_requested = header_contains(line, b"close");
                        keep_alive = !connection_close_requested && (version == "HTTP/1.1" || header_contains(line, b"keep-alive"));
                    } else if header_starts_with(line, b"if-modified-since:") {
                        if let Some(value) = extract_header_value(line, b"if-modified-since:") {
                            if_modified_since = Some(String::from_utf8_lossy(value).into_owned());
                        }
                    } else if header_starts_with(line, b"if-none-match:") {
                        if let Some(value) = extract_header_value(line, b"if-none-match:") {
                            if_none_match = Some(String::from_utf8_lossy(value).into_owned());
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
        return send_response(stream, &HEADER_TEMPLATES.get().unwrap().health_response, is_head).await;
    }

    if path == "/ready" {
        return send_response(stream, &HEADER_TEMPLATES.get().unwrap().ready_response, is_head).await;
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
    // Strip query parameters for cache lookup (zero allocation)
    let clean_path = path.split('?').next().unwrap_or(path);
    
    // Smart lookup: try direct path first, then index.html for root
    let file_cache = FILE_CACHE.get().unwrap();
    let file_metadata = file_cache.get(clean_path)
        .or_else(|| if clean_path == "/" { 
            file_cache.get("/index.html") 
        } else { 
            None 
        });

    // Try to get file metadata from cache
    if let Some(file_metadata) = file_metadata {
        // Check file size limit (already validated during cache build, but double-check)
        if file_metadata.size > MAX_FILE_SIZE {
            return send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().file_too_large).await;
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

        // Send pre-generated headers - zero allocations!
        stream.write_all(&file_metadata.headers).await?;

        // For HEAD requests, only send headers, not the file content
        if !is_head {
            // Use pre-computed file path - zero string building!
            match File::open(&file_metadata.file_path).await {
                Ok(mut file) => {
                    tokio::io::copy(&mut file, stream).await?;
                }
                Err(_) => {
                    // File disappeared since cache was built - should be rare
                    return send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().not_found).await;
                }
            }
        }
        stream.flush().await?;
    } else {
        // File not in cache - return 404
        return send_precompiled_response(stream, &HEADER_TEMPLATES.get().unwrap().not_found).await;
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
        // Handle ETag comparison - support both weak and strong ETags, and "*" wildcard
        if none_match_value == "*" {
            return Some(true);
        }
        
        // Optimized ETag comparison without Vec allocation
        let our_etag = strip_etag_wrapper(etag);
        
        // Parse comma-separated ETags using iterator (no allocation)
        for client_etag_raw in none_match_value.split(',') {
            let client_etag = strip_etag_wrapper(client_etag_raw.trim());
            if client_etag == our_etag {
                return Some(true);
            }
        }
        
        return Some(false);
    }
    
    // Handle If-Modified-Since - optimized timestamp comparison
    if let Some(modified_since_str) = if_modified_since {
        return Some(is_not_modified_since(modified_since_str, last_modified));
    }
    
    None // No conditional headers present
}

// Fast ETag wrapper stripping without allocation
fn strip_etag_wrapper(etag: &str) -> &str {
    etag.trim()
        .strip_prefix("W/").unwrap_or(etag.trim())
        .trim_matches('"')
}

// Helper to truncate SystemTime to second precision for HTTP date comparison
fn truncate_to_seconds(time: &SystemTime) -> SystemTime {
    let duration_since_epoch = time.duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_else(|_| std::time::Duration::from_secs(0));
    let seconds_only = std::time::Duration::from_secs(duration_since_epoch.as_secs());
    SystemTime::UNIX_EPOCH + seconds_only
}

// RFC-compliant HTTP date comparison for If-Modified-Since
fn is_not_modified_since(modified_since_str: &str, last_modified: &SystemTime) -> bool {
    // Parse the HTTP date from client's If-Modified-Since header
    match httpdate::parse_http_date(modified_since_str) {
        Ok(client_time) => {
            // Return true (304 Not Modified) if our file is not newer than client's cached version
            // Use <= because HTTP dates have 1-second resolution
            // Note: last_modified is already truncated to second precision during cache building
            *last_modified <= client_time
        }
        Err(_) => {
            // Invalid date format - be conservative and assume file was modified
            false
        }
    }
}

// Unified response handler that supports all response types
async fn send_response(
    stream: &mut TcpStream,
    response_data: &[u8],
    is_head: bool,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if is_head {
        // For HEAD requests, extract and send only headers
        let header_end = response_data.windows(4).position(|w| w == b"\r\n\r\n")
            .map(|pos| pos + 4)
            .unwrap_or(response_data.len());
        stream.write_all(&response_data[..header_end]).await?;
    } else {
        // For GET requests, send the full response
        stream.write_all(response_data).await?;
    }
    stream.flush().await?;
    Ok(())
}

// Convenience function for simple precompiled responses (GET only)
async fn send_precompiled_response(
    stream: &mut TcpStream,
    response: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    send_response(stream, response, false).await
}

// 304 Not Modified response - headers only, no body
async fn send_not_modified_response(
    stream: &mut TcpStream,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let response = b"HTTP/1.1 304 Not Modified\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n";
    // 304 responses never have a body, regardless of request method
    stream.write_all(response).await?;
    stream.flush().await?;
    Ok(())
}




