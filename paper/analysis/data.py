"""The trials the paper analyses, read from a `gungnir report` directory."""

import numpy as np
import pandas as pd

from . import sessions

LANGUAGES = ["python", "javascript", "typescript", "go", "rust", "zig", "c", "cpp"]
LANGUAGE_NAMES = {"python": "Python", "javascript": "JavaScript", "typescript": "TypeScript",
                  "go": "Go", "rust": "Rust", "zig": "Zig", "c": "C", "cpp": "C++"}
EFFORTS = ["none", "low", "medium", "high", "xhigh", "max"]
MODELS = ["gpt-6-luna", "gpt-6.1-terra", "claude-haiku-5-5"]
MODEL_NAMES = {"gpt-6-luna": "GPT-6 Luna", "gpt-6.1-terra": "GPT-6.1 Terra",
               "claude-haiku-5-5": "Claude Haiku 5.5"}
SHORT = {"gpt-6-luna": "luna", "gpt-6.1-terra": "terra", "claude-haiku-5-5": "haiku"}

# A generation of one model at one effort enters the analysis once this share of its
# scenario-language cells is graded.
COMPLETE = 0.95


def complete(trials):
    """The graded trials of the generations that are complete enough to analyse."""
    trials = trials[trials.model.isin(MODELS) & trials.verdict.notna()]
    cells = trials.scenario.nunique() * len(LANGUAGES)
    graded = trials.groupby(["model", "effort", "rep"]).size()
    keep = graded[graded >= COMPLETE * cells].index
    return trials.set_index(["model", "effort", "rep"]).loc[keep].reset_index()


def load(report, runs):
    trials = complete(pd.read_csv(f"{report}/trials.csv"))

    trials["passed"] = (trials.verdict == "pass").astype(int)
    trials["case_share"] = trials.cases_passed / trials.cases_total
    trials["tokens"] = (trials.input_tokens + trials.cache_read_tokens
                        + trials.cache_write_tokens + trials.output_tokens)
    trials["outcome"] = trials.failure.fillna("pass")
    trials = trials.merge(pd.DataFrame(sessions.sessions(runs, trials)),
                          on=["system", "scenario", "language", "rep"])

    workloads = pd.read_csv(f"{report}/workloads.csv")
    workloads = workloads[workloads.outcome == "timed"].merge(
        trials[["system", "scenario", "language", "rep", "model", "effort"]],
        on=["system", "scenario", "language", "rep"])
    return trials, workloads


def relative_to_task(trials, column):
    """Each trial's value over the geometric mean of the same task (scenario, model, effort and
    generation) across the languages, so 1.0 is an average language."""
    logged = np.log(trials[column].clip(lower=1e-9))
    task_mean = logged.groupby([trials.model, trials.effort, trials.scenario, trials.rep]).transform("mean")
    return np.exp(logged - task_mean)


def generations(trials):
    """The generations analysed for each model and effort."""
    return trials.groupby(["model", "effort"]).rep.nunique()
