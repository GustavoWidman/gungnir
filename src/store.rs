//! Durable files: content digests and crash-safe writes.
mod digest;

pub use digest::Digest;

use eyre::{Result, WrapErr};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::{ErrorKind, Write},
    path::Path,
};
use tempfile::NamedTempFile;

/// Replaces `path` with `bytes` so that a reader, or a crash at any point, sees either the
/// old file or the new one, never a mix. Creates missing parent directories.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = parent(path);
    fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
    let mut temp = NamedTempFile::new_in(dir)
        .wrap_err_with(|| format!("creating a file in {}", dir.display()))?;
    temp.write_all(bytes)
        .and_then(|()| temp.as_file().sync_all())
        .wrap_err_with(|| format!("writing {}", temp.path().display()))?;
    temp.persist(path)
        .wrap_err_with(|| format!("replacing {}", path.display()))?;
    sync_dir(dir)
}

/// Writes `value` as pretty JSON through [`write_atomic`].
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .wrap_err_with(|| format!("encoding {}", path.display()))?;
    bytes.push(b'\n');
    write_atomic(path, &bytes)
}

/// Reads a JSON file, or `None` when it does not exist. A file that exists but does not
/// parse is an error, never treated as missing.
pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err).wrap_err_with(|| format!("reading {}", path.display())),
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .wrap_err_with(|| format!("parsing {}", path.display()))
}

fn sync_dir(dir: &Path) -> Result<()> {
    File::open(dir)
        .and_then(|file| file.sync_all())
        .wrap_err_with(|| format!("syncing {}", dir.display()))
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use tempfile::TempDir;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Record {
        name: String,
        count: u32,
    }

    #[test]
    fn json_round_trips_and_creates_parents() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sessions/s1.json");
        let record = Record {
            name: "pilot".into(),
            count: 3,
        };
        write_json(&path, &record).unwrap();
        assert_eq!(read_json::<Record>(&path).unwrap(), Some(record));
    }

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp_files() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("record.json");
        write_atomic(&path, b"old").unwrap();
        write_atomic(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn missing_json_is_none_and_corrupt_json_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("record.json");
        assert_eq!(read_json::<Record>(&path).unwrap(), None);

        fs::write(&path, "{\"name\": ").unwrap();
        let err = read_json::<Record>(&path).unwrap_err();
        assert!(format!("{err}").contains("record.json"), "{err}");
    }
}
