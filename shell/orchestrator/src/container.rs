//! Container lifecycle + value type.
//!
//! Status: skeleton. `Container::new` constructs the value, but
//! `start` / `stop` / `destroy` aren't wired to any backend yet —
//! they return `ContainerError::NotImplemented`. The shape is fixed
//! so JNI marshalling and the Compose UI know what to expect.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Container {
    pub name:        String,
    pub prefix_path: PathBuf,
    /// Number of installed Windows applications inside the prefix.
    /// Bumped by the importer; consumed by the launcher UI.
    pub installed_app_count: u32,
}

impl Container {
    pub fn new<P: Into<PathBuf>>(name: impl Into<String>, prefix_path: P) -> Self {
        Self {
            name: name.into(),
            prefix_path: prefix_path.into(),
            installed_app_count: 0,
        }
    }

    /// Validate the prefix directory exists and is readable. Used by
    /// the launcher before showing a Container in the UI list.
    pub fn validate(&self) -> Result<(), ContainerError> {
        if !Path::new(&self.prefix_path).is_dir() {
            return Err(ContainerError::PrefixMissing(self.prefix_path.clone()));
        }
        Ok(())
    }

    pub fn start(&self) -> Result<(), ContainerError> {
        Err(ContainerError::NotImplemented("Container::start"))
    }
    pub fn stop(&self) -> Result<(), ContainerError> {
        Err(ContainerError::NotImplemented("Container::stop"))
    }
    pub fn destroy(&self) -> Result<(), ContainerError> {
        Err(ContainerError::NotImplemented("Container::destroy"))
    }
}

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("prefix directory does not exist: {0}")]
    PrefixMissing(PathBuf),

    #[error("operation not implemented yet: {0}")]
    NotImplemented(&'static str),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_constructs_with_zero_installed_apps() {
        let c = Container::new("default", "/tmp/wine-default");
        assert_eq!(c.name, "default");
        assert_eq!(c.installed_app_count, 0);
    }

    #[test]
    fn validate_rejects_missing_prefix() {
        let c = Container::new("bogus", "/no/such/path/12345");
        assert!(matches!(
            c.validate(),
            Err(ContainerError::PrefixMissing(_))
        ));
    }

    #[test]
    fn validate_accepts_existing_prefix() {
        // Use a known-existing dir; on every supported host /tmp
        // exists and is readable.
        let c = Container::new("ok", "/tmp");
        assert!(c.validate().is_ok());
    }

    #[test]
    fn start_returns_not_implemented_for_now() {
        let c = Container::new("x", "/tmp");
        assert!(matches!(c.start(), Err(ContainerError::NotImplemented(_))));
    }
}
