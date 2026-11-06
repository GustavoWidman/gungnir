"""Why Python programs crash on hidden tests: replays one crashing case of each such trial in
a sandbox and keeps the exception it raised.

    python -m analysis.crashes REPORT RUNS SCENARIOS OUT

Needs `bwrap` on PATH. Writes OUT/data/python-crashes.json.
"""

import json
import re
import shutil
import subprocess
import sys
from collections import Counter
from pathlib import Path

import pandas as pd

from .data import SHORT, complete


def replay(bwrap, trial_dir, case_in):
    toolchains = json.loads((trial_dir / "run" / "record.json").read_text())["inputs"]["toolchains"]
    path = f"{toolchains['language']}/bin:{toolchains['base']}/bin"
    command = [bwrap, "--unshare-all", "--die-with-parent", "--ro-bind", "/nix/store", "/nix/store",
               "--ro-bind", str(trial_dir / "run" / "final"), "/work", "--chdir", "/work",
               "--dev", "/dev", "--proc", "/proc", "--tmpfs", "/tmp", "--setenv", "PATH", path,
               "--setenv", "HOME", "/tmp", "python3", "main.py"]
    try:
        with open(case_in) as stdin:
            done = subprocess.run(command, stdin=stdin, capture_output=True, timeout=30)
    except subprocess.TimeoutExpired:
        return "Timeout"
    lines = done.stderr.decode(errors="replace").strip().splitlines()
    return re.split(r"[:(]", lines[-1], maxsplit=1)[0].strip() if lines else "None"


def main(report, runs, scenarios, out):
    bwrap = shutil.which("bwrap")
    trials = complete(pd.read_csv(f"{report}/trials.csv"))
    cases = pd.read_csv(f"{report}/cases.csv")
    crashed = trials[(trials.language == "python") & (trials.failure == "crash")]
    first = cases[(cases.outcome == "crash")].merge(
        crashed[["system", "scenario", "language", "rep", "model"]],
        on=["system", "scenario", "language", "rep"]).groupby(["system", "scenario", "rep"]).head(1)
    result = {}
    for model, rows in first.groupby("model"):
        kinds, by_scenario = Counter(), {}
        for r in rows.itertuples():
            trial_dir = Path(runs) / r.experiment / "trials" / r.system / r.scenario / "python" / f"r{r.rep}"
            kind = replay(bwrap, trial_dir, Path(scenarios) / r.scenario / "hidden" / f"{r.case}.in")
            kinds[kind] += 1
            by_scenario.setdefault(r.scenario, Counter())[kind] += 1
        result[SHORT[model]] = {"trials": len(rows), "kinds": dict(kinds.most_common()),
                                "by_scenario": {s: dict(c) for s, c in by_scenario.items()}}
    Path(out, "data").mkdir(parents=True, exist_ok=True)
    Path(out, "data", "python-crashes.json").write_text(json.dumps(result, indent=1) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:5])
