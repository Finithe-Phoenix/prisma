//! Container lifecycle + value type.
//!
//! Status: `create` / `validate` / `destroy` manage the prefix directory on
//! disk (create the tree, check it, and tear it down completely — a restart
//! must not inherit a half-deleted prefix). `start` / `stop` still return
//! `ContainerError::NotImplemented` — they need the Wine backend. The shape is
//! fixed so JNI marshalling and the Compose UI know what to expect.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Container {
    pub name: String,
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

    /// Create the prefix directory tree. Idempotent — succeeds if it already
    /// exists as a directory; errors only if the path is unusable (e.g. it
    /// exists as a file or the parent is not writable).
    pub fn create(&self) -> Result<(), ContainerError> {
        std::fs::create_dir_all(&self.prefix_path)
            .map_err(|e| ContainerError::PrefixCreate(self.prefix_path.clone(), e.to_string()))
    }

    /// Tear the container down: remove the entire prefix directory tree.
    ///
    /// A restart must never inherit a half-deleted prefix, so this removes the
    /// whole tree or reports the failure — it never leaves a partial state
    /// silently. Idempotent: a prefix that is already gone is a clean no-op.
    pub fn destroy(&self) -> Result<(), ContainerError> {
        if !self.prefix_path.exists() {
            return Ok(());
        }
        std::fs::remove_dir_all(&self.prefix_path)
            .map_err(|e| ContainerError::PrefixRemove(self.prefix_path.clone(), e.to_string()))
    }

    pub const fn start(&self) -> Result<(), ContainerError> {
        Err(ContainerError::NotImplemented("Container::start"))
    }
    pub const fn stop(&self) -> Result<(), ContainerError> {
        Err(ContainerError::NotImplemented("Container::stop"))
    }
}

#[derive(Debug, Error)]
pub enum ContainerError {
    #[error("prefix directory does not exist: {0}")]
    PrefixMissing(PathBuf),

    #[error("failed to create prefix directory {0}: {1}")]
    PrefixCreate(PathBuf, String),

    #[error("failed to remove prefix directory {0}: {1}")]
    PrefixRemove(PathBuf, String),

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
        // Use the platform temp dir, which exists and is readable on
        // every supported host (Unix /tmp, Windows %TEMP%) — a literal
        // "/tmp" does not exist on the Windows CI runner.
        let c = Container::new("ok", std::env::temp_dir());
        assert!(c.validate().is_ok());
    }

    #[test]
    fn start_returns_not_implemented_for_now() {
        let c = Container::new("x", "/tmp");
        assert!(matches!(c.start(), Err(ContainerError::NotImplemented(_))));
    }

    #[test]
    fn create_then_validate_then_destroy_roundtrips() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prefix = tmp.path().join("prefix-a");
        let c = Container::new("a", &prefix);
        assert!(!prefix.exists());
        c.create().expect("create");
        assert!(prefix.is_dir());
        c.validate().expect("validate after create");
        c.destroy().expect("destroy");
        assert!(!prefix.exists());
    }

    #[test]
    fn create_is_idempotent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = Container::new("a", tmp.path().join("prefix"));
        c.create().expect("first create");
        c.create().expect("second create is a no-op");
    }

    #[test]
    fn destroy_is_idempotent_when_absent() {
        let tmp = tempfile::TempDir::new().unwrap();
        let c = Container::new("gone", tmp.path().join("never-created"));
        assert!(c.destroy().is_ok());
    }

    #[test]
    fn destroy_removes_a_populated_prefix_completely() {
        // The resource-discipline contract: tearing a container down leaves
        // nothing behind for a restart to inherit.
        let tmp = tempfile::TempDir::new().unwrap();
        let prefix = tmp.path().join("prefix-b");
        let c = Container::new("b", &prefix);
        c.create().expect("create");
        std::fs::write(prefix.join("drive_c.txt"), b"data").unwrap();
        std::fs::create_dir(prefix.join("sub")).unwrap();
        std::fs::write(prefix.join("sub").join("nested"), b"x").unwrap();
        c.destroy().expect("destroy populated");
        assert!(!prefix.exists());
    }
}
