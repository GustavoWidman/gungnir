"""The trials the paper analyses, read from a `gungnir report` directory."""

import numpy as np
import pandas as pd

from . import sessions

LANGUAGES = ["python", "javascript", "typescript", "go", "rust", "zig", "c", "cpp"]
LANGUAGE_NAMES = {"python": "Python", "javascript": "JavaScript", "typescript": "TypeScript",
                  "go": "Go", "rust": "Rust", "zig": "Zig", "c": "C", "cpp": "C++"}
EFFORTS = ["none", "low", "medium", "high", "xhigh", "max"]
MODELS = ["gpt-6-luna", "gpt-6.1-terra", "gpt-6.1-sol", "gpt-6-astra", "nemotron-3.5-super-vl",
          "deepseek-v4.1-flash", "claude-haiku-5-5", "claude-sonnet-5-5", "claude-opus-5-5"]
MODEL_NAMES = {"gpt-6-luna": "GPT-6 Luna", "gpt-6.1-terra": "GPT-6.1 Terra", "gpt-6.1-sol": "GPT-6.1 Sol",
               "gpt-6-astra": "GPT-6 Astra", "nemotron-3.5-super-vl": "Nemotron 3.5 Super VL",
               "deepseek-v4.1-flash": "DeepSeek V4.1 Flash", "claude-haiku-5-5": "Claude Haiku 5.5",
               "claude-sonnet-5-5": "Claude Sonnet 5.5", "claude-opus-5-5": "Claude Opus 5.5"}
SHORT = {"gpt-6-luna": "luna", "gpt-6.1-terra": "terra", "gpt-6.1-sol": "sol", "gpt-6-astra": "astra",
         "nemotron-3.5-super-vl": "nemotron", "deepseek-v4.1-flash": "deepseek",
         "claude-haiku-5-5": "haiku", "claude-sonnet-5-5": "sonnet", "claude-opus-5-5": "opus"}
# The short title of each model's panel in a figure.
PANEL = {m: SHORT[m].capitalize() if m != "deepseek-v4.1-flash" else "DeepSeek" for m in MODELS}


def complete(trials, index):
    """The graded trials of the generations the gungnir index in `index` counted: those graded
    completely enough at every effort of their model."""
    generations = pd.read_csv(f"{index}/generations.csv")
    counted = generations[generations.counted][["system", "rep"]]
    trials = trials[trials.model.isin(MODELS) & trials.verdict.notna()]
    return trials.merge(counted, on=["system", "rep"])


def load(report, index, runs):
    trials = complete(pd.read_csv(f"{report}/trials.csv"), index)

    trials["passed"] = (trials.verdict == "pass").astype(int)
    trials["case_share"] = trials.cases_passed / trials.cases_total
    trials["tokens"] = (trials.input_tokens + trials.cache_read_tokens + trials.cache_write_tokens
                        + trials.cache_write_1h_tokens + trials.output_tokens)
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
