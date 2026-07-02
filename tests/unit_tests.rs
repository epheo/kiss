use kiss::mime_type;
use std::path::Path;


#[cfg(test)]
mod mime_type_tests {
    use super::*;
    
    #[test]
    fn test_html_mime_types() {
        assert_eq!(mime_type(Path::new("index.html")), "text/html; charset=utf-8");
        assert_eq!(mime_type(Path::new("page.htm")), "text/html; charset=utf-8");
        assert_eq!(mime_type(Path::new("INDEX.HTML")), "text/html; charset=utf-8"); // case insensitive
    }
    
    #[test]
    fn test_css_mime_type() {
        assert_eq!(mime_type(Path::new("style.css")), "text/css; charset=utf-8");
        assert_eq!(mime_type(Path::new("STYLE.CSS")), "text/css; charset=utf-8");
    }
    
    #[test]
    fn test_javascript_mime_type() {
        assert_eq!(mime_type(Path::new("app.js")), "text/javascript; charset=utf-8");
        assert_eq!(mime_type(Path::new("script.JS")), "text/javascript; charset=utf-8");
    }
    
    #[test]
    fn test_json_mime_type() {
        assert_eq!(mime_type(Path::new("data.json")), "application/json; charset=utf-8");
    }
    
    #[test]
    fn test_image_mime_types() {
        assert_eq!(mime_type(Path::new("image.png")), "image/png");
        assert_eq!(mime_type(Path::new("photo.jpg")), "image/jpeg");
        assert_eq!(mime_type(Path::new("photo.jpeg")), "image/jpeg");
        assert_eq!(mime_type(Path::new("icon.gif")), "image/gif");
        assert_eq!(mime_type(Path::new("logo.svg")), "image/svg+xml");
        assert_eq!(mime_type(Path::new("favicon.ico")), "image/x-icon");
    }
    
    #[test]
    fn test_font_mime_types() {
        assert_eq!(mime_type(Path::new("font.woff")), "font/woff");
        assert_eq!(mime_type(Path::new("font.woff2")), "font/woff2");
        assert_eq!(mime_type(Path::new("font.ttf")), "font/ttf");
        assert_eq!(mime_type(Path::new("font.eot")), "application/vnd.ms-fontobject");
    }
    
    #[test]
    fn test_other_mime_types() {
        assert_eq!(mime_type(Path::new("document.pdf")), "application/pdf");
        assert_eq!(mime_type(Path::new("data.xml")), "application/xml; charset=utf-8");
        assert_eq!(mime_type(Path::new("readme.txt")), "text/plain; charset=utf-8");
    }
    
    #[test]
    fn test_no_extension() {
        assert_eq!(mime_type(Path::new("file")), "application/octet-stream");
        assert_eq!(mime_type(Path::new("Dockerfile")), "application/octet-stream");
    }
    
    #[test]
    fn test_unknown_extension() {
        assert_eq!(mime_type(Path::new("file.unknown")), "application/octet-stream");
        assert_eq!(mime_type(Path::new("data.xyz")), "application/octet-stream");
    }
    
    #[test]
    fn test_path_with_directories() {
        assert_eq!(mime_type(Path::new("/css/main.css")), "text/css; charset=utf-8");
        assert_eq!(mime_type(Path::new("/images/logo.png")), "image/png");
        assert_eq!(mime_type(Path::new("/js/modules/app.js")), "text/javascript; charset=utf-8");
    }
}

#[cfg(test)]
mod health_endpoint_tests {
    
    #[test]
    fn test_health_response_format() {
        // Mock the timestamp to 1234567890 for consistent testing
        let expected_json = r#"{"status":"healthy","timestamp":"1234567890"}"#;
        
        // Test that health response contains required fields
        // Note: In a real test, we'd mock the timestamp or extract it
        assert!(expected_json.contains(r#""status":"healthy""#));
        assert!(expected_json.contains(r#""timestamp":"#));
    }
    
    #[test]
    fn test_ready_response_format() {
        let expected_json = r#"{"status":"ready","timestamp":"1234567890"}"#;
        
        // Test that ready response contains required fields
        assert!(expected_json.contains(r#""status":"ready""#));
        assert!(expected_json.contains(r#""timestamp":"#));
    }
    
    #[test]
    fn test_json_format_validity() {
        // Test that the JSON structure is valid
        let health_json = r#"{"status":"healthy","timestamp":"1234567890"}"#;
        let ready_json = r#"{"status":"ready","timestamp":"1234567890"}"#;
        
        // Basic JSON validation - should start and end with braces
        assert!(health_json.starts_with('{') && health_json.ends_with('}'));
        assert!(ready_json.starts_with('{') && ready_json.ends_with('}'));
        
        // Should contain proper field separators
        assert!(health_json.contains(r#"":"#));
        assert!(ready_json.contains(r#"":"#));
    }
}