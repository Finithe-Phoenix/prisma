//! Container registry: enumerate / create / remove containers under a root.
//!
//! Status: the directory-backed catalogue the launcher reads. One container =
//! one sub-directory of `root` named by the container name; that sub-directory
//! is the container's Wine prefix. `list` scans the root once and hands back
//! [`Container`] values the Compose UI renders; `create`/`remove` are the
//! lifecycle entry points the UI's "+"/"trash" buttons drive. The Wine backend
//! is still absent — this is the on-disk bookkeeping layer beneath it.

use std::path::{Path, PathBuf};

use thiserror::Error;

use crate::container::{Container, ContainerError};

/// A root directory holding zero or more container prefixes, one per
/// sub-directory.
#[derive(Debug, Clone)]
pub struct ContainerRegistry {
    root: PathBuf,
}

impl ContainerRegistry {
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Ensure the root directory exists. Idempotent; a restart must be able to
    /// open an existing catalogue without clobbering it.
    pub fn ensure_root(&self) -> Result<(), RegistryError> {
        std::fs::create_dir_all(&self.root)
            .map_err(|e| RegistryError::Root(self.root.clone(), e.to_string()))
    }

    /// Prefix path a container of `name` would occupy. Pure; touches no disk.
    #[must_use]
    pub fn prefix_of(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// Enumerate the containers currently on disk, sorted by name for a stable
    /// UI ordering. A missing root is an empty catalogue, not an error — the
    /// launcher shows "no containers yet" rather than failing. Non-directory
    /// entries (stray files) are skipped, never surfaced as containers.
    pub fn list(&self) -> Result<Vec<Container>, RegistryError> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(RegistryError::Root(self.root.clone(), e.to_string())),
        };
        let mut out = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| RegistryError::Root(self.root.clone(), e.to_string()))?;
            if !entry.file_type().is_ok_and(|t| t.is_dir()) {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                out.push(Container::new(name, entry.path()));
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// Look a container up by name. `Ok(None)` if no such prefix exists.
    pub fn get(&self, name: &str) -> Result<Option<Container>, RegistryError> {
        let prefix = self.prefix_of(name);
        if prefix.is_dir() {
            Ok(Some(Container::new(name, prefix)))
        } else {
            Ok(None)
        }
    }

    /// Create a new container named `name`. Rejects a name that would escape
    /// the root (path separators / `..` / empty) so a crafted name can never
    /// write outside the catalogue, and rejects a name already taken.
    pub fn create(&self, name: &str) -> Result<Container, RegistryError> {
        validate_name(name)?;
        let prefix = self.prefix_of(name);
        if prefix.exists() {
            return Err(RegistryError::AlreadyExists(name.to_owned()));
        }
        let container = Container::new(name, prefix);
        container.create().map_err(RegistryError::Lifecycle)?;
        Ok(container)
    }

    /// Remove a container by name: tear its whole prefix down. Idempotent — a
    /// name that is already gone is a clean no-op, so a half-finished delete
    /// followed by a restart converges to "gone" rather than wedging.
    pub fn remove(&self, name: &str) -> Result<(), RegistryError> {
        validate_name(name)?;
        Container::new(name, self.prefix_of(name))
            .destroy()
            .map_err(RegistryError::Lifecycle)
    }
}

/// Reject names that are empty, absolute, or contain a path separator or `..`
/// component — anything that could resolve outside `root`.
fn validate_name(name: &str) -> Result<(), RegistryError> {
    let bad = name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || Path::new(name).components().count() != 1;
    if bad {
        return Err(RegistryError::InvalidName(name.to_owned()));
    }
    Ok(())
}

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("registry root {0} unusable: {1}")]
    Root(PathBuf, String),

    #[error("container name is not a single safe path component: {0:?}")]
    InvalidName(String),

    #[error("container already exists: {0}")]
    AlreadyExists(String),

    #[error(transparent)]
    Lifecycle(ContainerError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg() -> (tempfile::TempDir, ContainerRegistry) {
        let tmp = tempfile::TempDir::new().unwrap();
        let r = ContainerRegistry::new(tmp.path().join("containers"));
        (tmp, r)
    }

    #[test]
    fn list_on_missing_root_is_empty_not_error() {
        let (_t, r) = reg();
        assert!(r.list().unwrap().is_empty());
    }

    #[test]
    fn create_then_list_then_get() {
        let (_t, r) = reg();
        let c = r.create("default").expect("create");
        assert!(c.prefix_path.is_dir());
        let listed = r.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "default");
        assert!(r.get("default").unwrap().is_some());
        assert!(r.get("absent").unwrap().is_none());
    }

    #[test]
    fn list_is_sorted_and_skips_stray_files() {
        let (_t, r) = reg();
        r.create("zeta").unwrap();
        r.create("alpha").unwrap();
        // A stray file directly under the root is not a container.
        std::fs::write(r.root().join("README.txt"), b"x").unwrap();
        let names: Vec<_> = r.list().unwrap().into_iter().map(|c| c.name).collect();
        assert_eq!(names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn create_rejects_duplicate() {
        let (_t, r) = reg();
        r.create("dup").unwrap();
        assert!(matches!(
            r.create("dup"),
            Err(RegistryError::AlreadyExists(_))
        ));
    }

    #[test]
    fn create_rejects_path_escaping_names() {
        let (_t, r) = reg();
        for bad in ["", ".", "..", "a/b", "a\\b", "../escape"] {
            assert!(
                matches!(r.create(bad), Err(RegistryError::InvalidName(_))),
                "name {bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn remove_tears_down_prefix_and_is_idempotent() {
        let (_t, r) = reg();
        let c = r.create("temp").unwrap();
        std::fs::write(c.prefix_path.join("drive_c.txt"), b"data").unwrap();
        r.remove("temp").expect("remove");
        assert!(!c.prefix_path.exists());
        // Second remove is a clean no-op (restart-safe convergence).
        r.remove("temp").expect("idempotent remove");
    }

    #[test]
    fn ensure_root_is_idempotent() {
        let (_t, r) = reg();
        r.ensure_root().expect("first");
        r.ensure_root().expect("second");
        assert!(r.root().is_dir());
    }
}
