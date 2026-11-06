//! Output directories that are replaced as a whole, such as a report: built beside the old one
//! with their manifest written last, then swapped in.
use super::{write_atomic, write_json};
use eyre::{Result, WrapErr, ensure, eyre};
use serde::Serialize;
use std::{fs, io::ErrorKind, path::Path};

/// Written last, so a directory holding it is whole.
pub const MANIFEST: &str = "manifest.json";

/// Replaces the directory `out` with `files`, each a name and its text, and `manifest`. Builds
/// the new directory in `<out>.partial`, its manifest last, then swaps it in for `out`, so a
/// crash leaves the old directory or the new one at `out`, or neither, with the leftovers beside
/// it swept by the next call. Refuses to replace a directory that is neither empty nor holds a
/// manifest, naming it as not a `kind`.
pub fn replace_dir(
    out: &Path,
    kind: &str,
    files: &[(&str, &str)],
    manifest: &impl Serialize,
) -> Result<()> {
    let name = out
        .file_name()
        .ok_or_else(|| eyre!("{} does not name a directory", out.display()))?;
    let beside = |suffix: &str| {
        let mut sibling = name.to_owned();
        sibling.push(suffix);
        out.with_file_name(sibling)
    };
    let (partial, old) = (beside(".partial"), beside(".old"));
    ensure!(
        is_whole_or_empty(out)?,
        "{} is not a {kind}; refusing to replace it",
        out.display()
    );
    remove(&partial)?;
    remove(&old)?;
    for (file, text) in files {
        write_atomic(&partial.join(file), text.as_bytes())?;
    }
    write_json(&partial.join(MANIFEST), manifest)?;
    match fs::rename(out, &old) {
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        renamed => renamed.wrap_err_with(|| format!("moving {} aside", out.display()))?,
    }
    fs::rename(&partial, out)
        .wrap_err_with(|| format!("renaming {} to {}", partial.display(), out.display()))?;
    remove(&old)
}

/// Whether `dir` is missing, empty or holds a manifest.
fn is_whole_or_empty(dir: &Path) -> Result<bool> {
    match fs::read_dir(dir) {
        Ok(mut entries) => Ok(entries.next().is_none() || dir.join(MANIFEST).is_file()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(true),
        Err(err) => Err(err).wrap_err_with(|| format!("listing {}", dir.display())),
    }
}

fn remove(dir: &Path) -> Result<()> {
    match fs::remove_dir_all(dir) {
        Err(err) if err.kind() != ErrorKind::NotFound => {
            Err(err).wrap_err_with(|| format!("removing {}", dir.display()))
        }
        _ => Ok(()),
    }
}
