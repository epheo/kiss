//! End-to-end tests: each test starts the real `kiss` binary on a free port
//! with its own content directory, so plain `cargo test` runs everything.

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const SITE: &[(&str, &str)] = &[
    ("index.html", "Hello World\n"),
    ("css/style.css", "body { color: red; }\n"),
    ("app.js", "console.log(1);\n"),
    ("logo.svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n"),
    ("docs/index.html", "Docs\n"),
    ("digits.txt", "0123456789"),
];

fn content(files: &[(&str, &str)]) -> TempDir {
    let dir = TempDir::new().unwrap();
    for (path, body) in files {
        let path = dir.path().join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }
    dir
}

fn kiss() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_kiss"));
    cmd.env_clear();
    cmd
}

struct Server {
    child: Child,
    port: u16,
    _stdout: BufReader<ChildStdout>,
    _content: TempDir,
}

impl Server {
    fn start() -> Self {
        Self::with(content(SITE), &[], &[])
    }

    /// Configure through `KISS_*` variables, as in Kubernetes; `args` override them
    fn with(content: TempDir, env: &[(&str, &str)], args: &[&str]) -> Self {
        // Another process can take the probed port before kiss binds it: retry
        for _ in 0..5 {
            let probe = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = probe.local_addr().unwrap().port();
            drop(probe);
            let mut child = kiss()
                .env("KISS_PORT", port.to_string())
                .env("KISS_BIND_IP", "127.0.0.1")
                .env("KISS_STATIC_DIR", content.path())
                .envs(env.iter().copied())
                .args(args)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            // The address is printed once kiss accepts connections. The pipe
            // stays open in `Server` so later log lines have somewhere to go.
            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut lines = stdout.by_ref().lines().map_while(Result::ok);
            if lines.any(|line| line.contains("http://")) {
                return Server {
                    child,
                    port,
                    _stdout: stdout,
                    _content: content,
                };
            }
            child.wait().unwrap();
        }
        panic!("kiss did not start");
    }

    fn connect(&self) -> Conn {
        let stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        Conn(BufReader::new(stream))
    }

    /// One raw request on a fresh connection
    fn request(&self, raw: &str) -> Response {
        let mut conn = self.connect();
        conn.send(raw);
        read_response(&mut conn.0, raw.starts_with("HEAD"))
    }

    fn get(&self, path: &str) -> Response {
        self.request(&format!("GET {path} HTTP/1.1\r\nHost: test\r\n\r\n"))
    }

    fn get_with(&self, path: &str, header: &str) -> Response {
        self.request(&format!("GET {path} HTTP/1.1\r\n{header}\r\n\r\n"))
    }

    fn signal(&self, signal: i32) {
        let pid = i32::try_from(self.child.id()).unwrap();
        assert_eq!(unsafe { libc::kill(pid, signal) }, 0);
    }

    fn wait_exit(&mut self, limit: Duration) -> (ExitStatus, Duration) {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return (status, start.elapsed());
            }
            assert!(start.elapsed() < limit, "still running after {limit:?}");
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Conn(BufReader<TcpStream>);

impl Conn {
    fn send(&mut self, raw: &str) {
        self.0.get_mut().write_all(raw.as_bytes()).unwrap();
    }

    fn response(&mut self) -> Response {
        read_response(&mut self.0, false)
    }

    /// Whether the server closes the connection within `wait`
    fn closed_within(&mut self, wait: Duration) -> bool {
        self.0.get_ref().set_read_timeout(Some(wait)).unwrap();
        matches!(self.0.read(&mut [0u8; 1]), Ok(0))
    }
}

#[derive(Default)]
struct Response {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    fn text(&self) -> &str {
        std::str::from_utf8(&self.body).unwrap()
    }
}

/// Read exactly one response: headers, then Content-Length bytes of body
fn read_response(reader: &mut BufReader<TcpStream>, is_head: bool) -> Response {
    let mut r = Response::default();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
            break;
        }
        r.head.push_str(&line);
    }
    r.status = r.head.split(' ').nth(1).map_or(0, |n| n.parse().unwrap());
    if !is_head && r.status != 304 {
        let length = r.header("Content-Length").map_or(0, |n| n.parse().unwrap());
        r.body.resize(length, 0);
        reader.read_exact(&mut r.body).unwrap();
    }
    r
}

#[test]
fn serves_files_with_cache_headers() {
    let server = Server::start();
    let r = server.get("/index.html");
    assert_eq!(r.status, 200);
    assert_eq!(r.body, b"Hello World\n");
    assert_eq!(r.header("Content-Type"), Some("text/html; charset=utf-8"));
    assert_eq!(r.header("Content-Length"), Some("12"));
    assert_eq!(r.header("Cache-Control"), Some("public, max-age=3600"));
    assert_eq!(r.header("X-Content-Type-Options"), Some("nosniff"));
    assert_eq!(r.header("Accept-Ranges"), Some("bytes"));
    assert!(r.header("ETag").unwrap().starts_with("W/\""));
    let date = r.header("Last-Modified").unwrap();
    assert!(date.len() == 29 && date.ends_with(" GMT"), "{date}");

    for (path, mime) in [
        ("/css/style.css", "text/css; charset=utf-8"),
        ("/app.js", "text/javascript; charset=utf-8"),
        ("/logo.svg", "image/svg+xml"),
        ("/digits.txt", "text/plain; charset=utf-8"),
    ] {
        let r = server.get(path);
        assert_eq!(r.header("Content-Type"), Some(mime), "{path}");
    }
}

#[test]
fn directory_indexes_query_strings_and_absolute_uris() {
    let server = Server::start();
    for path in [
        "/",
        "/?utm=x",
        "/index.html?v=1",
        "http://a.com/",
        "https://a.com",
    ] {
        assert_eq!(server.get(path).body, b"Hello World\n", "{path}");
    }
    for path in ["/docs", "/docs/", "/docs/index.html", "/docs/?x=1"] {
        assert_eq!(server.get(path).body, b"Docs\n", "{path}");
    }
}

#[test]
fn missing_files_and_traversal_attempts_are_404() {
    let server = Server::start();
    for path in [
        "/missing.html",
        "/css/missing.css",
        "/kiss",
        "/etc/passwd",
        "/../etc/passwd",
        "/css/../../etc/passwd",
        "/..%2f..%2fetc/passwd",
    ] {
        let r = server.get(path);
        assert_eq!(r.status, 404, "{path}");
        assert_eq!(r.body, b"File not found");
        assert_eq!(r.header("Content-Type"), Some("text/plain"));
        assert_eq!(r.header("X-Content-Type-Options"), Some("nosniff"));
    }
}

#[test]
fn custom_404_page() {
    let server = Server::with(content(&[("404.html", "<h1>Nope</h1>")]), &[], &[]);
    let r = server.get("/missing");
    assert_eq!(r.status, 404);
    assert_eq!(r.header("Content-Type"), Some("text/html"));
    assert_eq!(r.body, b"<h1>Nope</h1>");
}

#[test]
fn symlinks_and_oversized_files_are_not_served() {
    use std::os::unix::fs::symlink;
    let outside = content(&[("secret.txt", "secret")]);
    let site = content(&[("index.html", "home")]);
    symlink(
        outside.path().join("secret.txt"),
        site.path().join("link.txt"),
    )
    .unwrap();
    symlink(outside.path(), site.path().join("linked")).unwrap();
    // Sparse, so it costs no disk: one byte over the 50 MB cache limit
    let huge = fs::File::create(site.path().join("huge.bin")).unwrap();
    huge.set_len(50 * 1024 * 1024 + 1).unwrap();

    let server = Server::with(site, &[], &[]);
    for path in ["/link.txt", "/linked/secret.txt", "/huge.bin"] {
        assert_eq!(server.get(path).status, 404, "{path}");
    }
    assert_eq!(server.get("/index.html").body, b"home");
}

#[test]
fn health_and_readiness_probes() {
    let server = Server::start();
    for (path, status) in [("/health", "healthy"), ("/ready", "ready")] {
        let r = server.get(path);
        assert_eq!(r.status, 200);
        assert_eq!(r.header("Content-Type"), Some("application/json"));
        let expected = format!(r#""status":"{status}""#);
        assert!(r.text().contains(&expected), "{}", r.text());
    }
}

#[test]
fn head_matches_get_without_body() {
    let server = Server::start();
    for path in ["/index.html", "/health", "/missing"] {
        let get = server.get(path);
        let head = server.request(&format!("HEAD {path} HTTP/1.1\r\nHost: test\r\n\r\n"));
        assert_eq!(head.head, get.head, "{path}");
    }
}

#[test]
fn conditional_requests() {
    let server = Server::start();
    let full = server.get("/index.html");
    let etag = full.header("ETag").unwrap();
    let date = full.header("Last-Modified").unwrap();
    let strong = etag.trim_start_matches("W/");
    let (future, past) = (
        "Tue, 01 Jan 2999 00:00:00 GMT",
        "Mon, 01 Jan 1990 00:00:00 GMT",
    );

    for (header, status) in [
        (format!("If-None-Match: {etag}"), 304),
        (format!("if-none-match: {strong}"), 304),
        (format!("IF-NONE-MATCH: \"other\", {etag}"), 304),
        ("If-None-Match: *".to_string(), 304),
        ("If-None-Match: W/\"999-999\"".to_string(), 200),
        (format!("If-Modified-Since: {date}"), 304),
        (format!("If-Modified-Since: {future}"), 304),
        (format!("If-Modified-Since: {past}"), 200),
        ("If-Modified-Since: not a date".to_string(), 200),
        // If-None-Match takes precedence over If-Modified-Since (RFC 7232 §6)
        (
            format!("If-None-Match: \"no\"\r\nIf-Modified-Since: {future}"),
            200,
        ),
    ] {
        let r = server.get_with("/index.html", &header);
        assert_eq!(r.status, status, "{header}");
        if status == 304 {
            assert_eq!(r.header("ETag"), Some(etag));
            assert_eq!(r.header("Cache-Control"), Some("public, max-age=3600"));
            assert_eq!(r.header("Content-Length"), None);
        }
    }
}

#[test]
fn range_requests() {
    let server = Server::start();
    for (range, status, body, content_range) in [
        ("bytes=0-3", 206, "0123", Some("bytes 0-3/10")),
        ("bytes=7-", 206, "789", Some("bytes 7-9/10")),
        ("bytes=-2", 206, "89", Some("bytes 8-9/10")),
        ("bytes=5-100", 206, "56789", Some("bytes 5-9/10")),
        ("bytes=10-", 416, "", Some("bytes */10")),
        ("bytes=0-1,4-5", 200, "0123456789", None),
        ("lines=0-1", 200, "0123456789", None),
    ] {
        let r = server.get_with("/digits.txt", &format!("Range: {range}"));
        assert_eq!(r.status, status, "{range}");
        assert_eq!(r.text(), body, "{range}");
        assert_eq!(r.header("Content-Range"), content_range, "{range}");
        if status == 206 {
            let mime = r.header("Content-Type");
            assert_eq!(mime, Some("text/plain; charset=utf-8"), "{range}");
            assert!(r.header("ETag").is_some() && r.header("Last-Modified").is_some());
        }
    }

    let head = server.request("HEAD /digits.txt HTTP/1.1\r\nRange: bytes=0-3\r\n\r\n");
    assert_eq!(head.status, 206);
    assert_eq!(head.header("Content-Length"), Some("4"));
}

#[test]
fn unsupported_methods_get_405_and_close() {
    let server = Server::start();
    for method in ["POST", "PUT", "DELETE", "PATCH", "OPTIONS"] {
        let mut conn = server.connect();
        conn.send(&format!("{method} /index.html HTTP/1.1\r\n\r\n"));
        let r = conn.response();
        assert_eq!(r.status, 405, "{method}");
        assert_eq!(r.header("Allow"), Some("GET, HEAD"));
        assert_eq!(r.header("Connection"), Some("close"));
        assert!(conn.closed_within(Duration::from_secs(3)), "{method}");
    }
}

#[test]
fn malformed_requests_get_400() {
    let server = Server::start();
    for raw in [
        "BAD REQUEST\r\n\r\n",
        "GET\r\n\r\n",
        "GET /health\r\n\r\n",
        "GET / HTTP/1.1 x\r\n\r\n",
    ] {
        let r = server.request(raw);
        assert_eq!(r.status, 400, "{raw:?}");
        assert_eq!(r.text(), "Malformed request");
    }
    // Tolerated: repeated spaces and a missing Host header
    assert_eq!(server.request("GET  /health  HTTP/1.1\r\n\r\n").status, 200);
}

#[test]
fn oversized_requests_get_413() {
    let large = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(9000));
    let server = Server::start();
    let r = server.request(&large);
    assert_eq!(r.status, 413);
    assert_eq!(r.text(), "Request too large");
    // Headers that never end must not be buffered without bound
    assert_eq!(server.request(large.trim_end()).status, 413);

    let server = Server::with(content(SITE), &[("KISS_MAX_REQUEST_SIZE", "100")], &[]);
    let request = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(100));
    assert_eq!(server.request(&request).status, 413);
    assert_eq!(server.get("/index.html").status, 200);
}

#[test]
fn cli_flags_override_environment() {
    let env = [("KISS_MAX_REQUEST_SIZE", "8192")];
    let server = Server::with(content(SITE), &env, &["--max-request-size", "100"]);
    let request = format!("GET /{} HTTP/1.1\r\n\r\n", "a".repeat(100));
    assert_eq!(server.request(&request).status, 413);
}

#[test]
fn keep_alive_and_pipelining() {
    let server = Server::start();
    let mut conn = server.connect();
    // Two requests in one write; a HEAD body leaking onto the wire would garble the GET
    conn.send("HEAD /index.html HTTP/1.1\r\n\r\nGET /app.js HTTP/1.1\r\n\r\n");
    let head = read_response(&mut conn.0, true);
    assert_eq!(head.header("Content-Length"), Some("12"));
    assert_eq!(conn.response().body, b"console.log(1);\n");
    // A large request that needs several reads, on the same connection
    let padding = "p".repeat(5000);
    conn.send(&format!(
        "GET /index.html HTTP/1.1\r\nX-Pad: {padding}\r\n\r\n"
    ));
    assert_eq!(conn.response().body, b"Hello World\n");
    assert!(!conn.closed_within(Duration::from_millis(300)));
}

#[test]
fn connection_header_and_http_version_control_persistence() {
    let server = Server::start();
    for (raw, closes) in [
        ("GET /health HTTP/1.1\r\nConnection: close\r\n\r\n", true),
        ("GET /health HTTP/1.0\r\n\r\n", true),
        (
            "GET /health HTTP/1.0\r\nConnection: keep-alive\r\n\r\n",
            false,
        ),
        ("GET /health HTTP/1.1\r\n\r\n", false),
    ] {
        let mut conn = server.connect();
        conn.send(raw);
        assert_eq!(conn.response().status, 200, "{raw:?}");
        let closed = conn.closed_within(Duration::from_millis(500));
        assert_eq!(closed, closes, "{raw:?}");
    }
}

#[test]
fn idle_and_incomplete_connections_time_out() {
    let server = Server::with(content(SITE), &[("KISS_KEEPALIVE_TIMEOUT", "1")], &[]);
    let mut idle = server.connect();
    idle.send("GET /health HTTP/1.1\r\n\r\n");
    idle.response();
    let mut incomplete = server.connect();
    incomplete.send("GET /health HTTP/1.1\r\nHost: x");

    let start = Instant::now();
    assert!(idle.closed_within(Duration::from_secs(3)));
    assert!(incomplete.closed_within(Duration::from_secs(3)));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn keep_alive_window_restarts_after_each_response() {
    let server = Server::with(content(SITE), &[("KISS_KEEPALIVE_TIMEOUT", "1")], &[]);
    let mut conn = server.connect();
    // 4 requests spanning more than one window, each within a window of the last
    for _ in 0..4 {
        conn.send("GET /health HTTP/1.1\r\n\r\n");
        assert_eq!(conn.response().status, 200);
        thread::sleep(Duration::from_millis(450));
    }
}

#[test]
fn slowly_sent_request_is_served() {
    let server = Server::start();
    let mut conn = server.connect();
    for byte in "GET /health HTTP/1.1\r\n\r\n".bytes() {
        conn.0.get_mut().write_all(&[byte]).unwrap();
        thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(conn.response().status, 200);
}

#[test]
fn concurrent_keep_alive_clients() {
    let server = Server::start();
    thread::scope(|scope| {
        for _ in 0..16 {
            scope.spawn(|| {
                let mut conn = server.connect();
                for _ in 0..50 {
                    conn.send("GET /index.html HTTP/1.1\r\n\r\n");
                    assert_eq!(conn.response().body, b"Hello World\n");
                }
            });
        }
    });
}

#[test]
fn graceful_shutdown_on_sigterm_and_sigint() {
    for signal in [libc::SIGTERM, libc::SIGINT] {
        let mut server = Server::start();
        assert_eq!(server.get("/health").status, 200);
        server.signal(signal);
        let (status, elapsed) = server.wait_exit(Duration::from_secs(5));
        assert!(status.success(), "{status}");
        assert!(elapsed < Duration::from_secs(2), "{elapsed:?}");
    }
}

#[test]
fn shutdown_answers_requests_on_open_connections_then_closes_them() {
    let mut server = Server::with(content(SITE), &[("KISS_KEEPALIVE_TIMEOUT", "2")], &[]);
    let mut conn = server.connect();
    conn.send("GET /health HTTP/1.1\r\n\r\n");
    conn.response();

    server.signal(libc::SIGTERM);
    thread::sleep(Duration::from_millis(200));
    let refused = TcpStream::connect(("127.0.0.1", server.port)).is_err();
    assert!(refused, "still accepting connections");

    conn.send("GET /index.html HTTP/1.1\r\n\r\n");
    assert_eq!(conn.response().body, b"Hello World\n");
    // Closed right after answering, well before its 2 s keep-alive would expire
    assert!(conn.closed_within(Duration::from_millis(500)));
    let (status, _) = server.wait_exit(Duration::from_secs(5));
    assert!(status.success(), "{status}");
}

#[test]
fn fails_when_port_is_taken() {
    let server = Server::start();
    let port = server.port.to_string();
    let output = kiss()
        .args(["-b", "127.0.0.1", "-p", &port])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("bind"));
}

#[test]
fn cli_help_version_and_validation() {
    let help = kiss().arg("--help").output().unwrap();
    assert!(help.status.success());
    let help = String::from_utf8_lossy(&help.stdout);
    for flag in [
        "port",
        "bind-ip",
        "static-dir",
        "max-request-size",
        "keepalive-timeout",
    ] {
        assert!(
            help.contains(&format!("--{flag}")),
            "--{flag} missing from:\n{help}"
        );
    }

    let version = kiss().arg("--version").output().unwrap();
    assert!(String::from_utf8_lossy(&version.stdout).contains(env!("CARGO_PKG_VERSION")));

    for args in [
        &["--port", "70000"][..],
        &["--port", "http"],
        &["--keepalive-timeout-secs", "0"],
        &["--max-request-size", "-1"],
        &["--no-such-flag"],
    ] {
        let output = kiss().args(args).output().unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
    }
}
