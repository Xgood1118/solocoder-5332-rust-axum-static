use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use regex::Regex;

use crate::config::CacheConfig;
use crate::mime_util::{category_from_path, FileCategory};

#[derive(Debug, Clone)]
pub struct CachePolicy {
    config: Arc<CacheConfig>,
    hash_regex: Regex,
}

impl CachePolicy {
    pub fn new(config: Arc<CacheConfig>) -> Self {
        let hash_regex = Regex::new(&config.js_css_hash_pattern)
            .unwrap_or_else(|_| Regex::new(r"[a-f0-9]{8,}").unwrap());
        Self { config, hash_regex }
    }

    pub fn get_cache_control(&self, path: &Path) -> CacheDirectives {
        let category = category_from_path(path);
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        match category {
            FileCategory::Html => CacheDirectives {
                max_age: Some(self.config.html_max_age),
                public: true,
                no_cache: false,
                no_store: false,
                immutable: false,
                must_revalidate: false,
            },
            FileCategory::Css | FileCategory::JavaScript => {
                let has_hash = self.hash_regex.is_match(file_name);
                if has_hash {
                    CacheDirectives {
                        max_age: Some(self.config.js_css_max_age),
                        public: true,
                        no_cache: false,
                        no_store: false,
                        immutable: true,
                        must_revalidate: false,
                    }
                } else {
                    CacheDirectives {
                        max_age: Some(Duration::from_secs(3600)),
                        public: true,
                        no_cache: false,
                        no_store: false,
                        immutable: false,
                        must_revalidate: false,
                    }
                }
            }
            FileCategory::Image => CacheDirectives {
                max_age: Some(self.config.image_max_age),
                public: true,
                no_cache: false,
                no_store: false,
                immutable: false,
                must_revalidate: false,
            },
            FileCategory::Font => CacheDirectives {
                max_age: Some(self.config.font_max_age),
                public: true,
                no_cache: false,
                no_store: false,
                immutable: true,
                must_revalidate: false,
            },
            FileCategory::Video | FileCategory::Audio => CacheDirectives {
                max_age: Some(self.config.video_max_age),
                public: true,
                no_cache: false,
                no_store: false,
                immutable: false,
                must_revalidate: false,
            },
            FileCategory::Other => CacheDirectives {
                max_age: Some(self.config.default_max_age),
                public: true,
                no_cache: false,
                no_store: false,
                immutable: false,
                must_revalidate: false,
            },
        }
    }

    pub fn is_immutable(&self, path: &Path) -> bool {
        let directives = self.get_cache_control(path);
        directives.immutable
    }
}

#[derive(Debug, Clone)]
pub struct CacheDirectives {
    pub max_age: Option<Duration>,
    pub public: bool,
    pub no_cache: bool,
    pub no_store: bool,
    pub immutable: bool,
    pub must_revalidate: bool,
}

impl CacheDirectives {
    pub fn to_header_value(&self) -> String {
        let mut parts = Vec::new();

        if self.public {
            parts.push("public".to_string());
        }

        if let Some(max_age) = self.max_age {
            parts.push(format!("max-age={}", max_age.as_secs()));
        }

        if self.no_cache {
            parts.push("no-cache".to_string());
        }

        if self.no_store {
            parts.push("no-store".to_string());
        }

        if self.immutable {
            parts.push("immutable".to_string());
        }

        if self.must_revalidate {
            parts.push("must-revalidate".to_string());
        }

        if parts.is_empty() {
            "no-cache".to_string()
        } else {
            parts.join(", ")
        }
    }

    pub fn no_cache() -> Self {
        Self {
            max_age: None,
            public: false,
            no_cache: true,
            no_store: false,
            immutable: false,
            must_revalidate: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_policy() -> CachePolicy {
        CachePolicy::new(Arc::new(CacheConfig::default()))
    }

    #[test]
    fn test_html_cache() {
        let policy = make_policy();
        let directives = policy.get_cache_control(&PathBuf::from("index.html"));
        assert_eq!(
            directives.max_age.unwrap().as_secs(),
            300
        );
        assert!(!directives.immutable);
    }

    #[test]
    fn test_js_with_hash() {
        let policy = make_policy();
        let directives = policy.get_cache_control(&PathBuf::from("main.abc123def.js"));
        assert!(directives.immutable);
        assert_eq!(directives.max_age.unwrap().as_secs(), 31536000);
    }

    #[test]
    fn test_js_without_hash() {
        let policy = make_policy();
        let directives = policy.get_cache_control(&PathBuf::from("main.js"));
        assert!(!directives.immutable);
    }

    #[test]
    fn test_image_cache() {
        let policy = make_policy();
        let directives = policy.get_cache_control(&PathBuf::from("logo.png"));
        assert_eq!(directives.max_age.unwrap().as_secs(), 86400);
    }

    #[test]
    fn test_cache_header_format() {
        let directives = CacheDirectives {
            max_age: Some(Duration::from_secs(300)),
            public: true,
            no_cache: false,
            no_store: false,
            immutable: false,
            must_revalidate: false,
        };
        assert_eq!(directives.to_header_value(), "public, max-age=300");
    }
}
