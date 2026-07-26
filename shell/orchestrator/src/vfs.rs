use std::collections::HashSet;
use std::fs::{self, File};
use std::io;
use std::path::{Component, Path, PathBuf};

pub struct VirtualFileSystem {
    base_path: PathBuf,
    overlay_path: PathBuf,
}

impl VirtualFileSystem {
    pub fn new<P1: Into<PathBuf>, P2: Into<PathBuf>>(base_path: P1, overlay_path: P2) -> Self {
        Self {
            base_path: base_path.into(),
            overlay_path: overlay_path.into(),
        }
    }

    fn normalize_win_path(win_path: &str) -> Vec<String> {
        let win_path = win_path.replace('\\', "/");
        let path = Path::new(&win_path);
        let mut components = Vec::new();
        for comp in path.components() {
            match comp {
                Component::Normal(s) => {
                    if let Some(s) = s.to_str() {
                        components.push(s.to_string());
                    }
                }
                Component::Prefix(_) | Component::RootDir => {}
                _ => {}
            }
        }
        components
    }

    fn resolve_case_insensitive(root: &Path, components: &[String]) -> Option<PathBuf> {
        if components.is_empty() {
            return Some(root.to_path_buf());
        }

        let current_comp = &components[0];
        let current_comp_lower = current_comp.to_lowercase();

        if let Ok(entries) = fs::read_dir(root) {
            for entry in entries.flatten() {
                if let Ok(file_name) = entry.file_name().into_string() {
                    if file_name.to_lowercase() == current_comp_lower {
                        let next_path = entry.path();
                        if components.len() == 1 {
                            return Some(next_path);
                        } else {
                            if let Some(resolved) =
                                Self::resolve_case_insensitive(&next_path, &components[1..])
                            {
                                return Some(resolved);
                            }
                        }
                    }
                }
            }
        }
        None
    }

    pub fn resolve_path(&self, win_path: &str) -> Option<PathBuf> {
        let components = Self::normalize_win_path(win_path);

        if let Some(path) = Self::resolve_case_insensitive(&self.overlay_path, &components) {
            return Some(path);
        }

        if let Some(path) = Self::resolve_case_insensitive(&self.base_path, &components) {
            return Some(path);
        }

        None
    }

    pub fn open_file(&self, win_path: &str) -> io::Result<File> {
        if let Some(path) = self.resolve_path(win_path) {
            File::open(path)
        } else {
            Err(io::Error::new(io::ErrorKind::NotFound, "File not found"))
        }
    }

    pub fn create_file(&self, win_path: &str) -> io::Result<File> {
        let components = Self::normalize_win_path(win_path);
        if components.is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "Invalid path"));
        }

        let mut current_dir = self.overlay_path.clone();
        for comp in &components[..components.len() - 1] {
            let mut found = false;
            if let Ok(entries) = fs::read_dir(&current_dir) {
                for entry in entries.flatten() {
                    if let Ok(file_name) = entry.file_name().into_string() {
                        if file_name.to_lowercase() == comp.to_lowercase() {
                            current_dir = entry.path();
                            found = true;
                            break;
                        }
                    }
                }
            }
            if !found {
                current_dir.push(comp);
                fs::create_dir_all(&current_dir)?;
            }
        }

        let file_name = components.last().unwrap();
        current_dir.push(file_name);
        File::create(current_dir)
    }

    pub fn read_dir(&self, win_path: &str) -> io::Result<Vec<String>> {
        let components = Self::normalize_win_path(win_path);
        let mut results = HashSet::new();
        let mut dir_found = false;

        if let Some(overlay_dir) = Self::resolve_case_insensitive(&self.overlay_path, &components) {
            if let Ok(entries) = fs::read_dir(overlay_dir) {
                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        results.insert(name);
                    }
                }
                dir_found = true;
            }
        }

        if let Some(base_dir) = Self::resolve_case_insensitive(&self.base_path, &components) {
            if let Ok(entries) = fs::read_dir(base_dir) {
                for entry in entries.flatten() {
                    if let Ok(name) = entry.file_name().into_string() {
                        let name_lower = name.to_lowercase();
                        if !results.iter().any(|r| r.to_lowercase() == name_lower) {
                            results.insert(name);
                        }
                    }
                }
                dir_found = true;
            }
        }

        if dir_found {
            Ok(results.into_iter().collect())
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "Directory not found",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_vfs_resolve_case_insensitive() {
        let base_dir = tempdir().unwrap();
        let overlay_dir = tempdir().unwrap();

        let base_path = base_dir.path();
        let overlay_path = overlay_dir.path();

        let base_subdir = base_path.join("BaseDir");
        fs::create_dir(&base_subdir).unwrap();
        fs::write(base_subdir.join("Test.txt"), "base").unwrap();

        let vfs = VirtualFileSystem::new(base_path, overlay_path);

        let resolved = vfs.resolve_path("C:\\basedir\\TEST.txt").unwrap();
        assert_eq!(fs::read_to_string(resolved).unwrap(), "base");
    }

    #[test]
    fn test_vfs_overlay_precedence() {
        let base_dir = tempdir().unwrap();
        let overlay_dir = tempdir().unwrap();

        let base_path = base_dir.path();
        let overlay_path = overlay_dir.path();

        let base_subdir = base_path.join("Windows").join("system32");
        fs::create_dir_all(&base_subdir).unwrap();
        fs::write(base_subdir.join("file.dll"), "base_content").unwrap();

        let overlay_subdir = overlay_path.join("windows").join("System32");
        fs::create_dir_all(&overlay_subdir).unwrap();
        fs::write(overlay_subdir.join("FILE.dll"), "overlay_content").unwrap();

        let vfs = VirtualFileSystem::new(base_path, overlay_path);

        let resolved = vfs.resolve_path("C:\\Windows\\System32\\file.dll").unwrap();
        assert_eq!(fs::read_to_string(resolved).unwrap(), "overlay_content");
    }

    #[test]
    fn test_create_file() {
        let base_dir = tempdir().unwrap();
        let overlay_dir = tempdir().unwrap();

        let vfs = VirtualFileSystem::new(base_dir.path(), overlay_dir.path());

        vfs.create_file("C:\\Users\\test\\new_file.txt").unwrap();

        let mut overlay_path = overlay_dir.path().to_path_buf();
        overlay_path.push("Users");
        overlay_path.push("test");
        overlay_path.push("new_file.txt");
        assert!(overlay_path.exists());
    }

    #[test]
    fn test_read_dir() {
        let base_dir = tempdir().unwrap();
        let overlay_dir = tempdir().unwrap();

        let base_path = base_dir.path();
        let overlay_path = overlay_dir.path();

        let base_subdir = base_path.join("folder");
        fs::create_dir_all(&base_subdir).unwrap();
        fs::write(base_subdir.join("file1.txt"), "").unwrap();
        fs::write(base_subdir.join("file2.txt"), "").unwrap();

        let overlay_subdir = overlay_path.join("FOLDER");
        fs::create_dir_all(&overlay_subdir).unwrap();
        fs::write(overlay_subdir.join("FILE1.txt"), "").unwrap();
        fs::write(overlay_subdir.join("file3.txt"), "").unwrap();

        let vfs = VirtualFileSystem::new(base_path, overlay_path);

        let mut files = vfs.read_dir("C:\\Folder").unwrap();
        files.sort();

        assert_eq!(files.len(), 3);

        let lower_files: Vec<String> = files.iter().map(|s| s.to_lowercase()).collect();
        assert!(lower_files.contains(&"file1.txt".to_string()));
        assert!(lower_files.contains(&"file2.txt".to_string()));
        assert!(lower_files.contains(&"file3.txt".to_string()));
    }
}
