//! A workspace as the files a digest of it covers: no build output and no empty directories.
use eyre::{Result, WrapErr, bail};
use globset::GlobSet;
use std::{
    fs::{self, FileType},
    os::unix::fs::symlink,
    path::{Path, PathBuf},
};

/// Copies the files and symlinks under `from` that `ignore` does not match to `to`, so two
/// workspaces with the same digest copy the same. Symlinks are copied, never followed.
pub(crate) fn copy(from: &Path, to: &Path, ignore: &GlobSet) -> Result<()> {
    fs::create_dir_all(to).wrap_err_with(|| format!("creating {}", to.display()))?;
    for (rel, kind) in files(from, ignore)? {
        let (source, target) = (from.join(&rel), to.join(&rel));
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)
                .wrap_err_with(|| format!("creating {}", parent.display()))?;
        }
        if kind.is_symlink() {
            let link =
                fs::read_link(&source).wrap_err_with(|| format!("reading {}", source.display()))?;
            symlink(&link, &target).wrap_err_with(|| format!("creating {}", target.display()))?;
        } else {
            fs::copy(&source, &target).wrap_err_with(|| {
                format!("copying {} to {}", source.display(), target.display())
            })?;
        }
    }
    Ok(())
}

/// The files and symlinks under `root` that `ignore` does not match, by path relative to
/// `root`, in sorted order. Anything but a file, directory or symlink is an error.
pub(super) fn files(root: &Path, ignore: &GlobSet) -> Result<Vec<(PathBuf, FileType)>> {
    let mut found = Vec::new();
    let mut pending = vec![PathBuf::new()];
    while let Some(dir) = pending.pop() {
        let abs = root.join(&dir);
        let entries = fs::read_dir(&abs)
            .and_then(|listing| listing.collect::<Result<Vec<_>, _>>())
            .wrap_err_with(|| format!("listing {}", abs.display()))?;
        for entry in entries {
            let rel = dir.join(entry.file_name());
            if ignore.is_match(&rel) {
                continue;
            }
            let kind = entry
                .file_type()
                .wrap_err_with(|| format!("inspecting {}", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(rel);
            } else if kind.is_file() || kind.is_symlink() {
                found.push((rel, kind));
            } else {
                bail!(
                    "{} is not a file, directory or symlink",
                    entry.path().display()
                );
            }
        }
    }
    found.sort_by(|(a, _), (b, _)| a.cmp(b));
    Ok(found)
}
