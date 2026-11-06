"""Intervals and tests. Trials of one scenario share a task, so every interval resamples
scenarios first and trials within them second."""

import numpy as np
import pandas as pd
import statsmodels.formula.api as smf
from scipy import stats

RESAMPLES = 4000


def mean_interval(values, clusters, seed=0, level=0.95):
    """Mean of `values` with a two-stage bootstrap interval over `clusters`."""
    values = np.asarray(values, dtype=float)
    clusters = np.asarray(clusters)
    groups = [values[clusters == c] for c in np.unique(clusters)]
    rng = np.random.default_rng(seed)
    means = np.empty(RESAMPLES)
    for i in range(RESAMPLES):
        total = count = 0.0
        for g in rng.integers(len(groups), size=len(groups)):
            sample = rng.choice(groups[g], size=len(groups[g]))
            total += sample.sum()
            count += len(sample)
        means[i] = total / count
    tail = (1 - level) / 2
    return values.mean(), np.quantile(means, tail), np.quantile(means, 1 - tail)


def geomean_interval(values, clusters, seed=0):
    mean, lo, hi = mean_interval(np.log(values), clusters, seed)
    return np.exp(mean), np.exp(lo), np.exp(hi)


def paired(trials, a, b):
    """Exact McNemar test of languages a and b over trials matched on model, effort, scenario
    and generation: how often each passed where the other failed."""
    pairs = trials[trials.language.isin([a, b])].pivot_table(
        index=["model", "effort", "scenario", "rep"], columns="language", values="passed").dropna()
    only_a = int(((pairs[a] == 1) & (pairs[b] == 0)).sum())
    only_b = int(((pairs[a] == 0) & (pairs[b] == 1)).sum())
    p = stats.binomtest(only_a, only_a + only_b).pvalue if only_a + only_b else 1.0
    return {"pairs": len(pairs), "only_a": only_a, "only_b": only_b, "p": p,
            "both": int(((pairs[a] == 1) & (pairs[b] == 1)).sum())}


def logistic(trials):
    """Likelihood-ratio tests of language, and of its interaction with the scenario and the
    effort, in logistic models of pass or fail that already account for scenario and effort."""
    d = trials.assign(language=pd.Categorical(trials.language), effort=pd.Categorical(trials.effort))

    def fit(formula):
        return smf.logit(formula, d).fit(disp=0, maxiter=500, method="bfgs")

    base = fit("passed ~ C(scenario) + C(effort)")
    main = fit("passed ~ C(scenario) + C(effort) + C(language, Treatment('python'))")
    by_scenario = fit("passed ~ C(scenario) * C(language) + C(effort)")
    by_effort = fit("passed ~ C(scenario) + C(effort) * C(language)")

    def lr(small, big):
        chi2 = 2 * (big.llf - small.llf)
        df = big.df_model - small.df_model
        return {"chi2": chi2, "df": int(df), "p": stats.chi2.sf(chi2, df)}

    clustered = smf.logit("passed ~ C(scenario) + C(effort) + C(language, Treatment('python'))", d).fit(
        disp=0, maxiter=500, method="bfgs", cov_type="cluster",
        cov_kwds={"groups": pd.factorize(d.scenario + d.language.astype(str))[0]})
    odds = {}
    for name, coef in clustered.params.items():
        if "language" in name:
            lang = name.split("[T.")[1].rstrip("]")
            lo, hi = clustered.conf_int().loc[name]
            odds[lang] = {"or": np.exp(coef), "lo": np.exp(lo), "hi": np.exp(hi),
                          "p": clustered.pvalues[name]}
    return {"language": lr(base, main), "scenario_x_language": lr(main, by_scenario),
            "effort_x_language": lr(main, by_effort),
            "r2": {"base": base.prsquared, "language": main.prsquared,
                   "scenario_x_language": by_scenario.prsquared},
            "odds_vs_python": odds}


def language_index(trials, models, languages, seed=0, level=0.95):
    """The tokens an agent spends per correct program in each language, relative to the average
    language for the same model and task, as an index where the average language scores 100 and a
    higher score is cheaper.

    In one cell (model, task, language) a session passes with probability p and spends c tokens
    (the geometric mean over its sessions), so an agent that retries until a program passes
    spends c / p tokens per correct program. p is smoothed as (k + 1/2) / (n + 1), so a cell that
    never passed costs a finite multiple. A model's index for a language is 100 over the
    geometric mean, across tasks, of that cost relative to the task's average language; the
    overall index weighs every model equally. Intervals resample tasks, then sessions in a cell.
    """
    scenarios = sorted(trials.scenario.unique())
    cells = {}
    for (m, s, lang), d in trials.groupby(["model", "scenario", "language"]):
        cells[m, s, lang] = (d.passed.to_numpy(float), np.log(d.tokens.to_numpy(float)))

    def log_cost(pick_scenarios, rng):
        # log relative cost per model and language, averaged over the picked tasks.
        out = np.zeros((len(models), len(languages)))
        for s in pick_scenarios:
            for i, m in enumerate(models):
                row = np.empty(len(languages))
                for j, lang in enumerate(languages):
                    passed, logc = cells[m, s, lang]
                    if rng is not None:
                        take = rng.integers(len(passed), size=len(passed))
                        passed, logc = passed[take], logc[take]
                    p = (passed.sum() + 0.5) / (len(passed) + 1)
                    row[j] = logc.mean() - np.log(p)
                out[i] += row - row.mean()
        return out / len(pick_scenarios)

    point = log_cost(scenarios, None)
    rng = np.random.default_rng(seed)
    draws = np.empty((RESAMPLES, len(models) + 1, len(languages)))
    for r in range(RESAMPLES):
        pick = [scenarios[k] for k in rng.integers(len(scenarios), size=len(scenarios))]
        sample = log_cost(pick, rng)
        draws[r, :-1] = sample
        draws[r, -1] = sample.mean(axis=0)
    point = np.vstack([point, point.mean(axis=0)])
    tail = (1 - level) / 2
    lo, hi = np.quantile(draws, 1 - tail, axis=0), np.quantile(draws, tail, axis=0)
    score = lambda x: 100 * np.exp(-x)
    keys = list(models) + ["all"]
    return {k: {lang: {"score": float(score(point[i, j])), "lo": float(score(lo[i, j])),
                       "hi": float(score(hi[i, j]))} for j, lang in enumerate(languages)}
            for i, k in enumerate(keys)}
