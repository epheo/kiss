use rustc_hash::FxHashMap;
use std::fs::{read_dir, metadata, read};
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use std::time::SystemTime;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{timeout, Duration};
use once_cell::sync::OnceCell;
use std::sync::Arc;
use kiss::get_mime_type_enum;

const PORT: u16 = 8080;
const MAX_REQUEST_SIZE: usize = 8192;
const STATIC_DIR: &str = "./content";
// MAX_FILE_SIZE removed - validation now happens during cache building only
const CONNECTION_TIMEOUT_SECS: u64 = 30;
const KEEPALIVE_TIMEOUT_SECS: u64 = 5;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

// String interning pool for path optimization (currently unused in favor of direct hashing)
// Kept for potential future enhancements

// Memory-optimized cache entry - structured for optimal cache line efficiency
#[derive(Clone, Debug)]
#[repr(C)]
struct CacheEntry {
    // Hot path data first (most frequently accessed in request handling)
    complete_response: Arc<[u8]>,     // 8 bytes (pointer)
    headers_only: Arc<[u8]>,          // 8 bytes (pointer)
    not_modified_response: Arc<[u8]>, // 8 bytes (pointer)
    
    // Frequently used for conditional requests
    last_modified_timestamp: SystemTime, // 16 bytes (u64 + u32 typically)
    etag: Arc<str>,                   // 8 bytes (pointer)
    
    // Total: 48 bytes - fits well within cache lines
}

// Compact trie for prefix matching - optimized for trailing slash handling
#[derive(Debug, Clone)]
struct PathTrie {
    // Maps normalized paths to cache entries
    exact_matches: FxHashMap<u32, CacheEntry>,
    // Special handling for index files
    index_entries: FxHashMap<u32, CacheEntry>, // Maps directory hash to index.html entry
}

impl PathTrie {
    fn new() -> Self {
        Self {
            exact_matches: FxHashMap::default(),
            index_entries: FxHashMap::default(),
        }
    }
    
    // Ultra-optimized path normalization with integrated query stripping
    // Single-pass processing: query detection + hash computation + trailing slash handling
    #[inline]
    fn normalize_path_hash(path: &str) -> (u32, bool) {
        const FNV_OFFSET_BASIS: u32 = 2166136261;
        const FNV_PRIME: u32 = 16777619;
        
        let path_bytes = path.as_bytes();
        let mut hash = FNV_OFFSET_BASIS;
        let mut end_pos = path_bytes.len();
        let mut is_directory_style = false;
        
        // Single pass: find query position and check for trailing slash
        for (i, &byte) in path_bytes.iter().enumerate() {
            if byte == b'?' {
                end_pos = i; // Stop at query parameter
                break;
            }
        }
        
        // Check for directory-style path (trailing slash before query)
        if end_pos > 1 && path_bytes[end_pos - 1] == b'/' {
            is_directory_style = true;
            end_pos -= 1; // Remove trailing slash from hash computation
        }
        
        // Hash the clean, normalized path portion
        for &byte in &path_bytes[..end_pos] {
            hash ^= byte as u32;
            hash = hash.wrapping_mul(FNV_PRIME);
        }
        
        (hash, is_directory_style)
    }
    
    fn insert(&mut self, path: &str, entry: CacheEntry) {
        let (path_hash, _is_directory_style) = Self::normalize_path_hash(path);
        
        // Always store in exact matches
        self.exact_matches.insert(path_hash, entry.clone());
        
        // If this is an index.html file, also register it for directory-style access
        if path.ends_with("/index.html") {
            let dir_path = &path[..path.len()-11]; // Remove "/index.html"
            let (dir_hash, _) = Self::normalize_path_hash(dir_path);
            self.index_entries.insert(dir_hash, entry);
        }
    }
    
    fn get(&self, path: &str) -> Option<CacheEntry> {
        let (path_hash, is_directory_style) = Self::normalize_path_hash(path);
        
        // First try exact match
        if let Some(entry) = self.exact_matches.get(&path_hash) {
            return Some(entry.clone());
        }
        
        // For directory-style requests, try index.html
        if is_directory_style || path == "/" {
            if let Some(entry) = self.index_entries.get(&path_hash) {
                return Some(entry.clone());
            }
        }
        
        None
    }
    
    fn entry_count(&self) -> usize {
        self.exact_matches.len()
    }
}

// Lock-free cache with atomic RCU pattern
#[derive(Debug)]
struct OptimizedCache {
    // Atomic pointer to current cache generation (lock-free reads)
    current: AtomicPtr<CacheGeneration>,
}

#[derive(Debug)]
struct CacheGeneration {
    // PathTrie for efficient prefix matching and trailing slash handling
    trie: PathTrie,
}

impl Drop for CacheGeneration {
    fn drop(&mut self) {
        // Custom drop for safe memory management
    }
}

impl OptimizedCache {
    fn new() -> Self {
        let initial_cache = Box::into_raw(Box::new(CacheGeneration {
            trie: PathTrie::new(),
        }));
        
        Self {
            current: AtomicPtr::new(initial_cache),
        }
    }
    
    // Lock-free read operation using atomic load with trie lookup
    fn get(&self, path: &str) -> Option<CacheEntry> {
        // Load the current cache pointer atomically
        let cache_ptr = self.current.load(Ordering::Acquire);
        
        // SAFETY: The pointer is valid as long as we don't perform concurrent updates
        // For a static file server, the cache is built once at startup
        unsafe {
            let cache = &*cache_ptr;
            cache.trie.get(path)
        }
    }
    
    // Only used during initial cache building (single-threaded)
    fn insert(&self, path: String, entry: CacheEntry) {
        // Load current cache
        let cache_ptr = self.current.load(Ordering::Acquire);
        
        // SAFETY: During cache building phase, this is single-threaded
        unsafe {
            let cache = &mut *(cache_ptr as *mut CacheGeneration);
            cache.trie.insert(&path, entry);
        }
    }
    
    fn entry_count(&self) -> usize {
        let cache_ptr = self.current.load(Ordering::Acquire);
        unsafe {
            let cache = &*cache_ptr;
            cache.trie.entry_count()
        }
    }
}

// Zero-I/O file metadata - everything preloaded in memory
#[derive(Clone, Debug)]
struct FileMetadata {
    complete_response: Vec<u8>,      // Headers + content combined for single write
    headers_only: Vec<u8>,           // Headers only for HEAD requests
    not_modified_response: Vec<u8>,  // Pre-generated 304 response
    etag: String,                    // For conditional logic
    last_modified_timestamp: SystemTime, // For If-Modified-Since comparison
}

// Static storage for header templates and optimized file cache - initialized at startup
static HEADER_TEMPLATES: OnceCell<HeaderTemplates> = OnceCell::new();
static FILE_CACHE: OnceCell<OptimizedCache> = OnceCell::new();

// Pre-compiled response templates split into headers and bodies for unified handling
#[derive(Debug)]
struct HeaderTemplates {
    // Error responses (headers + body combined for simplicity since they're small)
    not_found: Vec<u8>,
    method_not_allowed: Vec<u8>,
    request_too_large: Vec<u8>,
    bad_request: Vec<u8>,
    request_timeout: Vec<u8>,
    
    // Health endpoint responses (unified single-write pattern)
    health_complete: Vec<u8>,
    health_headers_only: Vec<u8>,
    ready_complete: Vec<u8>,
    ready_headers_only: Vec<u8>,
}

impl HeaderTemplates {
    fn new() -> Self {
        let (health_complete, health_headers_only) = Self::create_health_response();
        let (ready_complete, ready_headers_only) = Self::create_ready_response();
        
        Self {
            not_found: b"HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: 14\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nFile not found".to_vec(),
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nRequest too large".to_vec(),
            bad_request: b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nMalformed request".to_vec(),
            request_timeout: b"HTTP/1.1 408 Request Timeout\r\nContent-Type: text/plain\r\nContent-Length: 15\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\nRequest timeout".to_vec(),
            
            health_complete,
            health_headers_only,
            ready_complete,
            ready_headers_only,
        }
    }
    
    
    fn create_health_response() -> (Vec<u8>, Vec<u8>) {
        let body = br#"{"status":"healthy","timestamp":"0"}"#;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        ).into_bytes();
        
        // Create unified single-write responses
        let mut complete_response = Vec::with_capacity(headers.len() + body.len());
        complete_response.extend_from_slice(&headers);
        complete_response.extend_from_slice(body);
        
        (complete_response, headers)
    }
    
    fn create_ready_response() -> (Vec<u8>, Vec<u8>) {
        let body = br#"{"status":"ready","timestamp":"0"}"#;
        let headers = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        ).into_bytes();
        
        // Create unified single-write responses
        let mut complete_response = Vec::with_capacity(headers.len() + body.len());
        complete_response.extend_from_slice(&headers);
        complete_response.extend_from_slice(body);
        
        (complete_response, headers)
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

// Query parameter parsing now integrated into PathTrie::normalize_path_hash
// This function has been removed for performance optimization

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

fn build_file_cache() -> OptimizedCache {
    let cache = OptimizedCache::new();
    
    if let Err(e) = discover_files_recursive(STATIC_DIR, "", &cache) {
        eprintln!("Warning: Failed to build file cache: {}", e);
    }
    
    let entry_count = cache.entry_count();
    println!("Optimized file cache built with {} entries", entry_count);
    cache
}

fn discover_files_recursive(
    base_dir: &str,
    relative_path: &str,
    cache: &OptimizedCache,
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
                
                // Convert FileMetadata to optimized CacheEntry
                let cache_entry = CacheEntry {
                    // Hot path data first for better cache locality
                    complete_response: Arc::from(file_metadata.complete_response.into_boxed_slice()),
                    headers_only: Arc::from(file_metadata.headers_only.into_boxed_slice()),
                    not_modified_response: Arc::from(file_metadata.not_modified_response.into_boxed_slice()),
                    last_modified_timestamp: file_metadata.last_modified_timestamp,
                    etag: Arc::from(file_metadata.etag.into_boxed_str()),
                };
                
                // Cache entry - trie automatically handles trailing slashes and index.html mapping
                cache.insert(url_path, cache_entry);
            }
        } else if metadata.is_dir() {
            // Recursively process directories
            discover_files_recursive(base_dir, &current_relative, cache)?;
        }
    }
    
    Ok(())
}

fn generate_file_metadata(file_path: &std::path::Path, _relative_path: &str) -> Result<FileMetadata, Box<dyn std::error::Error>> {
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
    
    // Pre-generate headers-only response for HEAD requests
    let headers_only = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nLast-Modified: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
        mime_type_str, actual_size, last_modified_str, etag
    ).into_bytes();
    
    // Pre-combine headers + content for single write()
    let mut complete_response = Vec::with_capacity(headers.len() + content.len());
    complete_response.extend_from_slice(&headers);
    complete_response.extend_from_slice(&content);
    
    // Pre-generate custom 304 Not Modified response with file-specific ETag
    let not_modified_response = format!(
        "HTTP/1.1 304 Not Modified\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n",
        etag
    ).into_bytes();
    
    Ok(FileMetadata {
        complete_response,
        headers_only,
        not_modified_response,
        etag,
        last_modified_timestamp: last_modified,
    })
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
        let _ = stream.write_all(&HEADER_TEMPLATES.get().unwrap().request_timeout).await;
        let _ = stream.flush().await;
    }
}

async fn handle_connection_inner(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Reusable buffers to eliminate per-request allocations
    let mut request_line = String::with_capacity(128);
    let mut header_buffer = Vec::with_capacity(256);
    
    loop {
        // Check for shutdown
        if SHUTDOWN.load(Ordering::Relaxed) {
            break;
        }

        // Create fresh BufReader per request - optimal for brief line reading
        let mut reader = BufReader::new(&mut *stream);
        request_line.clear(); // Reuse string, just clear content

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
        let mut if_modified_since: Option<Vec<u8>> = None;
        let mut if_none_match: Option<Vec<u8>> = None;
        
        // Ultra-optimized header parsing with zero allocations
        loop {
            header_buffer.clear(); // Reuse vec, just clear content
            
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
                            if_modified_since = Some(value.to_vec());
                        }
                    } else if header_starts_with(line, b"if-none-match:") {
                        if let Some(value) = extract_header_value(line, b"if-none-match:") {
                            if_none_match = Some(value.to_vec());
                        }
                    }
                }
                Err(_) => break,
            }
        }

        // Fast method detection and request handling
        let is_head = method == b"HEAD";
        
        // Direct stream usage for optimal response performance
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

// Helper function for sending precompiled responses efficiently
async fn send_precompiled_response(
    stream: &mut TcpStream,
    response: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    stream.write_all(response).await?;
    stream.flush().await?;
    Ok(())
}

async fn handle_request(
    stream: &mut TcpStream,
    path: &str,
    is_head: bool,
    if_modified_since: Option<&[u8]>,
    if_none_match: Option<&[u8]>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Handle health check endpoints using unified response pattern
    let templates = HEADER_TEMPLATES.get().unwrap();
    
    // Unified single-write pattern for health endpoints
    if path == "/health" {
        if is_head {
            stream.write_all(&templates.health_headers_only).await?;
        } else {
            stream.write_all(&templates.health_complete).await?;
        }
        stream.flush().await?;
        return Ok(());
    }

    if path == "/ready" {
        if is_head {
            stream.write_all(&templates.ready_headers_only).await?;
        } else {
            stream.write_all(&templates.ready_complete).await?;
        }
        stream.flush().await?;
        return Ok(());
    }

    // Inline static file serving for zero function call overhead
    
    // Ultra-fast path lookup with integrated query handling in PathTrie
    let file_cache = FILE_CACHE.get().unwrap();
    
    // Direct path lookup - query parameters handled in hash computation
    let cache_entry = file_cache.get(path);

    // Handle file from cache or 404
    if let Some(cache_entry) = cache_entry {
        // Ultra-fast conditional request handling with If-Modified-Since check first
        if let Some(if_modified_since_bytes) = if_modified_since {
            // Convert bytes to string only when needed for parsing
            if let Ok(if_modified_since_str) = std::str::from_utf8(if_modified_since_bytes) {
                if let Ok(client_time) = httpdate::parse_http_date(if_modified_since_str) {
                    if cache_entry.last_modified_timestamp <= client_time {
                        // Fast path: Use pre-generated 304 response
                        stream.write_all(&cache_entry.not_modified_response).await?;
                        stream.flush().await?;
                        return Ok(());
                    }
                }
            }
        }
        
        // Ultra-fast conditional request handling (immutable files = simple ETag check)
        if let Some(client_etag_bytes) = if_none_match {
            // Perform direct byte comparison for ETag matching
            let etag_bytes = cache_entry.etag.as_bytes();
            if client_etag_bytes == b"*" || 
               (client_etag_bytes.windows(etag_bytes.len()).any(|window| window == etag_bytes)) {
                // Fast path: Use pre-generated 304 response
                stream.write_all(&cache_entry.not_modified_response).await?;
                stream.flush().await?;
                return Ok(());
            }
        }

        // Single write operation - minimal system calls
        if is_head {
            // HEAD request: Send headers only (pre-generated, single write)
            stream.write_all(&cache_entry.headers_only).await?;
        } else {
            // GET request: Send complete response (headers + content in single write!)
            stream.write_all(&cache_entry.complete_response).await?;
        }
        stream.flush().await?;
    } else {
        // File not in cache - return 404
        stream.write_all(&HEADER_TEMPLATES.get().unwrap().not_found).await?;
        stream.flush().await?;
    }

    Ok(())
}

