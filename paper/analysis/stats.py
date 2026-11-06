"""Intervals and tests. Trials of one scenario share a task, so every interval resamples
scenarios first and trials within them second."""

import warnings

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



# The gungnir index's dimensions: the trial column each reads, whether it reads every session or
# only the passing ones, and whether a larger value is better.
DIMENSIONS = {
    "correctness": ("passed", "all", True),
    "tokens": ("log_tokens", "all", False),
    "time": ("log_wall", "all", False),
    "runtime": ("log_runtime", "passing", False),
    "complexity": ("log_ccn", "passing", False),
}


def language_index(trials, models, languages, seed=0, level=0.95):
    """The gungnir index: how a language compares with the average language for the same model
    and task on correctness, tokens, session time, program runtime and code complexity, folded into
    one score where the average language scores 100 and higher is better.

    In each cell (model, task, language) a dimension is the smoothed pass rate (k + 1/2) / (n + 1),
    or the mean logarithm of a session's tokens or wall time over every session, or of a passing
    program's runtime (geometric mean over workloads, against the fastest reference for the
    workload) or total cyclomatic complexity. Its advantage is the logarithm of the cell's value
    against the mean over the languages of the same model and task, signed so that better is
    positive, averaged over tasks. A dimension's score is 100 times the exponential of its mean
    advantage over the models that have it, and the index is 100 times the exponential of the mean
    over dimensions: a geometric mean with every dimension and every model weighed equally.
    Intervals resample tasks, then sessions within a cell.
    """
    scenarios = sorted(trials.scenario.unique())
    cols = ["passed", "log_tokens", "log_wall", "log_runtime", "log_ccn"]
    cells = {key: d[cols].to_numpy(float) for key, d in trials.groupby(["model", "scenario", "language"])}
    dims = list(DIMENSIONS)

    def advantage(pick, rng):
        # adv[model, dimension, language], averaged over the picked tasks; NaN where a model lacks one.
        total = np.zeros((len(models), len(dims), len(languages)))
        count = np.zeros_like(total)
        for s in pick:
            for i, m in enumerate(models):
                v = np.full((len(dims), len(languages)), np.nan)
                for j, lang in enumerate(languages):
                    x = cells.get((m, s, lang))
                    if x is None:
                        continue
                    if rng is not None:
                        x = x[rng.integers(len(x), size=len(x))]
                    passing = x[x[:, 0] == 1]
                    v[0, j] = np.log((x[:, 0].sum() + 0.5) / (len(x) + 1))
                    v[1, j] = x[:, 1].mean()
                    v[2, j] = x[:, 2].mean()
                    for d, c in ((3, 3), (4, 4)):
                        vals = passing[:, c][~np.isnan(passing[:, c])]
                        v[d, j] = vals.mean() if len(vals) else np.nan
                with np.errstate(all="ignore"), warnings.catch_warnings():
                    warnings.simplefilter("ignore", RuntimeWarning)
                    rel = v - np.nanmean(v, axis=1, keepdims=True)
                for d, name in enumerate(dims):
                    if not DIMENSIONS[name][2]:
                        rel[d] = -rel[d]
                ok = ~np.isnan(rel)
                total[i][ok] += rel[ok]
                count[i][ok] += 1
        with np.errstate(all="ignore"):
            return total / count

    def fold(adv):
        # dimension scores over models, and the index, per language; models and dimensions weigh equally.
        with np.errstate(all="ignore"), warnings.catch_warnings():
            warnings.simplefilter("ignore", RuntimeWarning)
            per_model_index = np.nanmean(adv, axis=1)
            dim_all = np.nanmean(adv, axis=0)
            return per_model_index, dim_all, np.nanmean(dim_all, axis=0)

    point = fold(advantage(scenarios, None))
    rng = np.random.default_rng(seed)
    draws = [[], [], []]
    for _ in range(RESAMPLES):
        pick = [scenarios[k] for k in rng.integers(len(scenarios), size=len(scenarios))]
        for a, b in zip(draws, fold(advantage(pick, rng))):
            a.append(b)
    tail = (1 - level) / 2
    score = lambda x: 100 * np.exp(x)
    bounds = [(np.nanquantile(np.array(d), tail, axis=0), np.nanquantile(np.array(d), 1 - tail, axis=0))
              for d in draws]

    def entry(p, lo, hi):
        return {"score": float(score(p)), "lo": float(score(lo)), "hi": float(score(hi))}

    out = {m: {lang: entry(point[0][i, j], bounds[0][0][i, j], bounds[0][1][i, j])
               for j, lang in enumerate(languages)} for i, m in enumerate(models)}
    out["all"] = {lang: entry(point[2][j], bounds[2][0][j], bounds[2][1][j]) for j, lang in enumerate(languages)}
    out["dimensions"] = {name: {lang: entry(point[1][d, j], bounds[1][0][d, j], bounds[1][1][d, j])
                                for j, lang in enumerate(languages)} for d, name in enumerate(dims)}
    return out
