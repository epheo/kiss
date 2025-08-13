use std::path::Path;
use lazy_static::lazy_static;
use std::collections::HashMap;

lazy_static! {
    static ref MIME_TYPES: HashMap<&'static str, &'static str> = {
        [
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
        ].iter().cloned().collect()
    };
}

pub fn sanitize_path(path: &str) -> String {
    // Remove query parameters and fragments
    let path = path.split('?').next().unwrap_or(path);
    let path = path.split('#').next().unwrap_or(path);
    
    // Normalize the path to resolve . and .. components
    // This simulates path resolution to prevent directory traversal
    let mut stack = Vec::new();
    
    // Ensure path starts with / for consistent processing
    let normalized_input = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{}", path)
    };
    
    // Split path and process each component
    for part in normalized_input.split('/') {
        if part.is_empty() || part == "." {
            // Skip empty parts and current directory references
            continue;
        } else if part == ".." {
            // Parent directory - pop from stack if possible
            stack.pop();
        } else {
            // Normal component - add to stack
            stack.push(part);
        }
    }
    
    // Build the normalized path
    let normalized_path = if stack.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", stack.join("/"))
    };
    
    // Block access to the kiss binary specifically  
    if normalized_path == "/kiss" {
        "/".to_string() // Return root path to trigger 404
    } else {
        normalized_path
    }
}

pub fn get_mime_type(file_path: &str) -> &'static str {
    if let Some(extension) = Path::new(file_path).extension().and_then(|s| s.to_str()) {
        MIME_TYPES.get(extension.to_lowercase().as_str())
            .unwrap_or(&"application/octet-stream")
    } else {
        "application/octet-stream"
    }
}