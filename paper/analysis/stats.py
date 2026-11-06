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
