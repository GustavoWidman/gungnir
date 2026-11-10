"""The paper's figures, each one column wide. Grids put models in rows and languages in columns,
and intervals are 95% two-stage bootstrap intervals over scenarios and trials."""

import os
from pathlib import Path

import matplotlib

matplotlib.use("svg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
from matplotlib import font_manager  # noqa: E402

from .data import EFFORTS, LANGUAGE_NAMES, LANGUAGES, MODEL_NAMES, PANEL, SHORT  # noqa: E402

COLUMN = 3.4  # inches, one column of the two-column layout
INK = "#222222"
MUTED = "#9a9a9a"
ACCENT = "#c0392b"
SHADE = matplotlib.colors.LinearSegmentedColormap.from_list("shade", ["#f5f5f5", "#262626"])
MARKERS = dict(zip(MODEL_NAMES, "osD^v<>ph"))


def style():
    for root in os.environ.get("TYPST_FONT_PATHS", "").split(":"):
        for path in Path(root).rglob("texgyretermes-*.otf") if root else []:
            font_manager.fontManager.addfont(str(path))
    plt.rcParams.update({
        "font.family": "TeX Gyre Termes", "font.size": 8, "axes.titlesize": 8,
        "axes.labelsize": 8, "xtick.labelsize": 7, "ytick.labelsize": 7.5,
        "axes.edgecolor": INK, "axes.linewidth": 0.6, "xtick.major.width": 0.6,
        "ytick.major.width": 0.6, "xtick.major.size": 2.5, "ytick.major.size": 0,
        "axes.spines.top": False, "axes.spines.right": False, "svg.fonttype": "path",
        "lines.linewidth": 0.9, "figure.dpi": 150, "svg.hashsalt": "gungnir",
    })


def save(fig, out, name):
    fig.savefig(Path(out) / f"{name}.svg", bbox_inches="tight", pad_inches=0.02,
                metadata={"Date": None})
    plt.close(fig)


def index_bars(rows, values, out, name):
    """The gungnir index as one bar per language from the best down, values[language] =
    (score, lo, hi), with its interval and a line at the average language's 100. A bar is dark
    when its interval lies above 100, light when it lies below, and grey otherwise."""
    fig, ax = plt.subplots(figsize=(COLUMN, 0.2 * len(rows) + 0.45), layout="constrained")
    y = np.arange(len(rows))[::-1]
    score = np.array([values[r][0] for r in rows])
    lo = np.array([values[r][1] for r in rows])
    hi = np.array([values[r][2] for r in rows])
    shade = ["#3a3a3a" if l > 100 else "#d4d4d4" if h < 100 else "#8c8c8c" for l, h in zip(lo, hi)]
    ax.barh(y, score, height=0.68, color=shade, zorder=2)
    ax.errorbar(score, y, xerr=[score - lo, hi - score], fmt="none", ecolor=INK, elinewidth=0.7,
                capsize=1.8, capthick=0.7, zorder=3)
    for yi, h, v in zip(y, hi, score):
        ax.text(max(h, 100) + 3, yi, f"{v:.0f}", va="center", ha="left", fontsize=7.5, color=INK)
    ax.axvline(100, color=INK, lw=0.6, ls="--", zorder=4)
    ax.set_yticks(y, [LANGUAGE_NAMES[r] for r in rows])
    ax.set_ylim(-0.6, len(rows) - 0.4)
    ax.set_xlim(0, max(hi) + 22)
    ax.xaxis.set_major_locator(matplotlib.ticker.FixedLocator([0, 50, 100, 150]))
    ax.grid(axis="x", color="#e6e6e6", lw=0.5, zorder=0)
    ax.set_xlabel("gungnir index (average language = 100)", fontsize=8)
    save(fig, out, name)

def grid_axes(ax, data, norm, fmt, columns, rows):
    """Shades and labels one grid of values, darker for higher; NaN cells stay blank."""
    shade = np.clip(np.nan_to_num(data, nan=norm.vmin), norm.vmin, norm.vmax)
    ax.imshow(np.ma.masked_where(np.isnan(data), shade), cmap=SHADE, norm=norm, aspect="auto")
    for i in range(data.shape[0]):
        for j in range(data.shape[1]):
            if not np.isnan(v := data[i, j]):
                ax.text(j, i, fmt(v), ha="center", va="center", fontsize=6.5,
                        color="white" if norm(np.clip(v, norm.vmin, norm.vmax)) > 0.55 else INK)
    ax.set_xticks(range(len(columns)), [LANGUAGE_NAMES[c] for c in columns], rotation=35, ha="right")
    ax.set_yticks(range(len(rows)), rows)
    ax.tick_params(length=0)
    for spine in ax.spines.values():
        spine.set_visible(False)


def model_grid(values, models, out, name, *, fmt, vmin, vmax, log=False):
    """Model by language grid, values[model][language], shaded darker for higher values."""
    data = np.array([[values[m].get(lang, np.nan) for lang in LANGUAGES] for m in models])
    norm = (matplotlib.colors.LogNorm if log else matplotlib.colors.Normalize)(vmin, vmax)
    fig, ax = plt.subplots(figsize=(COLUMN, 0.19 * len(models) + 0.5), layout="constrained")
    grid_axes(ax, data, norm, fmt, LANGUAGES, [PANEL[m] for m in models])
    save(fig, out, name)


def heatmaps(tables, out, name):
    """One scenario by language grid of pass rates in percent per model, stacked."""
    fig, axes = plt.subplots(len(tables), 1, sharex=True, layout="constrained",
                             figsize=(COLUMN, 1.75 * len(tables) + 0.35))
    for ax, (model, table) in zip(np.atleast_1d(axes), tables.items()):
        grid_axes(ax, table.values, matplotlib.colors.Normalize(-10, 130), lambda v: f"{v:.0f}",
                  list(table.columns), list(table.index))
        ax.set_title(MODEL_NAMES[model], pad=3)
    save(fig, out, name)


def effort_models(rates, models, labelled, out, name):
    """rates[short][effort] = pass rate over languages; each model is a line across the efforts it
    ran, black and named for the models in `labelled`, grey for the rest."""
    fig, ax = plt.subplots(figsize=(COLUMN, 1.9), layout="constrained")
    x = {e: i for i, e in enumerate(EFFORTS)}
    for model in models:
        r = rates[SHORT[model]]
        xs, ys = [x[e] for e in r], list(r.values())
        dark = model in labelled
        ax.plot(xs, ys, "-o", color=INK if dark else MUTED, lw=1.0 if dark else 0.7,
                ms=2.6 if dark else 1.8, zorder=3 if dark else 2)
        if dark:
            ax.annotate(PANEL[model], (xs[-1], ys[-1]), xytext=(4, 0), textcoords="offset points",
                        va="center", fontsize=7, color=INK)
    ax.set_xticks(range(len(EFFORTS)), EFFORTS)
    ax.set_xlim(-0.3, len(EFFORTS) - 0.4)
    ax.set_ylim(0, 102)
    ax.set_yticks([0, 25, 50, 75, 100])
    ax.tick_params(axis="y", length=2.5)
    ax.grid(axis="y", color="#e6e6e6", lw=0.5, zorder=0)
    ax.set_ylabel("pass rate (%)")
    ax.set_xlabel("reasoning effort")
    save(fig, out, name)


def effort_multiples(rates, mean, out, name):
    """rates[language] = list of (effort, rate, lo, hi); mean = list of rates over languages."""
    fig, axes = plt.subplots(2, 4, sharex=True, sharey=True, layout="constrained",
                             figsize=(COLUMN, 2.15))
    x = np.arange(len(EFFORTS))
    for ax, (lang, series) in zip(axes.flat, rates.items()):
        r = np.array([s[1] for s in series])
        ax.fill_between(x, [s[2] for s in series], [s[3] for s in series], color="#dddddd", lw=0)
        ax.plot(x, mean, color=MUTED, lw=0.8, ls="--")
        ax.plot(x, r, "-o", color=INK, ms=2.2)
        ax.set_title(LANGUAGE_NAMES[lang], pad=2)
        ax.set_ylim(0, 100)
        ax.set_yticks([0, 50, 100])
        ax.tick_params(axis="y", length=2.5)
    for ax in axes[1]:
        ax.set_xticks(x, ["none", "low", "med", "high", "xhigh", "max"], rotation=90)
    fig.supylabel("pass rate (%)", fontsize=8)
    save(fig, out, name)


def absolute_speed(values, rows, models, out, name):
    """values[key][language] = geometric-mean runtime over the fastest program for the
    workload, with key 'reference' or a model."""
    fig, ax = plt.subplots(figsize=(COLUMN, 0.18 * len(rows) + 0.5), layout="constrained")
    y = np.arange(len(rows))[::-1]
    ax.plot([values["reference"][r] for r in rows], y, "|", ms=7, mew=1.2, color=INK,
            label="reference")
    for model in models:
        ax.plot([values[model].get(r, np.nan) for r in rows], y, MARKERS[model], ms=3.4,
                mfc="white", mec=INK, mew=0.8, label=PANEL[model])
    ax.set_xscale("log", base=2)
    ax.set_xlim(0.9, 40)
    ax.xaxis.set_major_locator(matplotlib.ticker.FixedLocator([1, 2, 4, 8, 16, 32]))
    ax.xaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}×"))
    ax.xaxis.set_minor_locator(matplotlib.ticker.NullLocator())
    ax.grid(axis="x", color="#e6e6e6", lw=0.5, zorder=0)
    ax.set_yticks(y, [LANGUAGE_NAMES[r] for r in rows])
    ax.set_xlabel("runtime / fastest program for the workload, any language")
    fig.legend(frameon=False, fontsize=7, loc="outside upper center", ncol=5,
               handletextpad=0.2, columnspacing=1.0)
    save(fig, out, name)
