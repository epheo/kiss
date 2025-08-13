use std::sync::atomic::{AtomicBool, Ordering};
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::signal;
use tokio::time::{timeout, Duration};
use kiss::{get_mime_type, sanitize_path};

const PORT: u16 = 8080;
const MAX_REQUEST_SIZE: usize = 8192;
const STATIC_DIR: &str = "/app/static";
const MAX_FILE_SIZE: u64 = 50 * 1024 * 1024; // 50MB
const CONNECTION_TIMEOUT_SECS: u64 = 30;
const KEEPALIVE_TIMEOUT_SECS: u64 = 5;

static SHUTDOWN: AtomicBool = AtomicBool::new(false);

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
                        if let Ok(sock) = stream.into_std() {
                            let _ = sock.set_nodelay(true);
                            if let Ok(stream) = TcpStream::from_std(sock) {
                                tokio::spawn(handle_connection(stream));
                            }
                        }
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
        let _ = send_response(&mut stream, 408, "Request Timeout", "text/plain", b"Request timeout").await;
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
                send_response(stream, 413, "Request Entity Too Large", "text/plain", b"Request too large").await?;
                break;
            }
            Ok(Ok(_)) => {}
        }

        if request_line.trim().is_empty() {
            continue; // Keep-alive, wait for next request
        }

        let parts: Vec<&str> = request_line.trim().split_whitespace().collect();
        if parts.len() < 3 {
            send_response(stream, 400, "Bad Request", "text/plain", b"Malformed request").await?;
            break;
        }

        let method = parts[0];
        let path = parts[1];
        let version = parts[2];

        if method != "GET" {
            send_response(stream, 405, "Method Not Allowed", "text/plain", b"Method not allowed").await?;
            break;
        }

        // Read headers to check for keep-alive
        let mut keep_alive = version == "HTTP/1.1"; // Default for HTTP/1.1
        let mut headers = Vec::new();
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line).await {
                Ok(0) => break,
                Ok(_) => {
                    if line.trim().is_empty() {
                        break; // End of headers
                    }
                    if line.to_lowercase().starts_with("connection:") {
                        keep_alive = line.to_lowercase().contains("keep-alive");
                    }
                    headers.push(line.clone());
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
                return send_response(stream, 413, "Request Entity Too Large", "text/plain", b"File too large").await;
            }

            // Get MIME type
            let mime_type = get_mime_type(&file_path);

            // Send response headers
            let headers = format!(
                "HTTP/1.1 200 OK\r\n\
                Content-Type: {}\r\n\
                Content-Length: {}\r\n\
                Cache-Control: public, max-age=3600\r\n\
                X-Content-Type-Options: nosniff\r\n\
                X-Frame-Options: DENY\r\n\
                Content-Security-Policy: default-src 'self'\r\n\
                Connection: keep-alive\r\n\
                \r\n",
                mime_type, file_size
            );

            stream.write_all(headers.as_bytes()).await?;

            // Stream file content efficiently
            let mut buffer = vec![0u8; 8192];
            loop {
                match file.read(&mut buffer).await? {
                    0 => break,
                    n => {
                        stream.write_all(&buffer[..n]).await?;
                    }
                }
            }

            stream.flush().await?;
        }
        Err(_) => {
            send_response(stream, 404, "Not Found", "text/plain", b"File not found").await?;
        }
    }

    Ok(())
}

async fn send_health_response(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let health_status = format!(r#"{{"status":"healthy","timestamp":"{}"}}"#, timestamp);
    send_response(stream, 200, "OK", "application/json", health_status.as_bytes()).await
}

async fn send_ready_response(stream: &mut TcpStream) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let ready_status = format!(r#"{{"status":"ready","timestamp":"{}"}}"#, timestamp);
    send_response(stream, 200, "OK", "application/json", ready_status.as_bytes()).await
}

async fn send_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let response = format!(
        "HTTP/1.1 {} {}\r\n\
        Content-Type: {}\r\n\
        Content-Length: {}\r\n\
        X-Content-Type-Options: nosniff\r\n\
        X-Frame-Options: DENY\r\n\
        Content-Security-Policy: default-src 'self'\r\n\
        Connection: keep-alive\r\n\
        \r\n",
        status_code, status_text, content_type, body.len()
    );

    stream.write_all(response.as_bytes()).await?;
    stream.write_all(body).await?;
    stream.flush().await?;

    Ok(())
}