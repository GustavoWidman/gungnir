"""Writes the paper's figures and the numbers its text cites.

    python -m analysis REPORT RUNS OUT

REPORT is a `gungnir report` directory for the study experiments, RUNS the results directory
those experiments write to, and OUT the paper directory, which receives `figures/*.svg` and
`data/numbers.json`.
"""

import json
import sys
from pathlib import Path

import numpy as np

from . import data, figures, stats
from .data import EFFORTS, LANGUAGES, MODELS, SHORT


def by_language(trials, fn):
    return {lang: fn(trials[trials.language == lang]) for lang in LANGUAGES}


def pass_rate(d):
    rate, lo, hi = stats.mean_interval(d.passed, d.scenario)
    return {"rate": 100 * rate, "lo": 100 * lo, "hi": 100 * hi, "k": int(d.passed.sum()), "n": len(d)}


def main(report, runs, out):
    out = Path(out)
    (out / "figures").mkdir(parents=True, exist_ok=True)
    (out / "data").mkdir(parents=True, exist_ok=True)
    trials, workloads = data.load(report, runs)
    models = [m for m in MODELS if m in set(trials.model)]
    per = {m: trials[trials.model == m] for m in models}
    n = {}

    n["data"] = {
        "trials": len(trials), "scenarios": int(trials.scenario.nunique()),
        "languages": len(LANGUAGES), "tokens": int(trials.tokens.sum()),
        "cost_usd": float(trials.cost_usd.sum()), "agent_hours": float(trials.wall_s.sum() / 3600),
        "models": {SHORT[m]: {"trials": len(d), "efforts": [e for e in EFFORTS if e in set(d.effort)],
                              "generations": {e: int(g) for e, g in d.groupby("effort").rep.nunique().items()},
                              "tokens": int(d.tokens.sum()), "cost_usd": float(d.cost_usd.sum()),
                              "measured": int(workloads[workloads.model == m].groupby(
                                  ["scenario", "language", "rep", "effort"]).ngroups)}
                   for m, d in per.items()},
    }

    # Correctness.
    n["pass"] = {SHORT[m]: by_language(d, pass_rate) | {"all": pass_rate(d)} for m, d in per.items()}
    n["cases"] = {SHORT[m]: by_language(d, lambda x: 100 * x.case_share.mean()) for m, d in per.items()}
    n["outcomes"] = {SHORT[m]: by_language(d, lambda x: {k: 100 * v for k, v in
                                                          x.outcome.value_counts(normalize=True).items()})
                     for m, d in per.items()}
    order = sorted(LANGUAGES, key=lambda lang: -n["pass"][SHORT[models[0]]][lang]["rate"])
    figures.style()
    figures.dot_panels(order, models, {m: {lang: (v["rate"], v["lo"], v["hi"]) for lang, v in
                                           n["pass"][SHORT[m]].items() if lang != "all"}
                                       for m in models},
                       "trials passing every hidden test (%)", out / "figures", "pass-by-language",
                       xlim=(0, 104))

    luna = per.get("gpt-6-luna")
    if luna is not None:
        grid = luna.pivot_table(index="scenario", columns="language", values="passed")[LANGUAGES] * 100
        n["scenario_grid"] = {s: {lang: float(v) for lang, v in row.items()} for s, row in grid.iterrows()}
        figures.heatmap(grid, out / "figures", "luna-scenarios", label="")
        rates = {}
        for lang in LANGUAGES:
            series = []
            for e in EFFORTS:
                d = luna[(luna.language == lang) & (luna.effort == e)]
                r, lo, hi = stats.mean_interval(d.passed, d.scenario)
                series.append((e, 100 * r, 100 * lo, 100 * hi))
            rates[lang] = series
        mean = [100 * luna[luna.effort == e].passed.mean() for e in EFFORTS]
        n["effort"] = {lang: {e: r for e, r, _, _ in s} for lang, s in rates.items()} | {
            "all": dict(zip(EFFORTS, mean))}
        figures.effort_multiples(rates, mean, out / "figures", "luna-effort")
        n["logistic"] = stats.logistic(luna)

    pairs = [("javascript", "typescript"), ("python", "rust"), ("c", "cpp"), ("python", "go"),
             ("rust", "go"), ("python", "typescript"), ("python", "javascript")]
    n["paired"] = {SHORT[m]: {f"{a}-{b}": stats.paired(d, a, b) for a, b in pairs}
                   for m, d in per.items()}

    # What the session cost, relative to an average language for the same task.
    n["cost"] = {}
    cost_points = {}
    for m, d in per.items():
        d = d.assign(**{f"{c}_rel": data.relative_to_task(d, c)
                        for c in ("tokens", "wall_s", "output_tokens", "commands", "cost_usd")})
        n["cost"][SHORT[m]] = by_language(d, lambda x: {
            c: dict(zip(("ratio", "lo", "hi"), stats.geomean_interval(x[f"{c}_rel"], x.scenario)))
            for c in ("tokens", "wall_s", "output_tokens", "commands", "cost_usd")})
        cost_points[m] = {lang: tuple(v["tokens"][k] for k in ("ratio", "lo", "hi"))
                          for lang, v in n["cost"][SHORT[m]].items()}
    figures.dot_panels(order, models, cost_points, "tokens relative to the same task's average language",
                       out / "figures", "tokens-by-language", log=True, ref=1)

    # What the agent saw while it worked.
    n["session"] = {SHORT[m]: by_language(d, lambda x: {
        "failed_builds": float(x.failed_builds.mean()), "diagnostics": float(x.diagnostics.mean()),
        "runtime_failures": float(x.runtime_failures.mean()), "commands": float(x.commands.median()),
        "sessions_with_failed_build": 100 * float((x.failed_builds > 0).mean()),
        "failed_builds_ci": stats.mean_interval(x.failed_builds, x.scenario)})
        for m, d in per.items()}
    figures.dot_panels(order, models,
                       {m: {lang: tuple(n["session"][SHORT[m]][lang]["failed_builds_ci"]) for lang in LANGUAGES}
                        for m in models},
                       "builds that failed with a compiler or type error, per session",
                       out / "figures", "failed-builds")
    kinds = {}
    for lang, cols in (("zig", ["api", "rules", "types", "syntax", "other"]),
                       ("rust", ["borrow", "other"]), ("typescript", ["null", "other"])):
        d = trials[trials.language == lang]
        kinds[lang] = {c: int(d[f"diag_{c}"].sum()) for c in cols} | {"sessions": len(d),
                                                                       "total": int(d.diagnostics.sum())}
    n["diagnostic_kinds"] = kinds

    # Runtime of the passing programs.
    per_trial = workloads.groupby(["model", "effort", "scenario", "language", "rep"]).agg(
        ratio=("wall_ratio", lambda v: float(np.exp(np.log(v).mean())))).reset_index()
    n["speed"] = {}
    points = {}
    for m in models:
        d = per_trial[per_trial.model == m]
        if d.empty:
            continue
        points[m] = {lang: d[d.language == lang].ratio.values for lang in LANGUAGES}
        n["speed"][SHORT[m]] = {lang: dict(zip(("ratio", "lo", "hi"), stats.geomean_interval(x.ratio, x.scenario)))
                                | {"n": len(x), "slower_share": 100 * float((x.ratio > 1.1).mean())}
                                for lang in LANGUAGES if len(x := d[d.language == lang])}
        n["speed"][SHORT[m]]["by_effort"] = {e: float(np.exp(np.log(d[d.effort == e].ratio).mean()))
                                             for e in EFFORTS if len(d[d.effort == e])}
    speed_models = [m for m in models if m in points]
    figures.speed_strips(points, speed_models, order, out / "figures", "speed")

    best = workloads.groupby(["scenario", "workload"]).reference_median_s.min().rename("best")
    w = workloads.join(best, on=["scenario", "workload"])
    absolute = {"reference": {}}
    for lang in LANGUAGES:
        refs = w[w.language == lang].groupby(["scenario", "workload"]).agg(
            r=("reference_median_s", "median"), b=("best", "first"))
        absolute["reference"][lang] = float(np.exp(np.log(refs.r / refs.b).mean()))
    for m in speed_models:
        x = w[w.model == m]
        absolute[SHORT[m]] = {lang: float(np.exp(np.log(g.candidate_median_s / g.best).mean()))
                              for lang, g in x.groupby("language")}
    n["speed_absolute"] = absolute
    figures.absolute_speed({"reference": absolute["reference"]} | {m: absolute[SHORT[m]] for m in speed_models},
                           order, speed_models, out / "figures", "speed-absolute")

    # One number per language: tokens per correct program against the task's average language.
    index = stats.language_index(trials, models, LANGUAGES)
    n["index"] = {(SHORT[k] if k in SHORT else k): v for k, v in index.items()}

    n["order"] = order
    (out / "data" / "numbers.json").write_text(json.dumps(n, indent=1, default=float) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:4])
