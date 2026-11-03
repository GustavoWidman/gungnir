use super::parent;
use eyre::{Result, WrapErr, bail};
use std::{
    fs::{self, File, OpenOptions, TryLockError},
    io::Write,
    path::Path,
};

/// An exclusive `flock` on a file, released when dropped or when the process dies. While
/// held, the file contains the holder's pid.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

impl Lock {
    /// Takes the lock at `path` without waiting, creating the file and its parents. Fails at
    /// once, naming the holder's pid, when another holder has it.
    pub fn acquire(path: &Path) -> Result<Self> {
        let dir = parent(path);
        fs::create_dir_all(dir).wrap_err_with(|| format!("creating {}", dir.display()))?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)
            .wrap_err_with(|| format!("opening {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                let holder = fs::read_to_string(path).unwrap_or_default();
                match holder.trim() {
                    "" => bail!("{} is held by another process", path.display()),
                    pid => bail!("{} is held by pid {pid}", path.display()),
                }
            }
            Err(TryLockError::Error(err)) => {
                return Err(err).wrap_err_with(|| format!("locking {}", path.display()));
            }
        }
        file.set_len(0)
            .and_then(|()| writeln!(file, "{}", std::process::id()))
            .wrap_err_with(|| format!("writing {}", path.display()))?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn a_held_lock_fails_fast_naming_the_holder() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".locks/run.lock");

        let held = Lock::acquire(&path).unwrap();
        let err = Lock::acquire(&path).unwrap_err();
        let message = err.to_string();
        assert!(message.contains("run.lock"), "{message}");
        assert!(
            message.contains(&format!("pid {}", std::process::id())),
            "{message}"
        );

        drop(held);
        Lock::acquire(&path).unwrap();
    }

    #[test]
    fn locks_on_different_files_are_independent() {
        let dir = TempDir::new().unwrap();
        let _run = Lock::acquire(&dir.path().join("run.lock")).unwrap();
        Lock::acquire(&dir.path().join("grade.lock")).unwrap();
    }
}
