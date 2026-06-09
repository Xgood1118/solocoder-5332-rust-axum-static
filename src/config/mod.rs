use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ServerConfig {
    pub listen_addr: String,
    pub root_dir: PathBuf,
    pub index_file: String,
    pub spa_fallback: bool,
    pub directory_listing: bool,
    pub directory_page_size: usize,
    pub follow_symlinks: bool,
    pub serve_hidden_files: bool,
    pub max_path_depth: usize,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:8080".to_string(),
            root_dir: PathBuf::from("./public"),
            index_file: "index.html".to_string(),
            spa_fallback: false,
            directory_listing: false,
            directory_page_size: 100,
            follow_symlinks: false,
            serve_hidden_files: false,
            max_path_depth: 32,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CacheConfig {
    pub html_max_age: Duration,
    pub js_css_max_age: Duration,
    pub image_max_age: Duration,
    pub font_max_age: Duration,
    pub video_max_age: Duration,
    pub default_max_age: Duration,
    pub js_css_hash_pattern: String,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            html_max_age: Duration::from_secs(300),
            js_css_max_age: Duration::from_secs(31536000),
            image_max_age: Duration::from_secs(86400),
            font_max_age: Duration::from_secs(31536000),
            video_max_age: Duration::from_secs(86400),
            default_max_age: Duration::from_secs(3600),
            js_css_hash_pattern: r"[a-f0-9]{8,}".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct CompressionConfig {
    pub enabled: bool,
    pub gzip_level: u32,
    pub brotli_level: u32,
    pub deflate_level: u32,
    pub precompressed: bool,
    pub min_size: usize,
    pub max_size: usize,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            gzip_level: 6,
            brotli_level: 4,
            deflate_level: 6,
            precompressed: true,
            min_size: 256,
            max_size: 10 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct HotlinkConfig {
    pub enabled: bool,
    pub allowed_referers: Vec<String>,
    pub allow_direct: bool,
    pub fallback_url: Option<String>,
}

impl Default for HotlinkConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_referers: vec![],
            allow_direct: true,
            fallback_url: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SecurityConfig {
    pub x_content_type_options: bool,
    pub x_frame_options: String,
    pub x_xss_protection: bool,
    pub strict_transport_security: bool,
    pub hsts_max_age: Duration,
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            x_content_type_options: true,
            x_frame_options: "DENY".to_string(),
            x_xss_protection: true,
            strict_transport_security: false,
            hsts_max_age: Duration::from_secs(31536000),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub cache: CacheConfig,
    pub compression: CompressionConfig,
    pub hotlink: HotlinkConfig,
    pub security: SecurityConfig,
}

impl AppConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_root_dir(mut self, root_dir: impl Into<PathBuf>) -> Self {
        self.server.root_dir = root_dir.into();
        self
    }

    pub fn with_listen_addr(mut self, addr: impl Into<String>) -> Self {
        self.server.listen_addr = addr.into();
        self
    }
}
