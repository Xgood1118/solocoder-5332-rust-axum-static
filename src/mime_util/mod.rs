use mime::Mime;
use std::path::Path;
use std::str::FromStr;

pub fn guess_mime(path: &Path) -> Mime {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "html" | "htm" => mime::TEXT_HTML_UTF_8,
        "css" => mime::TEXT_CSS,
        "js" | "mjs" => mime::APPLICATION_JAVASCRIPT_UTF_8,
        "json" => mime::APPLICATION_JSON,
        "xml" => mime::TEXT_XML,
        "txt" => mime::TEXT_PLAIN_UTF_8,
        "png" => mime::IMAGE_PNG,
        "jpg" | "jpeg" => mime::IMAGE_JPEG,
        "gif" => mime::IMAGE_GIF,
        "webp" => Mime::from_str("image/webp").unwrap_or(mime::IMAGE_PNG),
        "svg" | "svgz" => Mime::from_str("image/svg+xml").unwrap_or(mime::IMAGE_SVG),
        "ico" => Mime::from_str("image/x-icon").unwrap_or(mime::IMAGE_PNG),
        "bmp" => mime::IMAGE_BMP,
        "tiff" | "tif" => Mime::from_str("image/tiff").unwrap_or(mime::IMAGE_PNG),
        "woff" => Mime::from_str("font/woff").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "woff2" => Mime::from_str("font/woff2").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "ttf" => Mime::from_str("font/ttf").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "otf" => Mime::from_str("font/otf").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "eot" => Mime::from_str("application/vnd.ms-fontobject").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "mp4" => Mime::from_str("video/mp4").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "webm" => Mime::from_str("video/webm").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "ogg" => Mime::from_str("video/ogg").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "avi" => Mime::from_str("video/x-msvideo").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "mov" => Mime::from_str("video/quicktime").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "mp3" => Mime::from_str("audio/mpeg").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "wav" => Mime::from_str("audio/wav").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "flac" => Mime::from_str("audio/flac").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "aac" => Mime::from_str("audio/aac").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "pdf" => mime::APPLICATION_PDF,
        "zip" => Mime::from_str("application/zip").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "tar" => Mime::from_str("application/x-tar").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "gz" => Mime::from_str("application/gzip").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "br" => Mime::from_str("application/brotli").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "rar" => Mime::from_str("application/vnd.rar").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "7z" => Mime::from_str("application/x-7z-compressed").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "csv" => Mime::from_str("text/csv").unwrap_or(mime::TEXT_PLAIN_UTF_8),
        "md" | "markdown" => Mime::from_str("text/markdown").unwrap_or(mime::TEXT_PLAIN_UTF_8),
        "yaml" | "yml" => Mime::from_str("application/yaml").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "toml" => Mime::from_str("application/toml").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        "wasm" => Mime::from_str("application/wasm").unwrap_or(mime::APPLICATION_OCTET_STREAM),
        _ => mime_guess::from_path(path)
            .first_or_octet_stream(),
    }
}

pub fn is_text_based(mime: &Mime) -> bool {
    mime.type_() == mime::TEXT
        || mime.subtype() == mime::JAVASCRIPT
        || mime.subtype() == mime::JSON
        || mime.subtype() == mime::XML
        || mime.subtype().as_str() == "svg+xml"
        || mime.subtype().as_str() == "html"
        || mime.subtype().as_str() == "css"
        || mime.subtype().as_str() == "markdown"
        || mime.subtype().as_str() == "yaml"
        || mime.subtype().as_str() == "toml"
}

pub fn is_compressible(mime: &Mime) -> bool {
    is_text_based(mime)
        || mime.type_() == mime::IMAGE && mime.subtype().as_str() == "svg+xml"
        || mime.subtype().as_str() == "x-font-woff"
        || mime.type_().as_str() == "font"
}

pub fn category_from_path(path: &Path) -> FileCategory {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "html" | "htm" => FileCategory::Html,
        "css" => FileCategory::Css,
        "js" | "mjs" => FileCategory::JavaScript,
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "svgz" | "ico" | "bmp" | "tiff" | "tif" => {
            FileCategory::Image
        }
        "woff" | "woff2" | "ttf" | "otf" | "eot" => FileCategory::Font,
        "mp4" | "webm" | "ogg" | "avi" | "mov" => FileCategory::Video,
        "mp3" | "wav" | "flac" | "aac" => FileCategory::Audio,
        _ => FileCategory::Other,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileCategory {
    Html,
    Css,
    JavaScript,
    Image,
    Font,
    Video,
    Audio,
    Other,
}

impl FileCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            FileCategory::Html => "html",
            FileCategory::Css => "css",
            FileCategory::JavaScript => "javascript",
            FileCategory::Image => "image",
            FileCategory::Font => "font",
            FileCategory::Video => "video",
            FileCategory::Audio => "audio",
            FileCategory::Other => "other",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_guess_mime_html() {
        let mime = guess_mime(&PathBuf::from("index.html"));
        assert_eq!(mime, mime::TEXT_HTML_UTF_8);
    }

    #[test]
    fn test_guess_mime_font() {
        let mime = guess_mime(&PathBuf::from("font.woff"));
        assert_eq!(mime.type_().as_str(), "font");
    }

    #[test]
    fn test_category_html() {
        assert_eq!(
            category_from_path(&PathBuf::from("index.html")),
            FileCategory::Html
        );
    }

    #[test]
    fn test_category_image() {
        assert_eq!(
            category_from_path(&PathBuf::from("logo.png")),
            FileCategory::Image
        );
    }
}
