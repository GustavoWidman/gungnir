//! The agent's workspace: scaffolded from the language template and the brief's visible
//! examples, and kept for the run directory without build output or secrets.
use crate::{grade::tree, language::Language, scenario::Brief, secret::Redactor, store::Digest};
use eyre::{Result, WrapErr};
use globset::GlobSet;
use std::{fs, path::Path};

/// Fills `dir` with the language's template and, for each visible example,
/// `examples/NAME.in` and `examples/NAME.out`, and nothing else. Returns the digest of
/// everything the agent starts with.
pub(crate) fn scaffold(language: &Language, brief: &Brief, dir: &Path) -> Result<Digest> {
    tree::copy(language.template(), dir, &GlobSet::empty())?;
    let examples = dir.join("examples");
    fs::create_dir_all(&examples).wrap_err_with(|| format!("creating {}", examples.display()))?;
    for (id, example) in brief.examples() {
        for (extension, bytes) in [("in", &example.input), ("out", &example.output)] {
            let path = examples.join(format!("{id}.{extension}"));
            fs::write(&path, bytes).wrap_err_with(|| format!("writing {}", path.display()))?;
        }
    }
    Digest::of_tree(dir, &GlobSet::empty())
}

/// Copies `workspace` to `to` without the language's build output and with every secret
/// redacted from its files. Symlinks are copied as links and never followed. Returns the
/// digest of the copy, which is what grading judges.
pub(super) fn keep(
    workspace: &Path,
    language: &Language,
    redactor: &Redactor,
    to: &Path,
) -> Result<Digest> {
    tree::copy(workspace, to, language.ignore())?;
    for (rel, kind) in tree::files(to, &GlobSet::empty())? {
        if kind.is_file() {
            redactor.redact_file(&to.join(rel))?;
        }
    }
    Digest::of_tree(to, language.ignore())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        id::{LanguageId, ScenarioId},
        scenario::Scenario,
        secret::Secret,
        testing::{self, id, write},
    };
    use std::{os::unix::fs::symlink, path::PathBuf};
    use tempfile::TempDir;

    fn files(root: &Path) -> Vec<PathBuf> {
        tree::files(root, &GlobSet::empty())
            .unwrap()
            .into_iter()
            .map(|(rel, _)| rel)
            .collect()
    }

    #[test]
    fn a_workspace_holds_the_template_and_the_visible_examples_only() {
        let repo = TempDir::new().unwrap();
        testing::repo(repo.path());
        write(
            repo.path(),
            "languages/python/template/pkg/util.py",
            "x = 1\n",
        );
        let languages = Language::load_all(&repo.path().join("languages")).unwrap();
        let scenarios = Scenario::load_all(&repo.path().join("scenarios"), &languages).unwrap();
        let language = &languages[&id::<LanguageId>("python")];
        let brief = scenarios[&id::<ScenarioId>("echo")].brief();

        let workspace = TempDir::new().unwrap();
        let digest = scaffold(language, brief, workspace.path()).unwrap();
        assert_eq!(
            files(workspace.path()),
            [
                "examples/basic.in",
                "examples/basic.out",
                "main.py",
                "pkg/util.py"
            ]
            .map(PathBuf::from)
        );
        assert_eq!(
            fs::read_to_string(workspace.path().join("examples/basic.out")).unwrap(),
            "a\n"
        );
        let again = TempDir::new().unwrap();
        assert_eq!(scaffold(language, brief, again.path()).unwrap(), digest);
    }

    #[test]
    fn a_kept_workspace_has_no_build_output_no_secrets_and_unfollowed_links() {
        let repo = TempDir::new().unwrap();
        testing::repo(repo.path());
        let languages = Language::load_all(&repo.path().join("languages")).unwrap();
        let language = &languages[&id::<LanguageId>("python")];
        let outside = TempDir::new().unwrap();
        write(outside.path(), "host.txt", "token sk-live-123");

        let workspace = TempDir::new().unwrap();
        write(workspace.path(), "main.py", "KEY = 'sk-live-123'\n");
        write(workspace.path(), "__pycache__/main.pyc", "sk-live-123");
        symlink(
            outside.path().join("host.txt"),
            workspace.path().join("link"),
        )
        .unwrap();
        let secrets = [Secret::new("sk-live-123".to_owned())];

        let kept = TempDir::new().unwrap();
        let to = kept.path().join("final");
        let digest = keep(workspace.path(), language, &Redactor::new(&secrets), &to).unwrap();
        assert_eq!(files(&to), ["link", "main.py"].map(PathBuf::from));
        assert_eq!(
            fs::read_to_string(to.join("main.py")).unwrap(),
            "KEY = '[REDACTED]'\n"
        );
        assert!(to.join("link").is_symlink());
        assert_eq!(
            fs::read_to_string(outside.path().join("host.txt")).unwrap(),
            "token sk-live-123"
        );
        assert_eq!(digest, Digest::of_tree(&to, language.ignore()).unwrap());
    }
}
