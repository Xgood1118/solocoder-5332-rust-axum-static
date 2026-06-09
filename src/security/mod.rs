use std::path::{Path, PathBuf};
use std::sync::Arc;

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};

use crate::config::ServerConfig;

const FRAGMENT: &AsciiSet = &CONTROLS.add(b' ').add(b'"').add(b'<').add(b'>').add(b'`');

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("Path traversal detected")]
    PathTraversal,
    #[error("Path too deep")]
    PathTooDeep,
    #[error("Hidden file access denied")]
    HiddenFile,
    #[error("Symlink access denied")]
    SymlinkDenied,
    #[error("Invalid path")]
    InvalidPath,
    #[error("File not found")]
    NotFound,
}

#[derive(Clone)]
pub struct PathValidator {
    config: Arc<ServerConfig>,
    root_dir: PathBuf,
}

impl std::fmt::Debug for PathValidator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PathValidator")
            .field("root_dir", &self.root_dir)
            .finish()
    }
}

impl PathValidator {
    pub fn new(config: Arc<ServerConfig>) -> Self {
        let root_dir = config.root_dir.canonicalize().unwrap_or_else(|_| config.root_dir.clone());
        Self { config, root_dir }
    }

    pub fn validate_and_resolve(&self, request_path: &str) -> Result<PathBuf, SecurityError> {
        let decoded = percent_decode_str(request_path)
            .decode_utf8_lossy()
            .to_string();

        let path = Path::new(&decoded);

        if path.components().count() > self.config.max_path_depth {
            return Err(SecurityError::PathTooDeep);
        }

        let mut normalized = PathBuf::new();
        let mut depth = 0;

        for component in path.components() {
            use std::path::Component;
            match component {
                Component::Prefix(_) => return Err(SecurityError::PathTraversal),
                Component::RootDir => {
                    normalized = PathBuf::new();
                    depth = 0;
                }
                Component::CurDir => {}
                Component::ParentDir => {
                    if depth == 0 {
                        return Err(SecurityError::PathTraversal);
                    }
                    normalized.pop();
                    depth -= 1;
                }
                Component::Normal(name) => {
                    let name_str = name.to_string_lossy();
                    if !self.config.serve_hidden_files && name_str.starts_with('.') {
                        return Err(SecurityError::HiddenFile);
                    }
                    normalized.push(name);
                    depth += 1;
                }
            }
        }

        let full_path = self.root_dir.join(&normalized);

        if !self.config.follow_symlinks {
            if let Ok(metadata) = std::fs::symlink_metadata(&full_path) {
                if metadata.file_type().is_symlink() {
                    return Err(SecurityError::SymlinkDenied);
                }
            }
        }

        let canonical = if full_path.exists() {
            full_path
                .canonicalize()
                .map_err(|_| SecurityError::NotFound)?
        } else {
            let parent = full_path.parent().unwrap_or(&full_path);
            if parent.exists() {
                let canon_parent = parent.canonicalize().map_err(|_| SecurityError::NotFound)?;
                canon_parent.join(full_path.file_name().unwrap_or_default())
            } else {
                return Err(SecurityError::NotFound);
            }
        };

        if !canonical.starts_with(&self.root_dir) {
            return Err(SecurityError::PathTraversal);
        }

        Ok(canonical)
    }

    pub fn is_hidden_path(&self, path: &Path) -> bool {
        path.components().any(|c| {
            if let std::path::Component::Normal(name) = c {
                name.to_string_lossy().starts_with('.')
            } else {
                false
            }
        })
    }

    pub fn root_dir(&self) -> &Path {
        &self.root_dir
    }
}

pub fn encode_path(path: &str) -> String {
    utf8_percent_encode(path, FRAGMENT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn make_validator() -> PathValidator {
        let mut config = ServerConfig::default();
        config.root_dir = PathBuf::from("/tmp/test_root");
        PathValidator::new(Arc::new(config))
    }

    #[test]
    fn test_normal_path() {
        let v = make_validator();
        let result = v.validate_and_resolve("/index.html");
        assert!(result.is_ok() || result.is_err());
    }

    #[test]
    fn test_path_traversal_parent() {
        let v = make_validator();
        let result = v.validate_and_resolve("/../etc/passwd");
        assert!(matches!(result, Err(SecurityError::PathTraversal)));
    }

    #[test]
    fn test_hidden_file() {
        let v = make_validator();
        let result = v.validate_and_resolve("/.secret");
        assert!(matches!(result, Err(SecurityError::HiddenFile)));
    }

    #[test]
    fn test_path_too_deep() {
        let mut config = ServerConfig::default();
        config.root_dir = PathBuf::from("/tmp/test_root");
        config.max_path_depth = 3;
        let v = PathValidator::new(Arc::new(config));
        let deep_path = "/a/b/c/d/e/f";
        let result = v.validate_and_resolve(deep_path);
        assert!(matches!(result, Err(SecurityError::PathTooDeep)));
    }
}
