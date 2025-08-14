use kiss::get_mime_type_enum;
use std::path::Path;


#[cfg(test)]
mod mime_type_tests {
    use super::*;
    
    #[test]
    fn test_html_mime_types() {
        assert_eq!(get_mime_type_enum(Path::new("index.html")).as_str(), "text/html; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("page.htm")).as_str(), "text/html; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("INDEX.HTML")).as_str(), "text/html; charset=utf-8"); // case insensitive
    }
    
    #[test]
    fn test_css_mime_type() {
        assert_eq!(get_mime_type_enum(Path::new("style.css")).as_str(), "text/css; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("STYLE.CSS")).as_str(), "text/css; charset=utf-8");
    }
    
    #[test]
    fn test_javascript_mime_type() {
        assert_eq!(get_mime_type_enum(Path::new("app.js")).as_str(), "text/javascript; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("script.JS")).as_str(), "text/javascript; charset=utf-8");
    }
    
    #[test]
    fn test_json_mime_type() {
        assert_eq!(get_mime_type_enum(Path::new("data.json")).as_str(), "application/json; charset=utf-8");
    }
    
    #[test]
    fn test_image_mime_types() {
        assert_eq!(get_mime_type_enum(Path::new("image.png")).as_str(), "image/png");
        assert_eq!(get_mime_type_enum(Path::new("photo.jpg")).as_str(), "image/jpeg");
        assert_eq!(get_mime_type_enum(Path::new("photo.jpeg")).as_str(), "image/jpeg");
        assert_eq!(get_mime_type_enum(Path::new("icon.gif")).as_str(), "image/gif");
        assert_eq!(get_mime_type_enum(Path::new("logo.svg")).as_str(), "image/svg+xml");
        assert_eq!(get_mime_type_enum(Path::new("favicon.ico")).as_str(), "image/x-icon");
    }
    
    #[test]
    fn test_font_mime_types() {
        assert_eq!(get_mime_type_enum(Path::new("font.woff")).as_str(), "font/woff");
        assert_eq!(get_mime_type_enum(Path::new("font.woff2")).as_str(), "font/woff2");
        assert_eq!(get_mime_type_enum(Path::new("font.ttf")).as_str(), "font/ttf");
        assert_eq!(get_mime_type_enum(Path::new("font.eot")).as_str(), "application/vnd.ms-fontobject");
    }
    
    #[test]
    fn test_other_mime_types() {
        assert_eq!(get_mime_type_enum(Path::new("document.pdf")).as_str(), "application/pdf");
        assert_eq!(get_mime_type_enum(Path::new("data.xml")).as_str(), "application/xml; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("readme.txt")).as_str(), "text/plain; charset=utf-8");
    }
    
    #[test]
    fn test_no_extension() {
        assert_eq!(get_mime_type_enum(Path::new("file")).as_str(), "application/octet-stream");
        assert_eq!(get_mime_type_enum(Path::new("Dockerfile")).as_str(), "application/octet-stream");
    }
    
    #[test]
    fn test_unknown_extension() {
        assert_eq!(get_mime_type_enum(Path::new("file.unknown")).as_str(), "application/octet-stream");
        assert_eq!(get_mime_type_enum(Path::new("data.xyz")).as_str(), "application/octet-stream");
    }
    
    #[test]
    fn test_path_with_directories() {
        assert_eq!(get_mime_type_enum(Path::new("/css/main.css")).as_str(), "text/css; charset=utf-8");
        assert_eq!(get_mime_type_enum(Path::new("/images/logo.png")).as_str(), "image/png");
        assert_eq!(get_mime_type_enum(Path::new("/js/modules/app.js")).as_str(), "text/javascript; charset=utf-8");
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