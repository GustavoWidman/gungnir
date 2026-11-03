use eyre::{Report, Result, WrapErr, bail, ensure};
use globset::GlobSet;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    fs::{self, File},
    io::{ErrorKind, Read},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    str::FromStr,
};

/// A SHA-256 content digest, displayed and serialized as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

/// What a tree digest records for one path.
enum Entry {
    File(Digest),
    Symlink(Digest),
}

impl Digest {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Digest of the files under `root`, by relative path and content.
    ///
    /// A path matching `ignore` (relative to `root`) is skipped, and a skipped directory
    /// takes its contents with it. Symlinks are hashed as their target string and never
    /// followed. Directories themselves, modes and timestamps are not part of the digest.
    pub fn of_tree(root: &Path, ignore: &GlobSet) -> Result<Self> {
        let mut entries = BTreeMap::new();
        collect(root, Path::new(""), ignore, &mut entries)?;
        Ok(Self::of_entries(&entries))
    }

    /// Digest of a tree holding exactly `files`, equal to [`Digest::of_tree`] over the same
    /// files written to disk.
    pub fn of_files<'a>(files: impl IntoIterator<Item = (&'a str, &'a [u8])>) -> Self {
        let entries = files
            .into_iter()
            .map(|(path, bytes)| (PathBuf::from(path), Entry::File(Self::of_bytes(bytes))))
            .collect();
        Self::of_entries(&entries)
    }

    fn of_entries(entries: &BTreeMap<PathBuf, Entry>) -> Self {
        let mut hasher = Sha256::new();
        for (path, entry) in entries {
            let (kind, digest) = match entry {
                Entry::File(digest) => (b'f', digest),
                Entry::Symlink(digest) => (b'l', digest),
            };
            hasher.update([kind]);
            hasher.update(path.as_os_str().as_bytes());
            hasher.update([0]);
            hasher.update(digest.0);
        }
        Self(hasher.finalize().into())
    }

    fn of_file(path: &Path) -> Result<Self> {
        let mut file = File::open(path).wrap_err_with(|| format!("reading {}", path.display()))?;
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            match file.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => hasher.update(&buffer[..read]),
                Err(err) if err.kind() == ErrorKind::Interrupted => {}
                Err(err) => {
                    return Err(err).wrap_err_with(|| format!("reading {}", path.display()));
                }
            }
        }
        Ok(Self(hasher.finalize().into()))
    }
}

fn collect(
    root: &Path,
    dir: &Path,
    ignore: &GlobSet,
    entries: &mut BTreeMap<PathBuf, Entry>,
) -> Result<()> {
    let abs = root.join(dir);
    let listing = fs::read_dir(&abs).wrap_err_with(|| format!("listing {}", abs.display()))?;
    for item in listing {
        let item = item.wrap_err_with(|| format!("listing {}", abs.display()))?;
        let rel = dir.join(item.file_name());
        if ignore.is_match(&rel) {
            continue;
        }
        let path = item.path();
        let kind = item
            .file_type()
            .wrap_err_with(|| format!("inspecting {}", path.display()))?;
        if kind.is_dir() {
            collect(root, &rel, ignore, entries)?;
        } else if kind.is_file() {
            entries.insert(rel, Entry::File(Digest::of_file(&path)?));
        } else if kind.is_symlink() {
            let target =
                fs::read_link(&path).wrap_err_with(|| format!("reading {}", path.display()))?;
            let digest = Digest::of_bytes(target.as_os_str().as_bytes());
            entries.insert(rel, Entry::Symlink(digest));
        } else {
            bail!("{} is not a file, directory or symlink", path.display());
        }
    }
    Ok(())
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Digest({self})")
    }
}

impl FromStr for Digest {
    type Err = Report;

    fn from_str(hex: &str) -> Result<Self> {
        let valid = hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        ensure!(valid, "{hex:?} is not a lowercase hex SHA-256 digest");
        let mut bytes = [0; 32];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            *byte = u8::from_str_radix(std::str::from_utf8(pair)?, 16)?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Digest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Digest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::write;
    use globset::{Glob, GlobSetBuilder};
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    fn ignoring(patterns: &[&str]) -> GlobSet {
        let mut builder = GlobSetBuilder::new();
        for pattern in patterns {
            builder.add(Glob::new(pattern).unwrap());
        }
        builder.build().unwrap()
    }

    fn tree(files: &[(&str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        for (path, contents) in files {
            write(dir.path(), path, contents);
        }
        dir
    }

    fn digest(dir: &TempDir) -> Digest {
        Digest::of_tree(dir.path(), &GlobSet::empty()).unwrap()
    }

    #[test]
    fn known_vector() {
        assert_eq!(
            Digest::of_bytes(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn hex_round_trips_through_serde() {
        let digest = Digest::of_bytes(b"gungnir");
        let json = serde_json::to_string(&digest).unwrap();
        assert_eq!(json, format!("\"{digest}\""));
        assert_eq!(serde_json::from_str::<Digest>(&json).unwrap(), digest);
        assert!(serde_json::from_str::<Digest>("\"abc\"").is_err());
    }

    #[test]
    fn tree_digest_depends_on_names_and_contents_only() {
        let base = digest(&tree(&[("a.txt", "1"), ("src/b.txt", "2")]));
        assert_eq!(base, digest(&tree(&[("src/b.txt", "2"), ("a.txt", "1")])));
        assert_ne!(base, digest(&tree(&[("a.txt", "1"), ("src/b.txt", "3")])));
        assert_ne!(base, digest(&tree(&[("a.txt", "1"), ("src/c.txt", "2")])));
        assert_ne!(base, digest(&tree(&[("a.txt", "1"), ("b.txt", "2")])));

        let with_empty_dir = tree(&[("a.txt", "1"), ("src/b.txt", "2")]);
        fs::create_dir(with_empty_dir.path().join("empty")).unwrap();
        assert_eq!(base, digest(&with_empty_dir));
    }

    #[test]
    fn ignored_paths_do_not_change_the_digest() {
        let clean = tree(&[("main.py", "print()")]);
        let built = tree(&[
            ("main.py", "print()"),
            (".venv/lib/x.py", "x"),
            ("pkg/__pycache__/m.pyc", "c"),
        ]);
        let ignore = ignoring(&[".venv/**", "**/__pycache__/**"]);
        assert_eq!(
            Digest::of_tree(clean.path(), &ignore).unwrap(),
            Digest::of_tree(built.path(), &ignore).unwrap()
        );
        assert_ne!(digest(&clean), digest(&built));
    }

    #[test]
    fn symlinks_are_hashed_as_their_target_and_never_followed() {
        let outside = tree(&[("secret", "hidden case")]);
        let link = |target: &Path| {
            let dir = tree(&[("a.txt", "1")]);
            symlink(target, dir.path().join("link")).unwrap();
            digest(&dir)
        };
        let to_outside = link(&outside.path().join("secret"));
        let to_a = link(Path::new("a.txt"));
        assert_ne!(to_outside, to_a);

        fs::write(outside.path().join("secret"), "edited").unwrap();
        assert_eq!(to_outside, link(&outside.path().join("secret")));

        let dangling = link(Path::new("missing"));
        assert_ne!(dangling, to_a);
    }

    #[test]
    fn in_memory_files_digest_like_the_same_tree_on_disk() {
        let files = [("spec.md", "task"), ("examples/a.in", "1 2")];
        let on_disk = digest(&tree(&files));
        let in_memory = Digest::of_files(files.map(|(path, text)| (path, text.as_bytes())));
        assert_eq!(on_disk, in_memory);
    }
}
