//! The agent's workspace: scaffolded from the language template and the brief's visible
//! examples, and kept for the run directory without build output or secrets.
use crate::{grade::tree, language::Language, scenario::Brief, store::Digest};
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        id::{LanguageId, ScenarioId},
        scenario::Scenario,
        testing::{self, id, write},
    };
    use std::path::PathBuf;
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
}
