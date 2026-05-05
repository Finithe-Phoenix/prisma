//! TOML-backed per-container configuration.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContainerConfig {
    pub wine_version:  String,
    pub dxvk_version:  Option<String>,
    pub vkd3d_version: Option<String>,
    pub use_esync:     bool,
    pub use_fsync:     bool,
}

impl Default for ContainerConfig {
    fn default() -> Self {
        Self {
            wine_version:  "9.0".to_owned(),
            dxvk_version:  Some("2.4".to_owned()),
            vkd3d_version: Some("2.13".to_owned()),
            use_esync:     true,
            use_fsync:     true,
        }
    }
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
            wine_version:  "9.0".to_owned(),
            dxvk_version:  None,
            vkd3d_version: None,
            use_esync:     false,
            use_fsync:     false,
        };
        let s = toml::to_string(&cfg).unwrap();
        let restored: ContainerConfig = toml::from_str(&s).unwrap();
        assert_eq!(cfg, restored);
    }
}
