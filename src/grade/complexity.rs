//! Static complexity of a workspace's sources, measured by lizard. Lizard parses code but never
//! runs it, so it runs on the host from PATH, and a complexity names the lizard version that
//! counted it.
use super::tree;
use crate::language::Language;
use eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::process::Command;

const LIZARD_TIMEOUT: Duration = Duration::from_secs(300);

/// Raised whenever measuring would count the same sources differently, which makes every stored
/// complexity stale. Version 1 counted the code inside functions only. Version 2 also counts
/// each file's code outside them.
const VERSION: u32 = 2;

/// Lizard's `outside` extension reports each file's code outside any function as one more
/// function, `*global*`. That also catches code its Rust reader loses after a byte literal,
/// which it otherwise drops with every later function.
const OUTSIDE: &str = "-Eoutside";

/// Every source file lizard measured, and how it was counted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complexity {
    /// The [`VERSION`] of the counting. A record made before complexity had versions holds
    /// none, and was counted by version 1.
    #[serde(default = "unversioned")]
    version: u32,
    /// What `lizard --version` printed, absent from a record made before complexity had
    /// versions.
    #[serde(default)]
    lizard: Option<String>,
    /// By path relative to the workspace.
    pub files: BTreeMap<String, FileComplexity>,
}

impl Complexity {
    /// The version of the counting that measured it.
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The version of the lizard that measured it, when the record names one.
    pub fn lizard(&self) -> Option<&str> {
        self.lizard.as_deref()
    }
}

fn unversioned() -> u32 {
    1
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileComplexity {
    /// Lines of code, blank lines and comments excluded.
    pub nloc: u32,
    /// Every function lizard found, then the file's code outside them as one more, named
    /// `*global*`, so the functions' cyclomatic complexities sum to the file's.
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    /// The line the function starts on, 0 for `*global*`.
    pub line: u32,
    /// Cyclomatic complexity.
    pub ccn: u32,
    pub nloc: u32,
    pub tokens: u32,
    pub params: u32,
}

/// How measuring counts complexity now: this version of the counting, with the lizard on PATH.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counting {
    /// What `lizard --version` printed, asked only when some language names a lizard language.
    lizard: Option<String>,
}

impl Counting {
    /// Asks the lizard on PATH for its version when one of `languages` names a lizard language,
    /// and asks nothing otherwise.
    pub async fn probe<'a>(languages: impl IntoIterator<Item = &'a Language>) -> Result<Self> {
        let lizard = if languages
            .into_iter()
            .any(|language| language.lizard().is_some())
        {
            Some(version().await?)
        } else {
            None
        };
        Ok(Self { lizard })
    }

    /// Whether `complexity`, measured from a workspace in `language`, is what measuring that
    /// workspace would give now: none for a language lizard cannot measure, and otherwise a
    /// complexity counted by this version with this lizard.
    pub fn is_current(&self, language: &Language, complexity: Option<&Complexity>) -> bool {
        match (language.lizard(), complexity) {
            (None, None) => true,
            (Some(_), Some(complexity)) => {
                complexity.version == VERSION
                    && complexity.lizard.is_some()
                    && complexity.lizard == self.lizard
            }
            (None, Some(_)) | (Some(_), None) => false,
        }
    }

    /// The counting with a lizard that reports `version`, without asking one.
    #[cfg(test)]
    pub(crate) fn with_lizard(version: &str) -> Self {
        Self {
            lizard: Some(version.to_owned()),
        }
    }
}

/// Measures the files under `root` that match the language's source globs and not its ignore
/// globs. `None` when the language names no lizard language.
pub(super) async fn measure(language: &Language, root: &Path) -> Result<Option<Complexity>> {
    let Some(lizard) = language.lizard() else {
        return Ok(None);
    };
    let files = sources(root, language)?;
    if files.is_empty() {
        let lizard = Some(version().await?);
        let files = BTreeMap::new();
        return Ok(Some(Complexity {
            version: VERSION,
            lizard,
            files,
        }));
    }
    let (version, csv, report) = tokio::try_join!(
        version(),
        run(root, lizard, &["--csv"], &files),
        run(root, lizard, &["--ignore_warnings=-1"], &files),
    )?;
    let files = parse(&csv, &report, files)?;
    Ok(Some(Complexity {
        version: VERSION,
        lizard: Some(version),
        files,
    }))
}

/// Source paths as lizard prints them. Lizard quotes file names in its CSV without escaping, so
/// a name with a quote or a line break cannot be read back.
fn sources(root: &Path, language: &Language) -> Result<Vec<String>> {
    tree::files(root, language.ignore())?
        .into_iter()
        .filter(|(rel, kind)| kind.is_file() && language.sources().is_match(rel))
        .map(|(rel, _)| {
            rel.to_str()
                .filter(|name| !name.contains(['"', '\n', '\r']))
                .map(str::to_owned)
                .ok_or_else(|| eyre!("lizard cannot report on the source file {rel:?}"))
        })
        .collect()
}

async fn run(root: &Path, language: &str, mode: &[&str], files: &[String]) -> Result<String> {
    let mut command = Command::new("lizard");
    command
        .arg(OUTSIDE)
        .args(mode)
        .args(["--no-gitignore", "--languages", language, "--"])
        .args(files)
        .current_dir(root);
    output(command, &mode.join(" ")).await
}

/// What the lizard on PATH says its version is.
async fn version() -> Result<String> {
    let mut command = Command::new("lizard");
    command.arg("--version");
    parse_version(&output(command, "--version").await?)
}

/// The one line `lizard --version` prints.
fn parse_version(stdout: &str) -> Result<String> {
    match stdout.trim().lines().collect::<Vec<_>>()[..] {
        [line] if !line.trim().is_empty() => Ok(line.trim().to_owned()),
        _ => bail!("lizard --version printed {stdout:?}, not one line"),
    }
}

/// The stdout of `command`, a lizard invocation that runs `what`, once it exits 0.
async fn output(mut command: Command, what: &str) -> Result<String> {
    command.kill_on_drop(true);
    let output = tokio::time::timeout(LIZARD_TIMEOUT, command.output())
        .await
        .map_err(|_| eyre!("lizard ran longer than {LIZARD_TIMEOUT:?}"))?
        .wrap_err("running lizard, which must be on PATH")?;
    ensure!(
        output.status.success(),
        "lizard {what} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Combines the functions of `lizard --csv` with the file NLOC of lizard's default `report`,
/// both over exactly `files`.
fn parse(csv: &str, report: &str, files: Vec<String>) -> Result<BTreeMap<String, FileComplexity>> {
    let nloc = file_nloc(report).wrap_err("reading lizard's file table")?;
    let mut measured = BTreeMap::new();
    for file in files {
        let nloc = *nloc
            .get(file.as_str())
            .ok_or_else(|| eyre!("lizard reported no NLOC for {file}"))?;
        let functions = Vec::new();
        measured.insert(file, FileComplexity { nloc, functions });
    }
    for (number, line) in csv.lines().enumerate() {
        let raw = RawFunction::parse(line)
            .wrap_err_with(|| format!("lizard --csv line {}: {line:?}", number + 1))?;
        let file = measured
            .get_mut(raw.file)
            .ok_or_else(|| eyre!("lizard measured {}, which is not a source", raw.file))?;
        file.functions.push(raw.into());
    }
    Ok(measured)
}

/// One line of `lizard --csv`: NLOC, CCN, token, PARAM, length, location, file, function,
/// long_name, start, end.
struct RawFunction<'a> {
    nloc: u32,
    ccn: u32,
    tokens: u32,
    params: u32,
    file: &'a str,
    name: &'a str,
    start: u32,
}

impl<'a> RawFunction<'a> {
    fn parse(line: &'a str) -> Result<Self> {
        let fields = csv_fields(line)?;
        let [nloc, ccn, tokens, params, _, _, file, name, _, start, _] = fields[..] else {
            bail!("{} fields where lizard prints 11", fields.len());
        };
        let count = |field: &str| {
            field
                .parse::<u32>()
                .wrap_err_with(|| format!("{field:?} is not a count"))
        };
        Ok(Self {
            nloc: count(nloc)?,
            ccn: count(ccn)?,
            tokens: count(tokens)?,
            params: count(params)?,
            file,
            name,
            start: count(start)?,
        })
    }
}

impl From<RawFunction<'_>> for Function {
    fn from(raw: RawFunction<'_>) -> Self {
        Self {
            name: raw.name.to_owned(),
            line: raw.start,
            ccn: raw.ccn,
            nloc: raw.nloc,
            tokens: raw.tokens,
            params: raw.params,
        }
    }
}

/// Splits one CSV line. Lizard replaces any `"` in a function name with `'`, so a quoted field
/// ends at the next quote.
fn csv_fields(line: &str) -> Result<Vec<&str>> {
    let mut fields = Vec::new();
    let mut rest = line;
    loop {
        let (field, after) = match rest.strip_prefix('"') {
            Some(quoted) => quoted
                .split_once('"')
                .ok_or_else(|| eyre!("a quote is never closed"))?,
            None => rest.split_at(rest.find(',').unwrap_or(rest.len())),
        };
        fields.push(field);
        match after.strip_prefix(',') {
            Some(next) => rest = next,
            None if after.is_empty() => return Ok(fields),
            None => bail!("{after:?} follows a quoted field"),
        }
    }
}

/// Reads the file table of lizard's default report, whose rows are
/// `{nloc:7}{avg_nloc:10}{avg_ccn:8}{avg_token:10}{function_cnt:10}     {file}`.
fn file_nloc(report: &str) -> Result<BTreeMap<&str, u32>> {
    let mut lines = report
        .lines()
        .skip_while(|line| !(line.starts_with("NLOC") && line.ends_with(" file")));
    ensure!(lines.next().is_some(), "no file table");
    ensure!(
        lines.next().is_some_and(|rule| rule.starts_with("---")),
        "no rule under the file table's header"
    );
    lines
        .take_while(|line| !line.is_empty())
        .map(|row| {
            let mut rest = row;
            let mut columns = [""; 5];
            for column in &mut columns {
                let trimmed = rest.trim_start_matches(' ');
                (*column, rest) = trimmed.split_at(trimmed.find(' ').unwrap_or(trimmed.len()));
            }
            let file = rest
                .strip_prefix("     ")
                .ok_or_else(|| eyre!("row {row:?} names no file"))?;
            let nloc = columns[0]
                .parse()
                .wrap_err_with(|| format!("row {row:?}"))?;
            Ok((file, nloc))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{self, write};
    use tempfile::TempDir;

    /// `lizard -Eoutside --csv` and the default report over three Python files, captured from
    /// lizard 1.24.1. Every file has a `*global*`, the empty one included.
    const CSV: &str = r#"3,1,7,0,3,"*global*@0-2@-dash.py","-dash.py","*global*","*global*",0,2
1,1,1,0,1,"*global*@0-0@src/empty.py","src/empty.py","*global*","*global*",0,0
4,2,14,2,4,"f@1-4@src/sub dir/x.py","src/sub dir/x.py","f","f( a , b )",1,4
2,1,7,1,2,"m@8-9@src/sub dir/x.py","src/sub dir/x.py","m","m( self )",8,9
7,3,34,0,15,"*global*@0-14@src/sub dir/x.py","src/sub dir/x.py","*global*","*global*",0,14
"#;
    const REPORT: &str = "================================================
  NLOC    CCN   token  PARAM  length  location
------------------------------------------------
       3      1      7      0       3 *global*@0-2@-dash.py
       1      1      1      0       1 *global*@0-0@src/empty.py
       4      2     14      2       4 f@1-4@src/sub dir/x.py
       2      1      7      1       2 m@8-9@src/sub dir/x.py
       7      3     34      0      15 *global*@0-14@src/sub dir/x.py
3 file analyzed.
==============================================================
NLOC    Avg.NLOC  AvgCCN  Avg.token  function_cnt    file
--------------------------------------------------------------
      2       3.0     1.0        7.0         1     -dash.py
      0       1.0     1.0        1.0         1     src/empty.py
     10       4.3     2.0       18.3         3     src/sub dir/x.py

===============================================================================================================
No thresholds exceeded (cyclomatic_complexity > 15 or length > 1000 or nloc > 1000000 or parameter_count > 100)
";

    fn files() -> Vec<String> {
        ["-dash.py", "src/empty.py", "src/sub dir/x.py"]
            .map(str::to_owned)
            .to_vec()
    }

    fn function(name: &str, line: u32, [ccn, nloc, tokens, params]: [u32; 4]) -> Function {
        Function {
            name: name.into(),
            line,
            ccn,
            nloc,
            tokens,
            params,
        }
    }

    #[test]
    fn functions_and_file_nloc_combine_per_source() {
        let files = parse(CSV, REPORT, files()).unwrap();
        let file = |nloc, functions| FileComplexity { nloc, functions };
        assert_eq!(
            files,
            BTreeMap::from([
                (
                    "-dash.py".into(),
                    file(2, vec![function("*global*", 0, [1, 3, 7, 0])])
                ),
                (
                    "src/empty.py".into(),
                    file(0, vec![function("*global*", 0, [1, 1, 1, 0])])
                ),
                (
                    "src/sub dir/x.py".into(),
                    file(
                        10,
                        vec![
                            function("f", 1, [2, 4, 14, 2]),
                            function("m", 8, [1, 2, 7, 1]),
                            function("*global*", 0, [3, 7, 34, 0]),
                        ]
                    )
                ),
            ])
        );
    }

    #[test]
    fn long_names_may_hold_commas() {
        let line = r#"4,2,58,3,4,"less@49-52@src/main.zig","src/main.zig","less","less _ : void , a : Entry , b : Entry",49,52"#;
        let report = REPORT.replace("src/empty.py", "src/main.zig");
        let files = parse(line, &report, vec!["src/main.zig".into()]).unwrap();
        assert_eq!(
            files["src/main.zig"].functions,
            [function("less", 49, [2, 4, 58, 3])]
        );
    }

    #[test]
    fn malformed_output_is_an_error_naming_the_line() {
        let short = "4,2,14\n";
        let message = format!("{:#}", parse(short, REPORT, files()).unwrap_err());
        assert!(
            message.contains("line 1") && message.contains("3 fields"),
            "{message}"
        );

        let unknown = CSV.replace("src/sub dir/x.py", "elsewhere.py");
        let message = format!("{:#}", parse(&unknown, REPORT, files()).unwrap_err());
        assert!(message.contains("elsewhere.py"), "{message}");

        let message = format!(
            "{:#}",
            parse(CSV, "1 file analyzed.\n", files()).unwrap_err()
        );
        assert!(message.contains("no file table"), "{message}");

        let missing = vec!["src/other.py".to_owned()];
        let message = format!("{:#}", parse("", REPORT, missing).unwrap_err());
        assert!(message.contains("src/other.py"), "{message}");
    }

    #[test]
    fn the_version_is_the_one_line_lizard_prints() {
        assert_eq!(parse_version("1.24.1\n").unwrap(), "1.24.1");
        for printed in ["", "\n", "1.24.1\nwarning\n"] {
            let message = parse_version(printed).unwrap_err().to_string();
            assert!(message.contains("not one line"), "{message}");
        }
    }

    /// A `python` language that lizard measures and a `rust` one it does not.
    fn languages(root: &Path) -> (Language, Language) {
        testing::language(root, "python");
        testing::language(root, "rust");
        write(
            root,
            "languages/rust/language.toml",
            testing::LANGUAGE_TOML.replace("lizard = \"python\"\n", ""),
        );
        let mut all = Language::load_all(&root.join("languages")).unwrap();
        let mut take = |id: &str| all.remove(&testing::id(id)).unwrap();
        (take("python"), take("rust"))
    }

    #[test]
    fn a_complexity_is_current_when_counted_by_this_version_with_the_lizard_on_path() {
        let root = TempDir::new().unwrap();
        let (measured, unmeasured) = languages(root.path());
        let counting = Counting::with_lizard("1.24.1");
        let stored = |json: &str| serde_json::from_str::<Complexity>(json).unwrap();

        let older = stored(r#"{"files": {"main.py": {"nloc": 1, "functions": []}}}"#);
        assert_eq!((older.version(), older.lizard()), (1, None));
        assert_eq!(older.files["main.py"].nloc, 1);
        assert!(!counting.is_current(&measured, Some(&older)));

        let counted = stored(r#"{"version": 2, "lizard": "1.24.1", "files": {}}"#);
        assert!(counting.is_current(&measured, Some(&counted)));
        assert!(!Counting::with_lizard("1.25.0").is_current(&measured, Some(&counted)));
        assert!(!counting.is_current(&measured, None));

        assert!(counting.is_current(&unmeasured, None));
        assert!(!counting.is_current(&unmeasured, Some(&counted)));
    }

    #[tokio::test]
    async fn measuring_counts_code_outside_functions() {
        if std::process::Command::new("lizard")
            .arg("--version")
            .output()
            .is_err()
        {
            eprintln!("skipped: lizard is not on PATH");
            return;
        }
        let root = TempDir::new().unwrap();
        let (python, rust) = languages(root.path());
        let work = root.path().join("work");
        let top = "def f(a):\n    return a\n\nfor line in open(0):\n    if line:\n        print(f(line))\n";
        write(&work, "main.py", top);

        let complexity = measure(&python, &work).await.unwrap().unwrap();
        let counting = Counting::probe([&python, &rust]).await.unwrap();
        assert!(counting.is_current(&python, Some(&complexity)));
        let ccn: Vec<(&str, u32)> = complexity.files["main.py"]
            .functions
            .iter()
            .map(|function| (function.name.as_str(), function.ccn))
            .collect();
        assert_eq!(ccn, [("f", 1), ("*global*", 3)]);

        let nothing = root.path().join("nothing");
        std::fs::create_dir(&nothing).unwrap();
        let empty = measure(&python, &nothing).await.unwrap().unwrap();
        assert!(empty.files.is_empty() && counting.is_current(&python, Some(&empty)));
        assert_eq!(measure(&rust, &work).await.unwrap(), None);
        assert_eq!(Counting::probe([&rust]).await.unwrap().lizard, None);
    }
}
