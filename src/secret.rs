//! Credentials that never print, and their removal from everything gungnir keeps.

use std::{
    fmt, fs,
    ops::Range,
    path::{Path, PathBuf},
};

use eyre::{Result, WrapErr};

const MARKER: &[u8] = b"[REDACTED]";

/// A credential. `Debug` shows no part of the value and there is no `Display`.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// The value of one variable in a sandbox environment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvValue {
    Plain(String),
    Secret(Secret<String>),
}

/// Replaces every occurrence of a set of secrets with `[REDACTED]`.
#[derive(Debug)]
pub struct Redactor {
    secrets: Vec<Secret<Vec<u8>>>,
}

impl Redactor {
    pub fn new<'a>(secrets: impl IntoIterator<Item = &'a Secret<String>>) -> Self {
        let secrets = secrets
            .into_iter()
            .filter(|secret| !secret.expose().is_empty())
            .map(|secret| Secret::new(secret.expose().as_bytes().to_vec()))
            .collect();
        Self { secrets }
    }

    pub fn redact(&self, bytes: &[u8]) -> Vec<u8> {
        let mut redacted = Vec::with_capacity(bytes.len());
        let mut copied = 0;
        for range in self.matches(bytes) {
            redacted.extend_from_slice(&bytes[copied..range.start]);
            redacted.extend_from_slice(MARKER);
            copied = range.end;
        }
        redacted.extend_from_slice(&bytes[copied..]);
        redacted
    }

    /// Rewrites `path` without its secrets. The rewrite is a rename, so a crash leaves either
    /// the old file or the redacted one.
    pub fn redact_file(&self, path: &Path) -> Result<()> {
        let bytes = fs::read(path).wrap_err_with(|| format!("reading {}", path.display()))?;
        if self.matches(&bytes).is_empty() {
            return Ok(());
        }
        let staging = staging_path(path);
        fs::write(&staging, self.redact(&bytes))
            .wrap_err_with(|| format!("writing {}", staging.display()))?;
        fs::rename(&staging, path).wrap_err_with(|| format!("replacing {}", path.display()))
    }

    /// Byte ranges covered by any secret, merged where secrets overlap or touch, so that no
    /// fragment of one secret survives next to the marker of another.
    fn matches(&self, bytes: &[u8]) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = self
            .secrets
            .iter()
            .flat_map(|secret| occurrences(bytes, secret.expose()))
            .collect();
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
        for range in ranges {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        merged
    }
}

fn occurrences<'a>(
    haystack: &'a [u8],
    needle: &'a [u8],
) -> impl Iterator<Item = Range<usize>> + 'a {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter(move |(_, window)| *window == needle)
        .map(move |(start, _)| start..start + needle.len())
}

fn staging_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".redacting");
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor(secrets: &[&str]) -> Redactor {
        let secrets: Vec<_> = secrets.iter().map(|s| Secret::new(s.to_string())).collect();
        Redactor::new(&secrets)
    }

    #[test]
    fn debug_output_never_shows_a_secret() {
        let env = [
            ("TOKEN", EnvValue::Secret(Secret::new("sk-live-123".into()))),
            ("MODE", EnvValue::Plain("fast".into())),
        ];
        let printed = format!("{env:?} {:?}", Secret::new("sk-live-123"));
        assert!(!printed.contains("sk-live-123"), "{printed}");
        assert!(printed.contains("fast"), "{printed}");
    }

    #[test]
    fn every_occurrence_of_every_secret_is_replaced() {
        let redacted = redactor(&["alpha", "beta"]).redact(b"alpha and beta, then alpha again");
        assert_eq!(
            redacted,
            b"[REDACTED] and [REDACTED], then [REDACTED] again"
        );
    }

    #[test]
    fn overlapping_secrets_leave_no_fragment() {
        let redacted = redactor(&["xyz123", "123abc"]).redact(b"<xyz123abc>");
        assert_eq!(redacted, b"<[REDACTED]>");
    }

    #[test]
    fn empty_secrets_and_clean_input_change_nothing() {
        let input = b"nothing to hide";
        assert_eq!(redactor(&[""]).redact(input), input);
        assert_eq!(redactor(&["absent"]).redact(input), input);
    }

    #[test]
    fn file_is_redacted_in_place() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("transcript.jsonl");
        fs::write(&path, "{\"env\":\"TOKEN=sk-live-123\"}\n")?;

        redactor(&["sk-live-123"]).redact_file(&path)?;

        assert_eq!(
            fs::read_to_string(&path)?,
            "{\"env\":\"TOKEN=[REDACTED]\"}\n"
        );
        assert_eq!(fs::read_dir(dir.path())?.count(), 1);
        Ok(())
    }
}
