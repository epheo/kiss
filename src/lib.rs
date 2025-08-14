use std::path::Path;

// Optimized MIME type system using enum indices instead of HashMap lookups
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MimeType {
    Html = 0,
    Css = 1,
    Javascript = 2,
    Json = 3,
    Xml = 4,
    PlainText = 5,
    Icon = 6,
    Png = 7,
    Jpeg = 8,
    Gif = 9,
    Svg = 10,
    Pdf = 11,
    Woff = 12,
    Woff2 = 13,
    Ttf = 14,
    Eot = 15,
    OctetStream = 16, // Default for unknown files
}

impl MimeType {
    // Static array for O(1) lookup - much faster than HashMap
    const MIME_STRINGS: [&'static str; 17] = [
        "text/html; charset=utf-8",           // Html
        "text/css; charset=utf-8",            // Css
        "text/javascript; charset=utf-8",     // Javascript
        "application/json; charset=utf-8",    // Json
        "application/xml; charset=utf-8",     // Xml
        "text/plain; charset=utf-8",          // PlainText
        "image/x-icon",                       // Icon
        "image/png",                          // Png
        "image/jpeg",                         // Jpeg
        "image/gif",                          // Gif
        "image/svg+xml",                      // Svg
        "application/pdf",                    // Pdf
        "font/woff",                          // Woff
        "font/woff2",                         // Woff2
        "font/ttf",                           // Ttf
        "application/vnd.ms-fontobject",      // Eot
        "application/octet-stream",           // OctetStream
    ];
    
    // Convert enum to MIME string - zero allocation, O(1) lookup
    pub fn as_str(self) -> &'static str {
        Self::MIME_STRINGS[self as usize]
    }
}


// Fast MIME type detection - optimized internal implementation
pub fn get_mime_type_enum(file_path: &Path) -> MimeType {
    if let Some(extension) = file_path.extension().and_then(|s| s.to_str()) {
        // Use direct string matching instead of HashMap lookup - much faster
        match extension.to_ascii_lowercase().as_str() {
            "html" | "htm" => MimeType::Html,
            "css" => MimeType::Css,
            "js" => MimeType::Javascript,
            "json" => MimeType::Json,
            "xml" => MimeType::Xml,
            "txt" => MimeType::PlainText,
            "ico" => MimeType::Icon,
            "png" => MimeType::Png,
            "jpg" | "jpeg" => MimeType::Jpeg,
            "gif" => MimeType::Gif,
            "svg" => MimeType::Svg,
            "pdf" => MimeType::Pdf,
            "woff" => MimeType::Woff,
            "woff2" => MimeType::Woff2,
            "ttf" => MimeType::Ttf,
            "eot" => MimeType::Eot,
            _ => MimeType::OctetStream,
        }
    } else {
        MimeType::OctetStream
    }
}

// Public API - maintains original string-based interface for compatibility
pub fn get_mime_type(file_path: &str) -> &'static str {
    get_mime_type_enum(Path::new(file_path)).as_str()
}