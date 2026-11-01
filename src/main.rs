//! KISS loads every file under the content directory into memory at startup,
//! together with its complete HTTP response, so serving a request is a hash
//! lookup and a single write.

use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal::unix::{SignalKind, signal};
use tokio::time::{Instant, sleep, timeout, timeout_at};

/// KISS (Kubernetes Instant Static Server) - A fast static file server
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Config {
    /// Port to bind the server to
    #[arg(short, long, env = "KISS_PORT", default_value_t = 8080, value_parser = clap::value_parser!(u16).range(1..))]
    port: u16,

    /// Maximum size of incoming requests in bytes
    #[arg(
        short = 'r',
        long,
        env = "KISS_MAX_REQUEST_SIZE",
        default_value_t = 8192
    )]
    max_request_size: usize,

    /// Directory to serve static files from
    #[arg(short, long, env = "KISS_STATIC_DIR", default_value = "./content")]
    static_dir: String,

    /// Keep-alive timeout in seconds
    #[arg(short, long, env = "KISS_KEEPALIVE_TIMEOUT", default_value_t = 5, value_parser = clap::value_parser!(u64).range(1..))]
    keepalive_timeout_secs: u64,

    /// IP address to bind the server to
    #[arg(short, long, env = "KISS_BIND_IP", default_value = "0.0.0.0")]
    bind_ip: String,
}

/// Files larger than this are skipped at startup to bound memory usage
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024;

// Errors close the connection, since the request boundary is unknown after a
// parse failure, so they advertise Connection: close.
const BAD_REQUEST: &[u8] = b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nMalformed request";
const METHOD_NOT_ALLOWED: &[u8] = b"HTTP/1.1 405 Method Not Allowed\r\nAllow: GET, HEAD\r\nContent-Type: text/plain\r\nContent-Length: 18\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nMethod not allowed";
const REQUEST_TOO_LARGE: &[u8] = b"HTTP/1.1 413 Request Entity Too Large\r\nContent-Type: text/plain\r\nContent-Length: 17\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\nRequest too large";

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static ACTIVE_CONNECTIONS: AtomicUsize = AtomicUsize::new(0);

/// A complete response built at startup. HEAD requests get just the headers.
#[derive(Default)]
struct Prebuilt {
    bytes: Box<[u8]>,
    header_len: usize,
}

impl Prebuilt {
    fn new(mut bytes: Vec<u8>, body: &[u8]) -> Self {
        let header_len = bytes.len();
        bytes.extend_from_slice(body);
        Self {
            bytes: bytes.into(),
            header_len,
        }
    }

    fn simple(status: &str, content_type: &str, body: &[u8]) -> Self {
        let headers = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            body.len()
        );
        Self::new(headers.into_bytes(), body)
    }

    fn for_method(&self, is_head: bool) -> &[u8] {
        if is_head {
            &self.bytes[..self.header_len]
        } else {
            &self.bytes
        }
    }

    fn body(&self) -> &[u8] {
        &self.bytes[self.header_len..]
    }
}

struct File {
    response: Prebuilt,
    not_modified: Box<[u8]>,
    content_type: &'static str,
    etag: String,
    last_modified: String,
    modified: SystemTime,
}

impl File {
    fn load(path: &Path) -> io::Result<Self> {
        let metadata = fs::metadata(path)?;
        if metadata.len() > MAX_FILE_SIZE {
            return Err(io::Error::other("file exceeds the 50 MB cache limit"));
        }
        let body = fs::read(path)?;

        // Whole seconds, matching the resolution of HTTP dates
        let since_epoch = metadata.modified()?.duration_since(UNIX_EPOCH);
        let mtime = since_epoch.map_or(0, |since_epoch| since_epoch.as_secs());
        let modified = UNIX_EPOCH + Duration::from_secs(mtime);
        let etag = format!("W/\"{}-{mtime}\"", body.len());
        let not_modified = format!(
            "HTTP/1.1 304 Not Modified\r\nETag: {etag}\r\nCache-Control: public, max-age=3600\r\nConnection: keep-alive\r\n\r\n"
        );
        let mut file = File {
            response: Prebuilt::default(),
            not_modified: not_modified.into_bytes().into(),
            content_type: mime_type(path),
            etag,
            last_modified: httpdate::fmt_http_date(modified),
            modified,
        };
        file.response = Prebuilt::new(file.headers("200 OK", body.len(), ""), &body);
        Ok(file)
    }

    /// Headers of the 200 and 206 responses; `range` is a Content-Range line or empty
    fn headers(&self, status: &str, length: usize, range: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: {}\r\nContent-Length: {length}\r\n{range}Last-Modified: {}\r\nETag: {}\r\nAccept-Ranges: bytes\r\nCache-Control: public, max-age=3600\r\nX-Content-Type-Options: nosniff\r\nConnection: keep-alive\r\n\r\n",
            self.content_type, self.last_modified, self.etag
        )
        .into_bytes()
    }
}

/// Everything a connection needs, built once at startup
struct Server {
    files: HashMap<String, Arc<File>>,
    health: Prebuilt,
    ready: Prebuilt,
    not_found: Prebuilt,
    max_request_size: usize,
    keepalive: Duration,
}

impl Server {
    fn load(config: &Config) -> Self {
        let dir = Path::new(&config.static_dir);
        let mut files = HashMap::new();
        load_dir(dir, "", &mut files);
        println!("File cache built with {} entries", files.len());

        // A 404.html in the content directory becomes the 404 body
        let not_found = match fs::read(dir.join("404.html")) {
            Ok(page) => Prebuilt::simple("404 Not Found", "text/html", &page),
            Err(_) => Prebuilt::simple("404 Not Found", "text/plain", b"File not found"),
        };
        let probe = |body: &[u8]| Prebuilt::simple("200 OK", "application/json", body);
        Server {
            files,
            health: probe(br#"{"status":"healthy","timestamp":"0"}"#),
            ready: probe(br#"{"status":"ready","timestamp":"0"}"#),
            not_found,
            max_request_size: config.max_request_size,
            keepalive: Duration::from_secs(config.keepalive_timeout_secs),
        }
    }
}

/// Cache every regular file under `dir`, keyed by URL path. `file_type()` does
/// not follow symlinks, so links, which could point outside the content
/// directory, are neither served nor traversed.
fn load_dir(dir: &Path, url_prefix: &str, files: &mut HashMap<String, Arc<File>>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => return eprintln!("Warning: skipping {}: {e}", dir.display()),
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let path = entry.path();
        let url = format!("{url_prefix}/{}", entry.file_name().to_string_lossy());
        if file_type.is_dir() {
            load_dir(&path, &url, files);
        } else if file_type.is_file() {
            match File::load(&path) {
                Ok(file) => {
                    let file = Arc::new(file);
                    // "/dir/index.html" is also served at "/dir", and "/index.html" at "/"
                    if let Some(dir_url) = url.strip_suffix("/index.html") {
                        let dir_url = if dir_url.is_empty() { "/" } else { dir_url };
                        files.insert(dir_url.to_string(), Arc::clone(&file));
                    }
                    files.insert(url, file);
                }
                Err(e) => eprintln!("Warning: skipping {}: {e}", path.display()),
            }
        }
    }
}

/// Map a file extension to its MIME type
fn mime_type(path: &Path) -> &'static str {
    let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
        return "application/octet-stream";
    };
    match extension.to_ascii_lowercase().as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "webmanifest" => "application/manifest+json",
        "xml" => "application/xml; charset=utf-8",
        "txt" => "text/plain; charset=utf-8",
        "ico" => "image/x-icon",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "ogg" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "eot" => "application/vnd.ms-fontobject",
        _ => "application/octet-stream",
    }
}

/// A request, borrowing from the connection buffer
#[derive(Default)]
struct Request<'a> {
    path: &'a str,
    is_head: bool,
    keep_alive: bool,
    if_none_match: Option<&'a [u8]>,
    if_modified_since: Option<&'a [u8]>,
    range: Option<&'a [u8]>,
}

/// Parse a request head (everything before the blank line). An error is the
/// response to send before closing the connection.
fn parse_request(head: &[u8]) -> Result<Request<'_>, &'static [u8]> {
    let mut lines = head.split(|&b| b == b'\n').map(<[u8]>::trim_ascii);
    let request_line = lines.next().unwrap_or_default();
    let request_line = std::str::from_utf8(request_line).map_err(|_| BAD_REQUEST)?;
    let mut parts = request_line.split_ascii_whitespace();
    let (Some(method), Some(target), Some(version), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(BAD_REQUEST);
    };
    let is_head = match method {
        "GET" => false,
        "HEAD" => true,
        _ => return Err(METHOD_NOT_ALLOWED),
    };
    // Absolute-form target (RFC 7230 §5.3.2): "http://host/path" → "/path"
    let path = match target.split_once("://") {
        Some((_scheme, rest)) if !target.starts_with('/') => {
            rest.find('/').map_or("/", |i| &rest[i..])
        }
        _ => target,
    };

    let is_http11 = version == "HTTP/1.1";
    let mut request = Request {
        path,
        is_head,
        keep_alive: is_http11,
        ..Default::default()
    };
    for line in lines {
        let Some(colon) = line.iter().position(|&b| b == b':') else {
            continue;
        };
        let (name, value) = (&line[..colon], line[colon + 1..].trim_ascii());
        if name.eq_ignore_ascii_case(b"connection") {
            request.keep_alive = !contains_ignore_case(value, b"close")
                && (is_http11 || contains_ignore_case(value, b"keep-alive"));
        } else if name.eq_ignore_ascii_case(b"if-none-match") {
            request.if_none_match = Some(value);
        } else if name.eq_ignore_ascii_case(b"if-modified-since") {
            request.if_modified_since = Some(value);
        } else if name.eq_ignore_ascii_case(b"range") {
            request.range = Some(value);
        }
    }
    Ok(request)
}

fn contains_ignore_case(text: &[u8], word: &[u8]) -> bool {
    text.windows(word.len())
        .any(|w| w.eq_ignore_ascii_case(word))
}

/// Cache key for a request path: no query string, no trailing slash
fn cache_key(path: &str) -> &str {
    let path = path.split_once('?').map_or(path, |(path, _query)| path);
    path.strip_suffix('/')
        .filter(|dir| !dir.is_empty())
        .unwrap_or(path)
}

/// RFC 7232 weak comparison: match any comma-separated entry, ignoring W/
/// prefixes and surrounding quotes
fn etag_matches(if_none_match: &[u8], etag: &[u8]) -> bool {
    fn opaque(tag: &[u8]) -> &[u8] {
        let tag = tag.strip_prefix(b"W/").unwrap_or(tag);
        tag.strip_prefix(b"\"")
            .and_then(|t| t.strip_suffix(b"\""))
            .unwrap_or(tag)
    }
    if_none_match == b"*"
        || if_none_match
            .split(|&b| b == b',')
            .any(|tag| opaque(tag.trim_ascii()) == opaque(etag))
}

/// Outcome of a Range header against a body of known length (RFC 7233)
#[derive(Debug, PartialEq, Eq)]
enum ByteRange {
    /// Inclusive byte offsets into the body
    Part(usize, usize),
    /// 416: the request names bytes the body does not have
    Unsatisfiable,
    /// Unparseable or multi-range: the header is ignored and the full body served
    Ignore,
}

/// Parse a single `bytes=` range. Multi-range requests are ignored rather than
/// answered with multipart/byteranges, which no browser media stack needs.
fn parse_byte_range(spec: &[u8], len: usize) -> ByteRange {
    use ByteRange::{Ignore, Part, Unsatisfiable};
    let spec = std::str::from_utf8(spec).unwrap_or_default().trim();
    let Some((first, last)) = spec
        .get(..6)
        .filter(|unit| unit.eq_ignore_ascii_case("bytes=") && !spec.contains(','))
        .and_then(|_| spec[6..].trim().split_once('-'))
    else {
        return Ignore;
    };
    // `str::parse` alone would also accept a leading '+'
    let number = |n: &str| n.parse::<usize>().ok().filter(|_| !n.starts_with('+'));
    match (number(first), number(last)) {
        // Suffix form "-n": the last n bytes
        (None, Some(n)) if first.is_empty() && n > 0 && len > 0 => Part(len - n.min(len), len - 1),
        (None, Some(_)) if first.is_empty() => Unsatisfiable,
        (Some(start), _) if start >= len => Unsatisfiable,
        (Some(start), None) if last.is_empty() => Part(start, len - 1),
        (Some(start), Some(end)) if end >= start => Part(start, end.min(len - 1)),
        _ => Ignore,
    }
}

#[tokio::main]
async fn main() {
    let config = Config::parse();
    // Lives as long as the process, so connections share it without refcounting
    let server: &'static Server = Box::leak(Box::new(Server::load(&config)));

    let listener = TcpListener::bind((config.bind_ip.as_str(), config.port))
        .await
        .expect("Failed to bind to address");
    let address = listener.local_addr().expect("Failed to read bound address");
    println!("Async KISS server running on http://{address}");

    let mut sigterm = signal(SignalKind::terminate()).expect("Failed to handle SIGTERM");
    let mut sigint = signal(SignalKind::interrupt()).expect("Failed to handle SIGINT");
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let _ = stream.set_nodelay(true);
                    ACTIVE_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
                    tokio::spawn(async move {
                        serve_connection(stream, server).await;
                        ACTIVE_CONNECTIONS.fetch_sub(1, Ordering::Release);
                    });
                }
                // Transient failure such as fd exhaustion: back off instead of spinning
                Err(_) => sleep(Duration::from_millis(10)).await,
            },
            _ = sigterm.recv() => break,
            _ = sigint.recv() => break,
        }
    }

    // Stop accepting, then drain: idle connections close within one keep-alive
    // window, and in-flight responses get a grace period on top.
    println!("Shutdown signal received, stopping server...");
    SHUTDOWN.store(true, Ordering::Relaxed);
    drop(listener);
    let deadline = Instant::now() + server.keepalive + Duration::from_secs(2);
    while ACTIVE_CONNECTIONS.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
        sleep(Duration::from_millis(25)).await;
    }
    println!("Server shutdown complete");
}

async fn serve_connection(mut stream: TcpStream, server: &Server) {
    // Requests rarely exceed 1 KB and larger ones grow the buffer; starting
    // bigger makes musl's allocator mmap and munmap for every connection.
    // Pipelined bytes beyond the current request stay buffered for the next.
    let mut buf = Vec::with_capacity(1024);

    loop {
        // Each request must arrive in full within one keep-alive window of the
        // previous response (or of connecting)
        let deadline = Instant::now() + server.keepalive;
        let mut start = 0;
        let head_len = loop {
            // The head must end within max_request_size bytes. Only new bytes
            // need scanning, plus 3 for a terminator split across reads.
            let limit = buf.len().min(server.max_request_size);
            if let Some(i) = buf[start..limit].windows(4).position(|w| w == b"\r\n\r\n") {
                break start + i;
            }
            if buf.len() >= server.max_request_size {
                return close_with(&mut stream, REQUEST_TOO_LARGE, &mut buf).await;
            }
            start = buf.len().saturating_sub(3);
            let read = timeout_at(deadline, stream.read_buf(&mut buf)).await;
            if !matches!(read, Ok(Ok(1..))) {
                return; // timed out, closed by the client, or failed
            }
        };
        let request = match parse_request(&buf[..head_len]) {
            Ok(request) => request,
            Err(response) => return close_with(&mut stream, response, &mut buf).await,
        };
        let sent = respond(&mut stream, server, &request).await;
        if sent.is_err() || !request.keep_alive || SHUTDOWN.load(Ordering::Relaxed) {
            return;
        }
        buf.drain(..head_len + 4);
    }
}

/// Send a final error response, then half-close and briefly drain what the
/// client is still sending: closing with unread data would send a RST, which
/// can destroy the response before the client reads it.
async fn close_with(stream: &mut TcpStream, response: &[u8], buf: &mut Vec<u8>) {
    if stream.write_all(response).await.is_ok() && stream.shutdown().await.is_ok() {
        let drain = async {
            while let Ok(1..) = stream.read_buf(buf).await {
                buf.clear();
            }
        };
        let _ = timeout(Duration::from_secs(2), drain).await;
    }
}

async fn respond(stream: &mut TcpStream, server: &Server, request: &Request<'_>) -> io::Result<()> {
    let prebuilt = match request.path {
        "/health" => &server.health,
        "/ready" => &server.ready,
        path => match server.files.get(cache_key(path)) {
            Some(file) => return send_file(stream, file, request).await,
            None => &server.not_found,
        },
    };
    stream.write_all(prebuilt.for_method(request.is_head)).await
}

async fn send_file(stream: &mut TcpStream, file: &File, request: &Request<'_>) -> io::Result<()> {
    // RFC 7232 §6: If-None-Match takes precedence over If-Modified-Since
    let not_modified = match (request.if_none_match, request.if_modified_since) {
        (Some(tags), _) => etag_matches(tags, file.etag.as_bytes()),
        // Clients usually echo Last-Modified verbatim, so compare bytes before parsing
        (None, Some(since)) => {
            since == file.last_modified.as_bytes()
                || std::str::from_utf8(since)
                    .ok()
                    .and_then(|since| httpdate::parse_http_date(since).ok())
                    .is_some_and(|since| file.modified <= since)
        }
        (None, None) => false,
    };
    if not_modified {
        return stream.write_all(&file.not_modified).await;
    }

    // Safari won't play media from a server that answers Range with a 200, so
    // partial responses are built per request, off the pre-built path.
    if let Some(range) = request.range {
        let body = file.response.body();
        let len = body.len();
        match parse_byte_range(range, len) {
            ByteRange::Part(start, end) => {
                let range = format!("Content-Range: bytes {start}-{end}/{len}\r\n");
                let mut response = file.headers("206 Partial Content", end - start + 1, &range);
                if !request.is_head {
                    response.extend_from_slice(&body[start..=end]);
                }
                return stream.write_all(&response).await;
            }
            ByteRange::Unsatisfiable => {
                let response = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{len}\r\nContent-Length: 0\r\nConnection: keep-alive\r\n\r\n"
                );
                return stream.write_all(response.as_bytes()).await;
            }
            ByteRange::Ignore => {}
        }
    }
    let response = file.response.for_method(request.is_head);
    stream.write_all(response).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mime_types() {
        for (path, expected) in [
            ("index.html", "text/html; charset=utf-8"),
            ("INDEX.HTM", "text/html; charset=utf-8"),
            ("/css/site.CSS", "text/css; charset=utf-8"),
            ("app.mjs", "text/javascript; charset=utf-8"),
            ("data.json", "application/json; charset=utf-8"),
            ("logo.svg", "image/svg+xml"),
            ("photo.JPEG", "image/jpeg"),
            ("font.woff2", "font/woff2"),
            ("clip.mp4", "video/mp4"),
            ("Dockerfile", "application/octet-stream"),
            (".hidden", "application/octet-stream"),
            ("file.", "application/octet-stream"),
            ("archive.tar.xyz", "application/octet-stream"),
        ] {
            assert_eq!(mime_type(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn byte_ranges() {
        use ByteRange::{Ignore, Part, Unsatisfiable};
        for (spec, len, expected) in [
            ("bytes=0-1", 100, Part(0, 1)),
            ("bytes=0-", 100, Part(0, 99)),
            ("bytes=50-500", 100, Part(50, 99)),
            ("bytes=-10", 100, Part(90, 99)),
            ("bytes=-500", 100, Part(0, 99)),
            ("BYTES=3-3", 100, Part(3, 3)),
            (" bytes= 3-3 ", 100, Part(3, 3)),
            ("bytes=100-", 100, Unsatisfiable),
            ("bytes=100-abc", 100, Unsatisfiable),
            ("bytes=-0", 100, Unsatisfiable),
            ("bytes=-5", 0, Unsatisfiable),
            ("bytes=0-", 0, Unsatisfiable),
            ("bytes=0-1,5-6", 100, Ignore),
            ("bytes=5-2", 100, Ignore),
            ("bytes=5-abc", 100, Ignore),
            ("items=0-1", 100, Ignore),
            ("bytes=a-b", 100, Ignore),
            ("bytes=a-5", 100, Ignore),
            ("bytes=+1-2", 100, Ignore),
            ("bytes=1-+2", 100, Ignore),
            ("bytes=-", 100, Ignore),
            ("bytes=", 100, Ignore),
            ("bytes", 100, Ignore),
        ] {
            assert_eq!(parse_byte_range(spec.as_bytes(), len), expected, "{spec}");
        }
    }
}
