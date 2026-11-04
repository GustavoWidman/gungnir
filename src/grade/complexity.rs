//! Static complexity of a workspace's sources, measured by lizard. Lizard parses code but never
//! runs it, so it runs on the host from PATH.
use super::tree;
use crate::language::Language;
use eyre::{Result, WrapErr, bail, ensure, eyre};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path, time::Duration};
use tokio::process::Command;

const LIZARD_TIMEOUT: Duration = Duration::from_secs(300);

/// Every source file lizard measured.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Complexity {
    /// By path relative to the workspace.
    pub files: BTreeMap<String, FileComplexity>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileComplexity {
    /// Lines of code, blank lines and comments excluded.
    pub nloc: u32,
    pub functions: Vec<Function>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Function {
    pub name: String,
    /// The line the function starts on.
    pub line: u32,
    /// Cyclomatic complexity.
    pub ccn: u32,
    pub nloc: u32,
    pub tokens: u32,
    pub params: u32,
}

/// Measures the files under `root` that match the language's source globs and not its ignore
/// globs. `None` when the language names no lizard language.
pub(super) async fn measure(language: &Language, root: &Path) -> Result<Option<Complexity>> {
    let Some(lizard) = language.lizard() else {
        return Ok(None);
    };
    let files = sources(root, language)?;
    if files.is_empty() {
        return Ok(Some(Complexity::default()));
    }
    let (csv, report) = tokio::try_join!(
        run(root, lizard, &["--csv"], &files),
        run(root, lizard, &["--ignore_warnings=-1"], &files),
    )?;
    parse(&csv, &report, files).map(Some)
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
        .args(mode)
        .args(["--no-gitignore", "--languages", language, "--"])
        .args(files)
        .current_dir(root)
        .kill_on_drop(true);
    let output = tokio::time::timeout(LIZARD_TIMEOUT, command.output())
        .await
        .map_err(|_| eyre!("lizard ran longer than {LIZARD_TIMEOUT:?}"))?
        .wrap_err("running lizard, which must be on PATH")?;
    ensure!(
        output.status.success(),
        "lizard {} failed ({}): {}",
        mode.join(" "),
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Combines the functions of `lizard --csv` with the file NLOC of lizard's default `report`,
/// both over exactly `files`.
fn parse(csv: &str, report: &str, files: Vec<String>) -> Result<Complexity> {
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
    Ok(Complexity { files: measured })
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

    /// `lizard --csv` and the default report over three Python files, captured from lizard 1.24.
    const CSV: &str = r#"4,2,14,2,4,"f@1-4@src/sub dir/x.py","src/sub dir/x.py","f","f( a , b )",1,4
2,1,7,1,2,"m@7-8@src/sub dir/x.py","src/sub dir/x.py","m","m( self )",7,8
"#;
    const REPORT: &str = "================================================
  NLOC    CCN   token  PARAM  length  location
------------------------------------------------
       4      2     14      2       4 f@1-4@src/sub dir/x.py
       2      1      7      1       2 m@7-8@src/sub dir/x.py
3 file analyzed.
==============================================================
NLOC    Avg.NLOC  AvgCCN  Avg.token  function_cnt    file
--------------------------------------------------------------
      0       0.0     0.0        0.0         0     src/empty.py
      7       3.0     1.5       10.5         2     src/sub dir/x.py
      2       0.0     0.0        0.0         0     -dash.py

=========================================================
No thresholds exceeded (cyclomatic_complexity > 15 or length > 1000 or nloc > 1000000)
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
        let complexity = parse(CSV, REPORT, files()).unwrap();
        let file = |nloc, functions| FileComplexity { nloc, functions };
        assert_eq!(
            complexity.files,
            BTreeMap::from([
                ("-dash.py".into(), file(2, vec![])),
                ("src/empty.py".into(), file(0, vec![])),
                (
                    "src/sub dir/x.py".into(),
                    file(
                        7,
                        vec![
                            function("f", 1, [2, 4, 14, 2]),
                            function("m", 7, [1, 2, 7, 1])
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
        let complexity = parse(line, &report, vec!["src/main.zig".into()]).unwrap();
        assert_eq!(
            complexity.files["src/main.zig"].functions,
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
}
