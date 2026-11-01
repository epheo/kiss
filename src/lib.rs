use std::path::Path;

/// Map a file extension to its MIME type. Runs once per file at cache build time.
pub fn mime_type(file_path: &Path) -> &'static str {
    let Some(extension) = file_path.extension().and_then(|s| s.to_str()) else {
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

/// Outcome of a Range header against a body of known length (RFC 7233).
#[derive(Debug, PartialEq, Eq)]
pub enum ByteRange {
    /// Inclusive byte offsets into the body
    Part(usize, usize),
    /// 416: the request names bytes the body does not have
    Unsatisfiable,
    /// Unparseable or multi-range: the header is ignored and the full body served
    Ignore,
}

/// Parse a single `bytes=` range spec. Multi-range requests are ignored rather
/// than answered with multipart/byteranges, which no browser media stack needs.
pub fn parse_byte_range(spec: &[u8], body_len: usize) -> ByteRange {
    let Some(rest) = strip_prefix_ignore_case(spec.trim_ascii(), b"bytes=") else {
        return ByteRange::Ignore;
    };
    let rest = rest.trim_ascii();
    if rest.contains(&b',') {
        return ByteRange::Ignore;
    }
    let Some(dash) = rest.iter().position(|&b| b == b'-') else {
        return ByteRange::Ignore;
    };
    let (first, last) = (&rest[..dash], &rest[dash + 1..]);

    if first.is_empty() {
        // Suffix form "-n": the last n bytes
        return match parse_digits(last) {
            None => ByteRange::Ignore,
            Some(0) => ByteRange::Unsatisfiable,
            Some(_) if body_len == 0 => ByteRange::Unsatisfiable,
            Some(n) => ByteRange::Part(body_len - n.min(body_len), body_len - 1),
        };
    }

    let Some(start) = parse_digits(first) else {
        return ByteRange::Ignore;
    };
    if start >= body_len {
        return ByteRange::Unsatisfiable;
    }
    let end = if last.is_empty() {
        body_len - 1
    } else {
        match parse_digits(last) {
            Some(end) if end >= start => end.min(body_len - 1),
            _ => return ByteRange::Ignore,
        }
    };
    ByteRange::Part(start, end)
}

fn strip_prefix_ignore_case<'a>(input: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    if input.len() >= prefix.len() && input[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(&input[prefix.len()..])
    } else {
        None
    }
}

fn parse_digits(input: &[u8]) -> Option<usize> {
    if input.is_empty() || !input.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(input).ok()?.parse().ok()
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
        assert_eq!(parse_byte_range(b"bytes=0-1", 100), ByteRange::Part(0, 1));
        assert_eq!(parse_byte_range(b"bytes=0-", 100), ByteRange::Part(0, 99));
        assert_eq!(parse_byte_range(b"bytes=50-500", 100), ByteRange::Part(50, 99));
        assert_eq!(parse_byte_range(b"bytes=-10", 100), ByteRange::Part(90, 99));
        assert_eq!(parse_byte_range(b"bytes=-500", 100), ByteRange::Part(0, 99));
        assert_eq!(parse_byte_range(b"BYTES=3-3", 100), ByteRange::Part(3, 3));
        assert_eq!(parse_byte_range(b"bytes=100-", 100), ByteRange::Unsatisfiable);
        assert_eq!(parse_byte_range(b"bytes=-0", 100), ByteRange::Unsatisfiable);
        assert_eq!(parse_byte_range(b"bytes=0-", 0), ByteRange::Unsatisfiable);
        assert_eq!(parse_byte_range(b"bytes=0-1,5-6", 100), ByteRange::Ignore);
        assert_eq!(parse_byte_range(b"bytes=5-2", 100), ByteRange::Ignore);
        assert_eq!(parse_byte_range(b"items=0-1", 100), ByteRange::Ignore);
        assert_eq!(parse_byte_range(b"bytes=a-b", 100), ByteRange::Ignore);
        assert_eq!(parse_byte_range(b"bytes=-", 100), ByteRange::Ignore);
    }
}
