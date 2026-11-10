"""Writes the paper's figures and the numbers its text cites.

    python -m analysis REPORT INDEX RUNS OUT

REPORT is a `gungnir report` directory for the study experiments, INDEX a `gungnir index`
directory for the same experiments, RUNS the results directory those experiments write to, and
OUT the paper directory, which receives `figures/*.svg` and `data/numbers.json`.
"""

import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd

from . import data, figures, stats
from .data import EFFORTS, LANGUAGES, MODELS, SHORT


def by_language(trials, fn):
    return {lang: fn(trials[trials.language == lang]) for lang in LANGUAGES}


def pass_rate(d):
    rate, lo, hi = stats.mean_interval(d.passed, d.scenario)
    return {"rate": 100 * rate, "lo": 100 * lo, "hi": 100 * hi, "k": int(d.passed.sum()), "n": len(d)}


def main(report, index_dir, runs, out):
    out = Path(out)
    (out / "figures").mkdir(parents=True, exist_ok=True)
    (out / "data").mkdir(parents=True, exist_ok=True)
    trials, workloads = data.load(report, index_dir, runs)
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
    figures.model_grid({m: {lang: v["rate"] for lang, v in n["pass"][SHORT[m]].items()} for m in models},
                       models, out / "figures", "pass-by-language", fmt=lambda v: f"{v:.0f}",
                       vmin=20, vmax=100)

    # The models that pass nearly everything, and those whose pass rates leave the language room
    # to matter.
    strong = [m for m in models if per[m].passed.mean() >= 0.9]
    weak = [m for m in models if m not in strong]
    groups = {"strong": trials[trials.model.isin(strong)], "weak": trials[trials.model.isin(weak)]}
    n["groups"] = {"strong": [SHORT[m] for m in strong], "weak": [SHORT[m] for m in weak]}
    n["outcomes_group"] = {g: by_language(d, lambda x: {k: 100 * v for k, v in
                                                         x.outcome.value_counts(normalize=True).items()})
                           for g, d in groups.items()}
    slow = groups["strong"][(groups["strong"].language == "python") & (groups["strong"].outcome == "timeout")]
    n["strong_python_timeouts"] = {"total": len(slow)} | {s: int(k) for s, k in slow.scenario.value_counts().items()}

    grids = {m: per[m].pivot_table(index="scenario", columns="language", values="passed")[LANGUAGES] * 100
             for m in weak}
    n["scenario_grid"] = {SHORT[m]: {s: {lang: float(v) for lang, v in row.items()} for s, row in g.iterrows()}
                          for m, g in grids.items()}
    figures.heatmaps(grids, out / "figures", "weak-scenarios")

    n["effort_models"] = {SHORT[m]: {e: 100 * float(d[d.effort == e].passed.mean())
                                     for e in EFFORTS if (d.effort == e).any()} for m, d in per.items()}
    figures.effort_models(n["effort_models"], models, weak, out / "figures", "effort-models")

    n["logistic"] = stats.logistic(trials, "system")
    luna = per.get("gpt-6-luna")
    if luna is not None:
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
        n["logistic_luna"] = stats.logistic(luna, "effort")

    pairs = [("javascript", "typescript"), ("python", "rust"), ("c", "cpp"), ("python", "go"),
             ("rust", "go"), ("python", "typescript"), ("python", "javascript")]
    n["paired"] = {SHORT[m]: {f"{a}-{b}": stats.paired(d, a, b) for a, b in pairs}
                   for m, d in per.items()} | {
        g: {f"{a}-{b}": stats.paired(d, a, b) for a, b in pairs}
        for g, d in (("all", trials), ("strong", groups["strong"]))}

    # What the session cost, relative to an average language for the same task.
    n["cost"] = {}
    for m, d in per.items():
        d = d.assign(**{f"{c}_rel": data.relative_to_task(d, c)
                        for c in ("tokens", "wall_s", "output_tokens", "commands", "cost_usd")})
        n["cost"][SHORT[m]] = by_language(d, lambda x: {
            c: dict(zip(("ratio", "lo", "hi"), stats.geomean_interval(x[f"{c}_rel"], x.scenario)))
            for c in ("tokens", "wall_s", "output_tokens", "commands", "cost_usd")})
    figures.model_grid({m: {lang: v["tokens"]["ratio"] for lang, v in n["cost"][SHORT[m]].items()}
                        for m in models}, models, out / "figures", "tokens-by-language",
                       fmt=lambda v: f"{v:.1f}", vmin=0.5, vmax=4, log=True)

    # What the agent saw while it worked.
    n["session"] = {SHORT[m]: by_language(d, lambda x: {
        "failed_builds": float(x.failed_builds.mean()), "diagnostics": float(x.diagnostics.mean()),
        "runtime_failures": float(x.runtime_failures.mean()), "commands": float(x.commands.median()),
        "sessions_with_failed_build": 100 * float((x.failed_builds > 0).mean()),
        "failed_builds_ci": stats.mean_interval(x.failed_builds, x.scenario)})
        for m, d in per.items()}
    figures.model_grid({m: {lang: n["session"][SHORT[m]][lang]["failed_builds"] for lang in LANGUAGES}
                        for m in models}, models, out / "figures", "failed-builds",
                       fmt=lambda v: f"{v:.1f}" if v < 10 else f"{v:.0f}", vmin=0.05, vmax=20, log=True)
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
    for m in models:
        d = per_trial[per_trial.model == m]
        if d.empty:
            continue
        n["speed"][SHORT[m]] = {lang: dict(zip(("ratio", "lo", "hi"), stats.geomean_interval(x.ratio, x.scenario)))
                                | {"n": len(x), "slower_share": 100 * float((x.ratio > 1.1).mean())}
                                for lang in LANGUAGES if len(x := d[d.language == lang])}
        n["speed"][SHORT[m]]["by_effort"] = {e: float(np.exp(np.log(d[d.effort == e].ratio).mean()))
                                             for e in EFFORTS if len(d[d.effort == e])}
    speed_models = [m for m in models if SHORT[m] in n["speed"]]
    figures.model_grid({m: {lang: v["ratio"] for lang, v in n["speed"][SHORT[m]].items() if lang in LANGUAGES}
                        for m in speed_models}, speed_models, out / "figures", "speed",
                       fmt=lambda v: f"{v:.1f}", vmin=0.5, vmax=3, log=True)

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

    # The gungnir index, as `gungnir index` scores it.
    scores = pd.read_csv(Path(index_dir) / "scores.csv")
    entry = lambda r: {"score": float(r.score), "lo": float(r.low), "hi": float(r.high)}
    index = {(SHORT.get(scope, scope)): {r.language: entry(r) for r in g[g.dimension == "index"].itertuples()}
             for scope, g in scores.groupby("scope")}
    overall = scores[scores.scope == "all"]
    index["dimensions"] = {d: {r.language: entry(r) for r in g.itertuples()}
                           for d, g in overall[overall.dimension != "index"].groupby("dimension")}
    n["index"] = index
    by_index = sorted(LANGUAGES, key=lambda lang: -index["all"][lang]["score"])
    figures.index_bars(by_index, {lang: (v["score"], v["lo"], v["hi"]) for lang, v in index["all"].items()},
                       out / "figures", "gungnir-index")

    n["order"] = order
    (out / "data" / "numbers.json").write_text(json.dumps(n, indent=1, default=float) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:5])
