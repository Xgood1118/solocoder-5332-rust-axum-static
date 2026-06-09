use std::sync::Arc;

use crate::config::HotlinkConfig;

#[derive(Debug, Clone)]
pub struct HotlinkProtector {
    config: Arc<HotlinkConfig>,
    allowed_patterns: Vec<WildcardPattern>,
}

#[derive(Debug, Clone)]
struct WildcardPattern {
    pattern: String,
    regex: regex::Regex,
}

impl WildcardPattern {
    fn new(pattern: &str) -> Result<Self, regex::Error> {
        let escaped = regex::escape(pattern)
            .replace(r"\*", ".*")
            .replace(r"\?", ".");
        let regex = regex::Regex::new(&format!("^{}$", escaped))?;
        Ok(Self {
            pattern: pattern.to_string(),
            regex,
        })
    }

    fn matches(&self, domain: &str) -> bool {
        self.regex.is_match(domain)
    }
}

impl HotlinkProtector {
    pub fn new(config: Arc<HotlinkConfig>) -> Self {
        let allowed_patterns = config
            .allowed_referers
            .iter()
            .filter_map(|p| WildcardPattern::new(p).ok())
            .collect();

        Self {
            config,
            allowed_patterns,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    pub fn check(&self, referer: Option<&str>) -> HotlinkResult {
        if !self.config.enabled {
            return HotlinkResult::Allowed;
        }

        let referer = match referer {
            Some(r) => r,
            None => {
                if self.config.allow_direct {
                    return HotlinkResult::Allowed;
                } else {
                    return HotlinkResult::Denied;
                }
            }
        };

        let domain = extract_domain(referer);

        if let Some(domain) = domain {
            for pattern in &self.allowed_patterns {
                if pattern.matches(&domain) {
                    return HotlinkResult::Allowed;
                }
            }
        }

        HotlinkResult::Denied
    }

    pub fn fallback_url(&self) -> Option<&str> {
        self.config.fallback_url.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotlinkResult {
    Allowed,
    Denied,
}

fn extract_domain(url: &str) -> Option<String> {
    let url = url.trim();

    if let Some(rest) = url.strip_prefix("http://") {
        extract_domain_from_host(rest)
    } else if let Some(rest) = url.strip_prefix("https://") {
        extract_domain_from_host(rest)
    } else if let Some(rest) = url.strip_prefix("//") {
        extract_domain_from_host(rest)
    } else {
        extract_domain_from_host(url)
    }
}

fn extract_domain_from_host(host_and_path: &str) -> Option<String> {
    let host = host_and_path.split('/').next()?;
    let host = host.split(':').next()?;
    Some(host.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_domain() {
        assert_eq!(
            extract_domain("https://example.com/path"),
            Some("example.com".to_string())
        );
        assert_eq!(
            extract_domain("http://sub.example.com:8080/path"),
            Some("sub.example.com".to_string())
        );
        assert_eq!(
            extract_domain("//cdn.example.com/asset.js"),
            Some("cdn.example.com".to_string())
        );
    }

    #[test]
    fn test_wildcard_pattern() {
        let pattern = WildcardPattern::new("*.example.com").unwrap();
        assert!(pattern.matches("cdn.example.com"));
        assert!(pattern.matches("sub.example.com"));
        assert!(!pattern.matches("example.com"));
    }

    #[test]
    fn test_hotlink_allow_direct() {
        let mut config = HotlinkConfig::default();
        config.enabled = true;
        config.allow_direct = true;
        let protector = HotlinkProtector::new(Arc::new(config));

        assert_eq!(protector.check(None), HotlinkResult::Allowed);
    }

    #[test]
    fn test_hotlink_deny_direct() {
        let mut config = HotlinkConfig::default();
        config.enabled = true;
        config.allow_direct = false;
        let protector = HotlinkProtector::new(Arc::new(config));

        assert_eq!(protector.check(None), HotlinkResult::Denied);
    }

    #[test]
    fn test_hotlink_whitelist() {
        let mut config = HotlinkConfig::default();
        config.enabled = true;
        config.allowed_referers = vec!["*.example.com".to_string()];
        let protector = HotlinkProtector::new(Arc::new(config));

        assert_eq!(
            protector.check(Some("https://cdn.example.com/page")),
            HotlinkResult::Allowed
        );
        assert_eq!(
            protector.check(Some("https://evil.com/page")),
            HotlinkResult::Denied
        );
    }
}
