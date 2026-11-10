//! The partial-directory commit protocol. A phase builds `<dir>.partial/`, writes
//! `record.json` last, and renames the whole directory to `<dir>/`, so `<dir>/` is either
//! complete or absent. A committed record may be rewritten in place, atomically.
use super::{json, parent, read_json, replace, sync_dir, write_json};
use eyre::{Result, WrapErr, ensure};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::{self, File},
    io::ErrorKind,
    path::{Path, PathBuf},
};

const RECORD: &str = "record.json";

/// A directory under construction beside its final path, which stays absent until
/// [`PartialDir::commit`] renames the directory into place.
#[derive(Debug)]
pub struct PartialDir {
    partial: PathBuf,
    target: PathBuf,
}

impl PartialDir {
    /// Starts building `target`, discarding whatever an interrupted earlier run left in its
    /// partial directory. Fails if `target` is already committed.
    pub fn begin(target: &Path) -> Result<Self> {
        ensure!(
            !target.exists(),
            "{} is already committed",
            target.display()
        );
        sweep_partial(target)?;
        let partial = partial_path(target);
        fs::create_dir_all(&partial).wrap_err_with(|| format!("creating {}", partial.display()))?;
        Ok(Self {
            partial,
            target: target.to_owned(),
        })
    }

    /// Where to build the directory's contents.
    pub fn path(&self) -> &Path {
        &self.partial
    }

    /// Writes `record` as `record.json`, syncs everything to disk, and publishes the
    /// directory at its final path.
    pub fn commit(self, record: &impl Serialize) -> Result<()> {
        write_json(&self.partial.join(RECORD), record)?;
        sync_tree(&self.partial)?;
        fs::rename(&self.partial, &self.target).wrap_err_with(|| {
            format!(
                "renaming {} to {}",
                self.partial.display(),
                self.target.display()
            )
        })?;
        sync_dir(parent(&self.target))
    }
}

/// Removes the partial directory an interrupted run left beside `target`. Returns whether
/// there was one.
pub fn sweep_partial(target: &Path) -> Result<bool> {
    let partial = partial_path(target);
    match fs::remove_dir_all(&partial) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err).wrap_err_with(|| format!("removing {}", partial.display())),
    }
}

/// The record of the committed directory `dir`, or `None` when `dir` is not committed.
pub fn read_record<T: DeserializeOwned>(dir: &Path) -> Result<Option<T>> {
    read_json(&dir.join(RECORD))
}

/// Replaces the record of the committed directory `dir` atomically, so a reader sees the old
/// record or the new one. Only a record re-derived from files already on disk is rewritten.
/// Fails, writing nothing, when `dir` is not committed, so a directory another phase
/// withdrew meanwhile is never recreated.
pub fn rewrite_record(dir: &Path, record: &impl Serialize) -> Result<()> {
    let path = dir.join(RECORD);
    ensure!(path.is_file(), "{} is not committed", dir.display());
    replace(&path, &json(&path, record)?)
}

fn partial_path(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_owned();
    name.push(".partial");
    target.with_file_name(name)
}

fn sync_tree(dir: &Path) -> Result<()> {
    let listing = fs::read_dir(dir).wrap_err_with(|| format!("listing {}", dir.display()))?;
    for entry in listing {
        let path = entry
            .wrap_err_with(|| format!("listing {}", dir.display()))?
            .path();
        let kind = fs::symlink_metadata(&path)
            .wrap_err_with(|| format!("inspecting {}", path.display()))?
            .file_type();
        if kind.is_dir() {
            sync_tree(&path)?;
        } else if kind.is_file() {
            File::open(&path)
                .and_then(|file| file.sync_all())
                .wrap_err_with(|| format!("syncing {}", path.display()))?;
        }
    }
    sync_dir(dir)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write;
    use serde::Deserialize;
    use tempfile::TempDir;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Record {
        exit: i32,
    }

    #[test]
    fn a_directory_appears_only_when_committed_and_complete() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("trial/run");

        let partial = PartialDir::begin(&target).unwrap();
        write(partial.path(), "final/main.py", "print()");
        assert!(!target.exists());
        assert_eq!(read_record::<Record>(&target).unwrap(), None);

        partial.commit(&Record { exit: 0 }).unwrap();
        assert_eq!(read_record(&target).unwrap(), Some(Record { exit: 0 }));
        assert_eq!(
            fs::read_to_string(target.join("final/main.py")).unwrap(),
            "print()"
        );
        let entries: Vec<_> = fs::read_dir(root.path().join("trial"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, ["run"]);
    }

    #[test]
    fn a_committed_directory_is_never_rebuilt() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("grade");
        PartialDir::begin(&target)
            .unwrap()
            .commit(&Record { exit: 1 })
            .unwrap();
        let err = PartialDir::begin(&target).unwrap_err();
        assert!(err.to_string().contains("already committed"), "{err}");
        assert_eq!(read_record(&target).unwrap(), Some(Record { exit: 1 }));
    }

    #[test]
    fn a_committed_record_is_rewritten_in_place_and_a_missing_one_never_written() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("grade");
        let partial = PartialDir::begin(&target).unwrap();
        write(partial.path(), "build.log", "built");
        partial.commit(&Record { exit: 0 }).unwrap();

        rewrite_record(&target, &Record { exit: 1 }).unwrap();
        assert_eq!(read_record(&target).unwrap(), Some(Record { exit: 1 }));
        let mut entries: Vec<_> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        assert_eq!(entries, ["build.log", "record.json"]);

        let withdrawn = root.path().join("measure");
        let err = rewrite_record(&withdrawn, &Record { exit: 1 }).unwrap_err();
        assert!(err.to_string().contains("not committed"), "{err}");
        assert!(!withdrawn.exists());
    }

    #[test]
    fn an_interrupted_build_leaves_nothing_committed_and_is_swept() {
        let root = TempDir::new().unwrap();
        let target = root.path().join("measure");

        let interrupted = PartialDir::begin(&target).unwrap();
        write(interrupted.path(), "stale.json", "{}");
        drop(interrupted);
        assert!(!target.exists());

        let retry = PartialDir::begin(&target).unwrap();
        assert!(!retry.path().join("stale.json").exists());
        drop(retry);

        assert!(sweep_partial(&target).unwrap());
        assert!(!sweep_partial(&target).unwrap());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 0);
    }
}
