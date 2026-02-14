use clap::Parser;
use rustc_hash::FxHashMap;
use std::fs::{read_dir, metadata, read};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{timeout, Duration};
use std::sync::OnceLock;
use std::sync::Arc;
use kiss::get_mime_type_enum;

/// KISS (Kubernetes Instant Static Server) - A fast static file server
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Config {
    /// Port to bind the server to
    #[arg(short, long, env = "KISS_PORT", default_value_t = 8080, value_parser = clap::value_parser!(u16).range(1..=65535))]
    port: u16,

    /// Maximum size of incoming requests in bytes
    #[arg(short = 'r', long, env = "KISS_MAX_REQUEST_SIZE", default_value_t = 8192)]
    max_request_size: usize,

    /// Directory to serve static files from
    #[arg(short = 's', long, env = "KISS_STATIC_DIR", default_value = "./content")]
    static_dir: String,

    /// Keep-alive timeout in seconds
    #[arg(short = 'k', long, env = "KISS_KEEPALIVE_TIMEOUT", default_value_t = 5)]
    keepalive_timeout_secs: u64,

    /// IP address to bind the server to
    #[arg(short = 'b', long, env = "KISS_BIND_IP", default_value = "0.0.0.0")]
    bind_ip: String,
}

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// Cache entry for static files
#[derive(Clone, Debug)]
struct CacheEntry {
    complete_response: Arc<[u8]>,        // Pre-generated complete HTTP response
    header_length: usize,                 // Where content starts in complete_response
    not_modified_response: Arc<[u8]>,    // Pre-generated 304 response
    last_modified_timestamp: SystemTime, // For conditional requests
    etag: Arc<str>,                      // For ETag validation
}

#[derive(Debug, Clone)]
struct FileCache {
    entries: FxHashMap<Box<str>, CacheEntry>,
}

impl FileCache {
    fn new() -> Self {
        Self {
            entries: FxHashMap::default(),
        }
    }

    // Strip query string and trailing slash, return normalized slice
    #[inline]
    fn normalize_path(path: &str) -> &str {
        let bytes = path.as_bytes();
        let mut end = bytes.len();

        for (i, &b) in bytes.iter().enumerate() {
            if b == b'?' {
                end = i;
                break;
            }
        }

        if end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }

        &path[..end]
    }

    fn insert(&mut self, path: &str, entry: CacheEntry) {
        let norm = Self::normalize_path(path);
        self.entries.insert(Box::from(norm), entry.clone());

        if path.ends_with("/index.html") {
            let dir_path = &path[..path.len() - 11];
            let dir_norm = if dir_path.is_empty() { "/" } else { Self::normalize_path(dir_path) };
            self.entries.insert(Box::from(dir_norm), entry.clone());

            if path == "/index.html" {
                self.entries.insert(Box::from("/"), entry);
            }
        }
    }

    fn get(&self, path: &str) -> Option<&CacheEntry> {
        let norm = Self::normalize_path(path);
        self.entries.get(norm)
    }

    fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// Static storage for header templates and file cache - initialized at startup
static HEADER_TEMPLATES: OnceLock<HeaderTemplates> = OnceLock::new();
static FILE_CACHE: OnceLock<FileCache> = OnceLock::new();

// Pre-compiled response templates split into headers and bodies for unified handling
#[derive(Debug)]
struct HeaderTemplates {
    // Error responses (headers + body combined for simplicity since they're small)
    not_found: Vec<u8>,
    method_not_allowed: Vec<u8>,
    request_too_large: Vec<u8>,
    bad_request: Vec<u8>,
    
    // Health endpoint responses (unified single-write pattern)
    health_complete: Vec<u8>,
    health_headers_only: Vec<u8>,
    ready_complete: Vec<u8>,
    ready_headers_only: Vec<u8>,
}

impl HeaderTemplates {
    fn new() -> Self {
        let (health_complete, health_headers_only) = Self::create_json_response(br#"{"status":"healthy","timestamp":"0"}"#);
        let (ready_complete, ready_headers_only) = Self::create_json_response(br#"{"status":"ready","timestamp":"0"}"#);

        Self {
            not_found: b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nFile not found".to_vec(),
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nRequest too large".to_vec(),
            bad_request: b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nMalformed request".to_vec(),

            health_complete,
            health_headers_only,
            ready_complete,
            ready_headers_only,
        }
    }

    fn create_json_response(body: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        ).into_bytes();

        let mut complete_response = Vec::with_capacity(headers.len() + body.len());
        complete_response.extend_from_slice(&headers);
        complete_response.extend_from_slice(body);

        (complete_response, headers)
    }
}

fn header_starts_with(header_line: &[u8], prefix: &[u8]) -> bool {
    if header_line.len() < prefix.len() {
        return false;
    }
    
    for i in 0..prefix.len() {
        if header_line[i].to_ascii_lowercase() != prefix[i] {
            return false;
        }
    }
    true
}

fn header_contains(header_line: &[u8], substring: &[u8]) -> bool {
    if substring.is_empty() {
        return true;
    }
    
    if header_line.len() < substring.len() {
        return false;
    }
    
    let first_char = substring[0];
    let mut i = 0;

    while i <= header_line.len() - substring.len() {
        if header_line[i].to_ascii_lowercase() != first_char {
            i += 1;
            continue;
        }

        let mut matches = true;
        for j in 1..substring.len() {
            if header_line[i + j].to_ascii_lowercase() != substring[j] {
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

// RFC 7232 weak ETag comparison: split on commas, strip W/ prefixes, compare opaque-tags
fn etag_matches(if_none_match: &[u8], server_etag: &[u8]) -> bool {
    if if_none_match == b"*" {
        return true;
    }

    // Extract server opaque-tag (strip W/ prefix and quotes)
    let server_tag = strip_etag_decoration(server_etag);

    // Split on commas, trim each entry, compare
    let mut start = 0;
    let len = if_none_match.len();

    while start < len {
        // Skip whitespace
        while start < len && (if_none_match[start] == b' ' || if_none_match[start] == b'\t') {
            start += 1;
        }
        if start >= len {
            break;
        }

        // Find end of this etag (next comma or end)
        let mut end = start;
        while end < len && if_none_match[end] != b',' {
            end += 1;
        }

        // Trim trailing whitespace from this entry
        let mut entry_end = end;
        while entry_end > start && (if_none_match[entry_end - 1] == b' ' || if_none_match[entry_end - 1] == b'\t') {
            entry_end -= 1;
        }

        let client_tag = strip_etag_decoration(&if_none_match[start..entry_end]);
        if client_tag == server_tag {
            return true;
        }

        start = end + 1;
    }
    false
}

fn strip_etag_decoration(etag: &[u8]) -> &[u8] {
    let mut s = etag;
    // Strip W/ prefix
    if s.len() >= 2 && s[0] == b'W' && s[1] == b'/' {
        s = &s[2..];
    }
    // Strip surrounding quotes
    if s.len() >= 2 && s[0] == b'"' && s[s.len() - 1] == b'"' {
        s = &s[1..s.len() - 1];
    }
    s
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
    
    Some((method, path, version))
}

fn build_file_cache(static_dir: &str) -> FileCache {
    let mut cache = FileCache::new();
    
    if let Err(e) = discover_files_recursive(static_dir, "", &mut cache) {
        eprintln!("Warning: Failed to build file cache: {}", e);
    }
    
    let entry_count = cache.entry_count();
    println!("File cache built with {} entries", entry_count);
    cache
}

fn discover_files_recursive(
    base_dir: &str,
    relative_path: &str,
    cache: &mut FileCache,
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
        let file_type = entry.file_type()?;

        // Skip symlinks to prevent directory escape
        if file_type.is_symlink() {
            continue;
        }

        let file_name_os = entry.file_name();
        let file_name = file_name_os.to_string_lossy();

        let current_relative = if relative_path.is_empty() {
            file_name.to_string()
        } else {
            let mut path = String::with_capacity(relative_path.len() + file_name.len() + 1);
            path.push_str(relative_path);
            path.push('/');
            path.push_str(&file_name);
            path
        };

        if file_type.is_file() {
            // Generate cache entry for this file
            if let Ok(cache_entry) = generate_cache_entry(&entry.path()) {
                // Optimized URL path construction
                let mut url_path = String::with_capacity(current_relative.len() + 1);
                url_path.push('/');
                url_path.push_str(&current_relative);
                
                // Cache entry - automatically handles trailing slashes and index.html mapping
                cache.insert(&url_path, cache_entry);
            }
        } else if file_type.is_dir() {
            // Recursively process directories
            discover_files_recursive(base_dir, &current_relative, cache)?;
        }
    }
    
    Ok(())
}

fn generate_cache_entry(file_path: &std::path::Path) -> Result<CacheEntry, Box<dyn std::error::Error>> {
    let file_metadata = metadata(file_path)?;
    let size = file_metadata.len();
    let last_modified_raw = file_metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    // Truncate to second precision during cache building for HTTP compliance
    let last_modified = {
        let duration_since_epoch = last_modified_raw.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        let seconds_only = Duration::from_secs(duration_since_epoch.as_secs());
        SystemTime::UNIX_EPOCH + seconds_only
    };
    
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
    let last_modified_str = httpdate::fmt_http_date(last_modified);
    
    // ZERO-I/O OPTIMIZATION: Pre-load file content into memory
    let content = read(file_path)?;
    let actual_size = content.len();
    
    // Pre-generate complete HTTP headers
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nLast-Modified: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
        mime_type_str, actual_size, last_modified_str, etag
    ).into_bytes();
    
    // Store header length for HEAD request slicing
    let header_length = headers.len();
    
    // Pre-combine headers + content for single write()
    let mut complete_response = Vec::with_capacity(headers.len() + content.len());
    complete_response.extend_from_slice(&headers);
    complete_response.extend_from_slice(&content);
    
    // Pre-generate custom 304 Not Modified response with file-specific ETag
    let not_modified_response = format!(
        "HTTP/1.1 304 Not Modified\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n",
        etag
    ).into_bytes();
    
    Ok(CacheEntry {
        complete_response: Arc::from(complete_response.into_boxed_slice()),
        header_length,
        not_modified_response: Arc::from(not_modified_response.into_boxed_slice()),
        etag: Arc::from(etag.into_boxed_str()),
        last_modified_timestamp: last_modified,
    })
}


#[tokio::main]
async fn main() {
    // Parse command line arguments - pay overhead only once here
    let config = Config::parse();
    
    // Extract values once at startup - zero runtime overhead thereafter
    let port = config.port;
    let max_request_size = config.max_request_size;
    let static_dir = config.static_dir;
    let keepalive_timeout_secs = config.keepalive_timeout_secs;
    let bind_ip = config.bind_ip;
    
    // Initialize header templates and file cache at startup - not on first request
    HEADER_TEMPLATES.set(HeaderTemplates::new())
        .expect("Failed to initialize header templates");
    
    let cache = build_file_cache(&static_dir);
    FILE_CACHE.set(cache)
        .expect("Failed to initialize file cache");

    // Run server with direct config values - true zero overhead
    run_server(bind_ip, port, max_request_size, keepalive_timeout_secs).await;
}

async fn run_server(
    bind_ip: String,
    port: u16,
    max_request_size: usize,
    keepalive_timeout_secs: u64,
) {
    let listener = TcpListener::bind(format!("{}:{}", bind_ip, port))
        .await
        .expect("Failed to bind to address");

    println!("Async KISS server running on http://{}:{}", bind_ip, port);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        // Configure TCP socket for performance
                        let _ = stream.set_nodelay(true);
                        tokio::spawn(handle_connection(
                            stream,
                            max_request_size,
                            keepalive_timeout_secs,
                        ));
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

async fn handle_connection(
    stream: TcpStream,
    max_request_size: usize,
    keepalive_timeout_secs: u64,
) {
    let templates = HEADER_TEMPLATES.get().unwrap();

    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half);

    // Pre-allocate buffers once per connection
    let mut request_line = Vec::with_capacity(512);
    let mut header_buffer = Vec::with_capacity(1024);
    let mut if_modified_since_buf = Vec::with_capacity(256);
    let mut if_none_match_buf = Vec::with_capacity(256);

    loop {
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        request_line.clear();
        header_buffer.clear();
        if_modified_since_buf.clear();
        if_none_match_buf.clear();

        match timeout(
            Duration::from_secs(keepalive_timeout_secs),
            reader.read_until(b'\n', &mut request_line),
        )
        .await
        {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Err(_)) => break,
            Ok(Ok(size)) if size > max_request_size => {
                let _ = write_half.write_all(&templates.request_too_large).await;
                break;
            }
            Ok(Ok(_)) => {}
        }

        let request_bytes = trim_header_line(&request_line);
        if request_bytes.is_empty() {
            continue;
        }
        let (method, path, version) = match parse_request_line_fast(request_bytes) {
            Some((m, p, v)) => (m, p, v),
            None => {
                let _ = write_half.write_all(&templates.bad_request).await;
                break;
            }
        };

        if method != b"GET" && method != b"HEAD" {
            let _ = write_half.write_all(&templates.method_not_allowed).await;
            break;
        }

        let mut keep_alive = version == "HTTP/1.1";
        let mut if_modified_since: Option<&[u8]> = None;
        let mut if_none_match: Option<&[u8]> = None;

        loop {
            header_buffer.clear();

            match reader.read_until(b'\n', &mut header_buffer).await {
                Ok(0) => break,
                Ok(_) => {
                    if header_buffer.is_empty() || (header_buffer.len() == 2 && header_buffer == b"\r\n") {
                        break;
                    }

                    let line = trim_header_line(&header_buffer);
                    if line.is_empty() {
                        break;
                    }

                    if header_starts_with(line, b"connection:") {
                        let connection_close_requested = header_contains(line, b"close");
                        keep_alive = !connection_close_requested && (version == "HTTP/1.1" || header_contains(line, b"keep-alive"));
                    } else if header_starts_with(line, b"if-modified-since:") {
                        if let Some(value) = extract_header_value(line, b"if-modified-since:") {
                            if_modified_since_buf.clear();
                            if_modified_since_buf.extend_from_slice(value);
                            if_modified_since = Some(&if_modified_since_buf);
                        }
                    } else if header_starts_with(line, b"if-none-match:") {
                        if let Some(value) = extract_header_value(line, b"if-none-match:") {
                            if_none_match_buf.clear();
                            if_none_match_buf.extend_from_slice(value);
                            if_none_match = Some(&if_none_match_buf);
                        }
                    }
                }
                Err(_) => break,
            }
        }

        let is_head = method == b"HEAD";

        if handle_request(&mut write_half, path, is_head, if_modified_since, if_none_match).await {
            if !keep_alive {
                break;
            }
        } else {
            break;
        }
    }
}


async fn handle_request(
    writer: &mut WriteHalf<TcpStream>,
    path: &str,
    is_head: bool,
    if_modified_since: Option<&[u8]>,
    if_none_match: Option<&[u8]>,
) -> bool {
    let templates = HEADER_TEMPLATES.get().unwrap();

    if path == "/health" {
        let result = if is_head {
            writer.write_all(&templates.health_headers_only).await
        } else {
            writer.write_all(&templates.health_complete).await
        };
        return result.is_ok();
    }

    if path == "/ready" {
        let result = if is_head {
            writer.write_all(&templates.ready_headers_only).await
        } else {
            writer.write_all(&templates.ready_complete).await
        };
        return result.is_ok();
    }

    let file_cache = FILE_CACHE.get().unwrap();
    let cache_entry = file_cache.get(path);

    if let Some(cache_entry) = cache_entry {
        if let Some(if_modified_since_bytes) = if_modified_since {
            if let Ok(if_modified_since_str) = std::str::from_utf8(if_modified_since_bytes) {
                if let Ok(client_time) = httpdate::parse_http_date(if_modified_since_str) {
                    if cache_entry.last_modified_timestamp <= client_time {
                        let result = writer.write_all(&cache_entry.not_modified_response).await;
                        return result.is_ok();
                    }
                }
            }
        }

        if let Some(client_etag_bytes) = if_none_match {
            if etag_matches(client_etag_bytes, cache_entry.etag.as_bytes()) {
                let result = writer.write_all(&cache_entry.not_modified_response).await;
                return result.is_ok();
            }
        }

        let result = if is_head {
            writer.write_all(&cache_entry.complete_response[..cache_entry.header_length]).await
        } else {
            writer.write_all(&cache_entry.complete_response).await
        };
        result.is_ok()
    } else {
        let result = writer.write_all(&templates.not_found).await;
        result.is_ok()
    }
}

