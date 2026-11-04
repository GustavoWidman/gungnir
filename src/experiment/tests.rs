use super::*;
use crate::testing::{self, write};
use tempfile::TempDir;

fn fixture() -> TempDir {
    let root = TempDir::new().unwrap();
    testing::repo(root.path());
    root
}

fn load(root: &Path) -> Result<Experiment> {
    Experiment::load(&root.join("experiments/trial.toml"))
}

fn edit(root: &Path, from: &str, to: &str) {
    assert!(
        testing::EXPERIMENT_TOML.contains(from),
        "fixture lacks {from:?}"
    );
    write(
        root,
        "experiments/trial.toml",
        testing::EXPERIMENT_TOML.replacen(from, to, 1),
    );
}

/// Loading the fixture experiment with `from` replaced by `to` fails with a message naming
/// the experiment file and every one of `needles`.
fn rejects(from: &str, to: &str, needles: &[&str]) {
    let root = fixture();
    edit(root.path(), from, to);
    let message = format!("{:#}", load(root.path()).unwrap_err());
    assert!(message.contains("trial.toml"), "{message}");
    for needle in needles {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }
}

fn system_ids(experiment: &Experiment) -> Vec<String> {
    experiment
        .systems()
        .iter()
        .map(|system| system.id().to_string())
        .collect()
}

#[test]
fn loads_the_matrix_against_the_enclosing_repository() {
    let root = fixture();
    let experiment = load(root.path()).unwrap();

    assert_eq!(experiment.name(), "trial-run");
    let root_path = fs::canonicalize(root.path()).unwrap();
    assert_eq!(
        experiment.results().path(),
        root_path.join("runs/trial-run")
    );
    assert_eq!(
        system_ids(&experiment),
        [
            "claude-code-2.1.316_anthropic_opus_low",
            "claude-code-2.1.316_anthropic_opus_high",
            "codex-0.166.0_openai_luna_none",
            "codex-0.166.0_openai_luna_low",
        ]
    );
    let languages: Vec<&str> = experiment
        .languages()
        .iter()
        .map(|l| l.id().as_str())
        .collect();
    assert_eq!(languages, ["python", "rust"]);
    assert_eq!(experiment.budget().wall(), Duration::from_secs(40 * 60));
    assert_eq!(experiment.budget().idle(), Duration::from_secs(10 * 60));
    let openai: BackendId = testing::id("openai");
    assert_eq!(experiment.concurrency()[&openai].get(), 1);
}

#[test]
fn trials_are_repetition_major_and_deterministic() {
    let root = fixture();
    let experiment = load(root.path()).unwrap();
    let trials = experiment.trials();
    assert_eq!(trials.len(), 4 * 2 * 2);
    assert_eq!(trials, load(root.path()).unwrap().trials());

    let reps: Vec<String> = trials.iter().map(|trial| trial.rep.to_string()).collect();
    assert_eq!(reps[..8], ["r1"; 8]);
    assert_eq!(reps[8..], ["r2"; 8]);

    let cells: Vec<(String, &str)> = trials[..4]
        .iter()
        .map(|trial| (trial.system.id().to_string(), trial.language.as_str()))
        .collect();
    let opus_low = "claude-code-2.1.316_anthropic_opus_low".to_owned();
    let opus_high = "claude-code-2.1.316_anthropic_opus_high".to_owned();
    assert_eq!(
        cells,
        [
            (opus_low.clone(), "python"),
            (opus_low, "rust"),
            (opus_high.clone(), "python"),
            (opus_high, "rust"),
        ]
    );
}

#[test]
fn a_model_served_by_several_backends_needs_one_chosen() {
    let glm = "[[systems]]\nmodel = \"glm\"\nefforts = [\"low\"]\n";
    rejects(
        "[[systems]]\nmodel = \"luna\"",
        &format!("{glm}\n[[systems]]\nmodel = \"luna\""),
        &["model = \"glm\"", "anthropic, zai", "backend"],
    );

    let root = fixture();
    let chosen = glm.replace("efforts", "backend = \"zai\"\nefforts");
    let mut toml = format!("{}\n{chosen}", testing::EXPERIMENT_TOML);
    toml = toml.replace("openai = 1", "openai = 1\nzai = 3");
    write(root.path(), "experiments/trial.toml", toml);
    let experiment = load(root.path()).unwrap();
    assert!(system_ids(&experiment).contains(&"claude-code-2.1.316_zai_glm_low".to_owned()));
}

#[test]
fn efforts_must_be_offered_by_the_model() {
    rejects(
        "efforts = [\"high\", \"low\"]",
        "efforts = [\"high\", \"medium\"]",
        &["model = \"opus\"", "medium", "low, high, max"],
    );
    rejects(
        "efforts = [\"high\", \"low\"]",
        "efforts = [\"hihg\"]",
        &["model = \"opus\"", "hihg"],
    );
    rejects(
        "efforts = \"all\"",
        "efforts = \"every\"",
        &["model = \"luna\"", "\"every\""],
    );
}

#[test]
fn efforts_must_be_carried_by_the_harness() {
    let root = fixture();
    let opus_offers_none = testing::CATALOG_TOML.replacen(
        "efforts = [\"low\", \"high\", \"max\"]",
        "efforts = [\"none\", \"low\", \"high\", \"max\"]",
        1,
    );
    write(root.path(), "gungnir.toml", opus_offers_none);
    edit(
        root.path(),
        "efforts = [\"high\", \"low\"]",
        "efforts = \"all\"",
    );
    let message = format!("{:#}", load(root.path()).unwrap_err());
    for needle in [
        "trial.toml",
        "model = \"opus\"",
        "claude-code",
        "effort none",
    ] {
        assert!(
            message.contains(needle),
            "{needle:?} missing from: {message}"
        );
    }
}

#[test]
fn every_harness_in_use_needs_a_pin() {
    rejects("codex = \"0.166.0\"\n", "", &["[pins]", "codex", "openai"]);
    rejects(
        "codex = \"0.166.0\"",
        "codex = \"0.166.0\"\ngemini = \"1.0.0\"",
        &["[pins]", "gemini"],
    );
}

#[test]
fn concurrency_covers_exactly_the_backends_in_use() {
    rejects("openai = 1\n", "", &["[concurrency]", "openai"]);
    rejects(
        "openai = 1",
        "openai = 1\nzai = 3",
        &["[concurrency]", "zai"],
    );
}

#[test]
fn every_scenario_needs_a_reference_for_every_language() {
    let root = fixture();
    testing::scenario(root.path(), "sum", &["python"]);
    edit(
        root.path(),
        "scenarios = [\"echo\"]",
        "scenarios = [\"echo\", \"sum\"]",
    );
    let message = format!("{:#}", load(root.path()).unwrap_err());
    assert!(message.contains("scenario sum"), "{message}");
    assert!(message.contains("reference/rust"), "{message}");
}

#[test]
fn scenarios_and_languages_must_exist_once() {
    rejects(
        "scenarios = [\"echo\"]",
        "scenarios = [\"ecco\"]",
        &["scenarios lists ecco", "scenarios/"],
    );
    rejects(
        "languages = [\"python\", \"rust\"]",
        "languages = [\"python\", \"python\"]",
        &["languages lists python twice"],
    );
}

#[test]
fn budgets_are_positive_durations() {
    rejects(
        "wall = \"40m\"",
        "wall = \"forty\"",
        &["budget.wall = \"forty\""],
    );
    rejects(
        "idle = \"10m\"",
        "idle = \"0s\"",
        &["budget.idle = \"0s\" is zero"],
    );
}

#[test]
fn an_experiment_outside_a_repository_is_an_error() {
    let dir = TempDir::new().unwrap();
    write(dir.path(), "lonely.toml", testing::EXPERIMENT_TOML);
    let err = Experiment::load(&dir.path().join("lonely.toml")).unwrap_err();
    assert!(err.to_string().contains("gungnir.toml"), "{err}");
}

#[test]
fn a_machine_pin_names_a_cpu_model_and_a_host_id() {
    let root = fixture();
    assert_eq!(load(root.path()).unwrap().machine(), None);
    let pin =
        "[machine]\ncpu = \"13th Gen Intel(R) Core(TM) i9-13900K\"\nhost = \"d35k70p\"\n\n[pins]";
    edit(root.path(), "[pins]", pin);
    let experiment = load(root.path()).unwrap();
    assert_eq!(
        experiment.machine(),
        Some(&Identity {
            cpu: Some("13th Gen Intel(R) Core(TM) i9-13900K".into()),
            host: Some("d35k70p".into()),
        })
    );

    rejects(
        "[pins]",
        "[machine]\ncpu = \"i9-13900K\"\nhost = \"\"\n\n[pins]",
        &["machine.host"],
    );
    rejects(
        "[pins]",
        "[machine]\ncpu = \"i9-13900K\"\n\n[pins]",
        &["host"],
    );
}
