"""Cyclomatic complexity of each passing program, code outside functions included.

The grade records sum lizard's complexity over functions, and lizard leaves out code at a file's
top level, where Python and JavaScript programs often keep their main loop. This module runs
lizard again over the source files a grade measured, with its `outside` extension, which counts
the top-level code as one more function.
"""

import csv
import io
import json
import subprocess
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

LIZARD = {"python": "python", "javascript": "javascript", "typescript": "typescript", "go": "go",
          "rust": "rust", "zig": "zig", "c": "c", "cpp": "cpp"}


def total(trial_dir, language):
    """The program's total cyclomatic complexity, or None when its grade measured no file."""
    grade = json.loads((trial_dir / "grade" / "record.json").read_text())
    files = sorted(((grade["judgement"].get("complexity") or {}).get("files") or {}))
    if not files:
        return None
    done = subprocess.run(
        ["lizard", "-Eoutside", "--csv", "--no-gitignore", "--languages", LIZARD[language], "--", *files],
        cwd=trial_dir / "run" / "final", capture_output=True, text=True, timeout=300, check=True)
    return sum(int(row[1]) for row in csv.reader(io.StringIO(done.stdout)) if row)


def totals(runs, trials):
    """Total complexity, with top-level code, of every passing trial: one row per trial."""
    passing = trials[trials.verdict == "pass"]
    keys = list(passing[["experiment", "system", "scenario", "language", "rep"]].itertuples(index=False))

    def one(k):
        trial_dir = Path(runs) / k.experiment / "trials" / k.system / k.scenario / k.language / f"r{k.rep}"
        return {"system": k.system, "scenario": k.scenario, "language": k.language, "rep": k.rep,
                "ccn_all": total(trial_dir, k.language)}

    with ThreadPoolExecutor(max_workers=8) as pool:
        return list(pool.map(one, keys))
