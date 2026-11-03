//! Fixture helpers shared by the unit tests.
use std::{fs, path::Path};

/// Writes `contents` to `root/rel`, creating parent directories.
pub fn write(root: &Path, rel: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}
