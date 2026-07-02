use clap::Parser;
use kiss::mime_type;
use rustc_hash::FxHashMap;
use std::fs::{metadata, read, read_dir};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::SystemTime;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{sleep, timeout, Duration, Instant};

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
    #[arg(short = 'k', long, env = "KISS_KEEPALIVE_TIMEOUT", default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
    keepalive_timeout_secs: u64,

    /// IP address to bind the server to
    #[arg(short = 'b', long, env = "KISS_BIND_IP", default_value = "0.0.0.0")]
    bind_ip: String,
}

/// Files larger than this are skipped at cache build time to bound memory usage
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// Decrements the connection count when a connection task finishes, so
/// shutdown can wait for in-flight requests to drain.
struct ConnGuard;

impl Drop for ConnGuard {
    fn drop(&mut self) {
        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Release);
    }
}

// Cache entry for static files
#[derive(Clone)]
struct CacheEntry {
    complete_response: Arc<[u8]>,        // Pre-generated complete HTTP response
    header_length: usize,                // Where content starts in complete_response
    not_modified_response: Arc<[u8]>,    // Pre-generated 304 response
    etag: Arc<str>,                      // For ETag validation
    last_modified_str: Arc<str>,         // Pre-formatted Last-Modified value for fast comparison
    last_modified_timestamp: SystemTime, // For conditional requests
}

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
        let mut end = bytes.iter().position(|&b| b == b'?').unwrap_or(bytes.len());
        if end > 1 && bytes[end - 1] == b'/' {
            end -= 1;
        }
        &path[..end]
    }

    fn insert(&mut self, path: &str, entry: CacheEntry) {
        self.entries
            .insert(Box::from(Self::normalize_path(path)), entry.clone());

        // "/dir/index.html" is also served at "/dir" ("/" for the root index)
        if let Some(dir_path) = path.strip_suffix("/index.html") {
            let dir_norm = if dir_path.is_empty() {
                "/"
            } else {
                Self::normalize_path(dir_path)
            };
            self.entries.insert(Box::from(dir_norm), entry);
        }
    }

    fn get(&self, path: &str) -> Option<&CacheEntry> {
        self.entries.get(Self::normalize_path(path))
    }

    fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

// Static storage for header templates and file cache - initialized at startup
static HEADER_TEMPLATES: OnceLock<HeaderTemplates> = OnceLock::new();
static FILE_CACHE: OnceLock<FileCache> = OnceLock::new();

// Pre-compiled responses: complete = headers + body, with the header length
// kept where HEAD requests need a headers-only slice.
struct HeaderTemplates {
    not_found: Vec<u8>,
    not_found_header_length: usize,
    method_not_allowed: Vec<u8>,
    request_too_large: Vec<u8>,
    bad_request: Vec<u8>,
    health: Vec<u8>,
    health_header_length: usize,
    ready: Vec<u8>,
    ready_header_length: usize,
}

impl HeaderTemplates {
    fn new() -> Self {
        let (health, health_header_length) = Self::keep_alive_response(
            "200 OK",
            "application/json",
            br#"{"status":"healthy","timestamp":"0"}"#,
        );
        let (ready, ready_header_length) = Self::keep_alive_response(
            "200 OK",
            "application/json",
            br#"{"status":"ready","timestamp":"0"}"#,
        );
        let (not_found, not_found_header_length) =
            Self::keep_alive_response("404 Not Found", "text/plain", b"File not found");

        Self {
            not_found,
            not_found_header_length,
            // These errors always close the connection (the request boundary is
            // unknown after a parse failure), so they advertise Connection: close.
            method_not_allowed: b"HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nMethod not allowed".to_vec(),
            request_too_large: b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nRequest too large".to_vec(),
            bad_request: b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nMalformed request".to_vec(),
            health,
            health_header_length,
            ready,
            ready_header_length,
        }
    }

    /// Build a complete keep-alive response plus its header length for HEAD slicing
    fn keep_alive_response(status: &str, content_type: &str, body: &[u8]) -> (Vec<u8>, usize) {
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        );
        let header_length = headers.len();
        let mut complete = headers.into_bytes();
        complete.extend_from_slice(body);
        (complete, header_length)
    }
}

#[inline]
fn header_starts_with(line: &[u8], prefix: &[u8]) -> bool {
    line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[inline]
fn header_contains(line: &[u8], needle: &[u8]) -> bool {
    line.windows(needle.len())
        .any(|window| window.eq_ignore_ascii_case(needle))
}

// Header value after "name:", with surrounding whitespace stripped
fn extract_header_value<'a>(line: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    let value = line[name.len()..].trim_ascii_start();
    (!value.is_empty()).then_some(value)
}

// RFC 7232 weak ETag comparison: match any comma-separated entry, ignoring
// W/ prefixes and surrounding quotes
fn etag_matches(if_none_match: &[u8], server_etag: &[u8]) -> bool {
    if if_none_match == b"*" {
        return true;
    }
    let server_tag = strip_etag_decoration(server_etag);
    if_none_match
        .split(|&b| b == b',')
        .any(|entry| strip_etag_decoration(entry.trim_ascii()) == server_tag)
}

fn strip_etag_decoration(etag: &[u8]) -> &[u8] {
    let s = etag.strip_prefix(b"W/").unwrap_or(etag);
    s.strip_prefix(b"\"")
        .and_then(|inner| inner.strip_suffix(b"\""))
        .unwrap_or(s)
}

// Zero-allocation request line parser: "METHOD path HTTP/x.y"
fn parse_request_line(line: &[u8]) -> Option<(&[u8], &str, &[u8])> {
    let mut parts = line.split(|&b| b == b' ').filter(|part| !part.is_empty());

    let method = parts.next()?;
    let path_bytes = parts.next()?;
    let version = parts.next()?;
    if parts.next().is_some() {
        return None;
    }

    let path = std::str::from_utf8(path_bytes).ok()?;

    // Absolute-form URI (RFC 7230 §5.3.2): "http://host/path" → "/path"
    let path = match path
        .strip_prefix("http://")
        .or_else(|| path.strip_prefix("https://"))
    {
        Some(rest) => match rest.find('/') {
            Some(i) => &rest[i..],
            None => "/",
        },
        None => path,
    };

    Some((method, path, version))
}

/// A parsed request borrowing directly from the connection buffer — no copies
struct ParsedRequest<'a> {
    consumed: usize,
    is_head: bool,
    keep_alive: bool,
    path: &'a str,
    if_modified_since: Option<&'a [u8]>,
    if_none_match: Option<&'a [u8]>,
}

enum RequestError {
    Malformed,
    MethodNotAllowed,
}

/// Find \r\n\r\n, returning the index of its first byte
#[inline]
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|window| window == b"\r\n\r\n")
}

/// Parse one request whose headers end at `header_end` (as found by find_header_end)
fn parse_request(buf: &[u8], header_end: usize) -> Result<ParsedRequest<'_>, RequestError> {
    let consumed = header_end + 4;

    // A '\n' always exists at header_end + 1, so the fallback is unreachable
    let req_line_end = buf
        .iter()
        .position(|&b| b == b'\n')
        .unwrap_or(header_end + 1);
    let request_line = buf[..req_line_end].trim_ascii();
    if request_line.is_empty() {
        return Err(RequestError::Malformed);
    }

    let (method, path, version) =
        parse_request_line(request_line).ok_or(RequestError::Malformed)?;
    if method != b"GET" && method != b"HEAD" {
        return Err(RequestError::MethodNotAllowed);
    }

    let is_http11 = version == b"HTTP/1.1";
    let mut keep_alive = is_http11;
    let mut if_modified_since = None;
    let mut if_none_match = None;

    let mut pos = req_line_end + 1;
    while pos < header_end {
        let line_end = buf[pos..header_end]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(header_end, |offset| pos + offset);
        let line = buf[pos..line_end].trim_ascii();
        pos = line_end + 1;

        if line.is_empty() {
            continue;
        }

        if header_starts_with(line, b"connection:") {
            keep_alive = !header_contains(line, b"close")
                && (is_http11 || header_contains(line, b"keep-alive"));
        } else if header_starts_with(line, b"if-modified-since:") {
            if_modified_since = extract_header_value(line, b"if-modified-since:");
        } else if header_starts_with(line, b"if-none-match:") {
            if_none_match = extract_header_value(line, b"if-none-match:");
        }
    }

    Ok(ParsedRequest {
        consumed,
        is_head: method == b"HEAD",
        keep_alive,
        path,
        if_modified_since,
        if_none_match,
    })
}

fn build_file_cache(static_dir: &str) -> FileCache {
    let mut cache = FileCache::new();

    if let Err(e) = discover_files_recursive(static_dir, "", &mut cache) {
        eprintln!("Warning: Failed to build file cache: {}", e);
    }

    println!("File cache built with {} entries", cache.entry_count());
    cache
}

fn discover_files_recursive(
    base_dir: &str,
    relative_path: &str,
    cache: &mut FileCache,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut full_path = String::with_capacity(base_dir.len() + relative_path.len() + 1);
    full_path.push_str(base_dir);
    if !relative_path.is_empty() {
        full_path.push('/');
        full_path.push_str(relative_path);
    }

    for entry in read_dir(&full_path)? {
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
            match generate_cache_entry(&entry.path()) {
                Ok(cache_entry) => {
                    let mut url_path = String::with_capacity(current_relative.len() + 1);
                    url_path.push('/');
                    url_path.push_str(&current_relative);
                    cache.insert(&url_path, cache_entry);
                }
                Err(e) => eprintln!("Warning: skipping {}: {}", entry.path().display(), e),
            }
        } else if file_type.is_dir() {
            discover_files_recursive(base_dir, &current_relative, cache)?;
        }
    }

    Ok(())
}

fn generate_cache_entry(
    file_path: &std::path::Path,
) -> Result<CacheEntry, Box<dyn std::error::Error>> {
    let file_metadata = metadata(file_path)?;
    if file_metadata.len() > MAX_FILE_SIZE {
        return Err(format!(
            "file exceeds the {} MB cache limit",
            MAX_FILE_SIZE / (1024 * 1024)
        )
        .into());
    }

    // Truncate mtime to whole seconds to match HTTP date resolution
    let mtime_secs = file_metadata
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let last_modified = SystemTime::UNIX_EPOCH + Duration::from_secs(mtime_secs);
    let last_modified_str = httpdate::fmt_http_date(last_modified);

    let content = read(file_path)?;
    let etag = format!("W/\"{}-{}\"", content.len(), mtime_secs);

    // Pre-combine headers + content so the hot path is a single write()
    let headers = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nLast-Modified: {}\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
        mime_type(file_path),
        content.len(),
        last_modified_str,
        etag
    );
    let header_length = headers.len();
    let mut complete_response = headers.into_bytes();
    complete_response.reserve_exact(content.len());
    complete_response.extend_from_slice(&content);

    let not_modified_response = format!(
        "HTTP/1.1 304 Not Modified\r\nETag: {}\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n",
        etag
    );

    Ok(CacheEntry {
        complete_response: complete_response.into(),
        header_length,
        not_modified_response: not_modified_response.into_bytes().into(),
        etag: etag.into(),
        last_modified_str: last_modified_str.into(),
        last_modified_timestamp: last_modified,
    })
}

#[tokio::main]
async fn main() {
    // Parse CLI arguments and environment once - zero runtime overhead thereafter
    let config = Config::parse();

    // Initialize header templates and file cache before accepting traffic
    assert!(
        HEADER_TEMPLATES.set(HeaderTemplates::new()).is_ok(),
        "header templates already initialized"
    );
    assert!(
        FILE_CACHE.set(build_file_cache(&config.static_dir)).is_ok(),
        "file cache already initialized"
    );

    run_server(
        &config.bind_ip,
        config.port,
        config.max_request_size,
        Duration::from_secs(config.keepalive_timeout_secs),
    )
    .await;
}

async fn run_server(bind_ip: &str, port: u16, max_request_size: usize, keepalive: Duration) {
    let listener = TcpListener::bind(format!("{bind_ip}:{port}"))
        .await
        .expect("Failed to bind to address");

    println!("Async KISS server running on http://{bind_ip}:{port}");

    // Install signal handlers once, outside the accept loop
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    loop {
        tokio::select! {
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        let _ = stream.set_nodelay(true);
                        ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
                        tokio::spawn(async move {
                            let _guard = ConnGuard;
                            handle_connection(stream, max_request_size, keepalive).await;
                        });
                    }
                    // Transient accept failure (e.g. fd exhaustion): back off instead of spinning
                    Err(_) => sleep(Duration::from_millis(10)).await,
                }
            }
            _ = &mut shutdown => {
                println!("Shutdown signal received, stopping server...");
                SHUTDOWN.store(true, Ordering::Relaxed);
                break;
            }
        }
    }

    // Stop accepting, then drain: idle connections notice the shutdown flag
    // within one keepalive window, in-flight responses get a grace period.
    drop(listener);
    let deadline = Instant::now() + keepalive + Duration::from_secs(2);
    while ACTIVE_CONNECTIONS.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(25)).await;
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

/// Send a fatal error response, then FIN and briefly drain unread request
/// bytes so the client can read the response before the socket closes
/// (closing with a non-empty receive queue would send a RST that destroys it).
async fn send_final_response(stream: &mut TcpStream, response: &[u8]) {
    if stream.write_all(response).await.is_err() {
        return;
    }
    let _ = stream.shutdown().await;

    let mut discard = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let wait = deadline.saturating_duration_since(Instant::now());
        match timeout(wait, stream.read(&mut discard)).await {
            Ok(Ok(n)) if n > 0 => continue,
            // EOF, error, or drain deadline reached
            _ => break,
        }
    }
}

async fn handle_connection(mut stream: TcpStream, max_request_size: usize, keepalive: Duration) {
    let templates = HEADER_TEMPLATES.get().unwrap();
    let file_cache = FILE_CACHE.get().unwrap();

    // Single per-connection buffer, reused across requests. Pipelined bytes
    // beyond the current request stay in the buffer for the next iteration.
    let mut buf: Vec<u8> = Vec::with_capacity(max_request_size.min(8192));

    loop {
        // Read until buf holds a complete header block. The keepalive window
        // bounds the idle wait for a first byte; once bytes arrive, the full
        // headers must land within one more window (anti-slowloris deadline).
        let mut deadline: Option<Instant> = None;
        let header_end = loop {
            if let Some(i) = find_header_end(&buf) {
                if i + 4 > max_request_size {
                    send_final_response(&mut stream, &templates.request_too_large).await;
                    return;
                }
                break i;
            }
            if buf.len() >= max_request_size {
                send_final_response(&mut stream, &templates.request_too_large).await;
                return;
            }

            if SHUTDOWN.load(Ordering::Relaxed) {
                return;
            }

            // Fast path (empty buf, whole request in one read) never touches the
            // clock; the deadline is armed only once a partial request is buffered.
            let wait = if buf.is_empty() {
                keepalive
            } else {
                deadline
                    .get_or_insert_with(|| Instant::now() + keepalive)
                    .saturating_duration_since(Instant::now())
            };
            match timeout(wait, stream.read_buf(&mut buf)).await {
                Ok(Ok(n)) if n > 0 => {}
                // Peer closed, IO error, or timed out
                _ => return,
            }
        };

        let parsed = match parse_request(&buf, header_end) {
            Ok(parsed) => parsed,
            Err(RequestError::MethodNotAllowed) => {
                send_final_response(&mut stream, &templates.method_not_allowed).await;
                return;
            }
            Err(RequestError::Malformed) => {
                send_final_response(&mut stream, &templates.bad_request).await;
                return;
            }
        };

        let consumed = parsed.consumed;
        let keep_alive = parsed.keep_alive;
        let ok = handle_request(&mut stream, templates, file_cache, &parsed).await;
        if consumed == buf.len() {
            buf.clear();
        } else {
            // Keep pipelined bytes belonging to the next request
            buf.drain(..consumed);
        }

        if !ok || !keep_alive {
            return;
        }
    }
}

async fn handle_request(
    stream: &mut TcpStream,
    templates: &HeaderTemplates,
    file_cache: &FileCache,
    req: &ParsedRequest<'_>,
) -> bool {
    if req.path == "/health" {
        let end = if req.is_head {
            templates.health_header_length
        } else {
            templates.health.len()
        };
        return stream.write_all(&templates.health[..end]).await.is_ok();
    }

    if req.path == "/ready" {
        let end = if req.is_head {
            templates.ready_header_length
        } else {
            templates.ready.len()
        };
        return stream.write_all(&templates.ready[..end]).await.is_ok();
    }

    let Some(entry) = file_cache.get(req.path) else {
        let end = if req.is_head {
            templates.not_found_header_length
        } else {
            templates.not_found.len()
        };
        return stream.write_all(&templates.not_found[..end]).await.is_ok();
    };

    // RFC 7232 §6: If-None-Match takes precedence; If-Modified-Since applies
    // only when no ETag was sent.
    let not_modified = match (req.if_none_match, req.if_modified_since) {
        (Some(inm), _) => etag_matches(inm, entry.etag.as_bytes()),
        (None, Some(ims)) => {
            // Fast path: clients usually echo our Last-Modified value byte-for-byte
            ims == entry.last_modified_str.as_bytes()
                || std::str::from_utf8(ims)
                    .ok()
                    .and_then(|s| httpdate::parse_http_date(s).ok())
                    .is_some_and(|t| entry.last_modified_timestamp <= t)
        }
        (None, None) => false,
    };
    if not_modified {
        return stream
            .write_all(&entry.not_modified_response)
            .await
            .is_ok();
    }

    let end = if req.is_head {
        entry.header_length
    } else {
        entry.complete_response.len()
    };
    stream
        .write_all(&entry.complete_response[..end])
        .await
        .is_ok()
}
