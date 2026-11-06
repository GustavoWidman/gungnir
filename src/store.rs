//! Files on disk: content digests, crash-safe writes, directory commits and replacements,
//! locks, and the readers every loader shares.
mod digest;
mod lock;
mod partial;
mod replace;

pub use digest::Digest;
pub use lock::Lock;
pub use partial::{PartialDir, read_record, rewrite_record, sweep_partial};
pub use replace::{MANIFEST, replace_dir};

use eyre::{Report, Result, WrapErr, eyre};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    str::FromStr,
};
use tempfile::NamedTempFile;

/// Replaces `path` with `bytes` so that a reader, or a crash at any point, sees either the
/// old file or the new one, never a mix. Creates missing parent directories.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = parent(path);
    fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
    replace(path, bytes)
}

/// Replaces `path` with `bytes` as [`write_atomic`] does, in a directory that must exist.
fn replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let dir = parent(path);
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
    write_atomic(path, &json(path, value)?)
}

/// `value` as pretty JSON and a final newline, for the file `path`.
fn json(path: &Path, value: &impl Serialize) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .wrap_err_with(|| format!("encoding {}", path.display()))?;
    bytes.push(b'\n');
    Ok(bytes)
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

/// Reads and parses a TOML file, naming the file in any error.
pub(crate) fn read_toml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = fs::read_to_string(path).wrap_err_with(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).wrap_err_with(|| format!("parsing {}", path.display()))
}

/// The subdirectories of `dir` keyed by their names parsed as `T`. Files are skipped and a
/// directory whose name does not parse is an error.
pub(crate) fn subdirs<T>(dir: &Path) -> Result<BTreeMap<T, PathBuf>>
where
    T: FromStr<Err = Report> + Ord,
{
    let mut found = BTreeMap::new();
    let listing = fs::read_dir(dir).wrap_err_with(|| format!("listing {}", dir.display()))?;
    for entry in listing {
        let entry = entry.wrap_err_with(|| format!("listing {}", dir.display()))?;
        let path = entry.path();
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| eyre!("{} is not a UTF-8 name", path.display()))?;
        let id = name
            .parse()
            .wrap_err_with(|| format!("{}", path.display()))?;
        found.insert(id, path);
    }
    Ok(found)
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
