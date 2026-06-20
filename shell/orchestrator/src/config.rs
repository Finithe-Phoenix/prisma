//! TOML-backed per-container configuration.

use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerConfig {
    pub wine_version: String,
    pub dxvk_version: Option<String>,
    pub vkd3d_version: Option<String>,
    pub use_esync: bool,
    pub use_fsync: bool,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            wine_version: "9.0".to_owned(),
            dxvk_version: Some("2.4".to_owned()),
            vkd3d_version: Some("2.13".to_owned()),
            use_esync: true,
            use_fsync: true,
        }
    }
}

impl ContainerConfig {
    /// Conventional file name for a container's config inside its prefix.
    pub const FILE_NAME: &str = "prisma-container.toml";

    /// Persist the config to `path` as TOML.
    ///
    /// Writes atomically — serialize to a sibling `*.tmp` then rename over the
    /// target — so a crash mid-write never leaves a truncated config behind for
    /// a restart to read. The temp file is flushed and closed before the rename.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let path = path.as_ref();
        let toml = toml::to_string(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml.as_bytes()).map_err(|e| ConfigError::Io(e.to_string()))?;
        std::fs::rename(&tmp, path).map_err(|e| ConfigError::Io(e.to_string()))?;
        Ok(())
    }

    /// Load and parse a config from `path`.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let s = std::fs::read_to_string(path).map_err(|e| ConfigError::Io(e.to_string()))?;
        toml::from_str(&s).map_err(|e| ConfigError::Parse(e.to_string()))
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("config I/O error: {0}")]
    Io(String),

    #[error("config serialize error: {0}")]
    Serialize(String),

    #[error("config parse error: {0}")]
    Parse(String),
}

/// Serialize → TOML → parse → equal. Round-trip pin so
/// schema drift surfaces immediately.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_round_trips_through_toml() {
        let original = ContainerConfig::default();
        let s = toml::to_string(&original).expect("serialize");
        let restored: ContainerConfig = toml::from_str(&s).expect("parse");
        assert_eq!(original, restored);
    }

    #[test]
    fn custom_dxvk_version_survives() {
        let cfg = ContainerConfig {
            dxvk_version: Some("2.5".to_owned()),
            ..ContainerConfig::default()
        };
        let s = toml::to_string(&cfg).unwrap();
        let restored: ContainerConfig = toml::from_str(&s).unwrap();
        assert_eq!(restored.dxvk_version.as_deref(), Some("2.5"));
    }

    #[test]
    fn missing_optional_fields_round_trip() {
        let cfg = ContainerConfig {
            wine_version: "9.0".to_owned(),
            dxvk_version: None,
            vkd3d_version: None,
            use_esync: false,
            use_fsync: false,
        };
        let s = toml::to_string(&cfg).unwrap();
        let restored: ContainerConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, restored);
    }

    #[test]
    fn save_then_load_from_file_roundtrips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(ContainerConfig::FILE_NAME);
        let cfg = ContainerConfig {
            dxvk_version: Some("2.5".to_owned()),
            use_fsync: false,
            ..ContainerConfig::default()
        };
        cfg.save_to_file(&path).expect("save");
        assert!(path.exists());
        let loaded = ContainerConfig::load_from_file(&path).expect("load");
        assert_eq!(cfg, loaded);
    }

    #[test]
    fn save_leaves_no_temp_file_behind() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join(ContainerConfig::FILE_NAME);
        ContainerConfig::default()
            .save_to_file(&path)
            .expect("save");
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn load_missing_file_errors() {
        let r = ContainerConfig::load_from_file("/no/such/dir/prisma-container.toml");
        assert!(matches!(r, Err(ConfigError::Io(_))));
    }

    #[test]
    fn load_malformed_toml_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("bad.toml");
        std::fs::write(&path, b"not valid = = = toml [[[").unwrap();
        let r = ContainerConfig::load_from_file(&path);
        assert!(matches!(r, Err(ConfigError::Parse(_))));
    }
}
