"""The paper's figures. Rows are languages, panels are models, and intervals are 95% two-stage
bootstrap intervals over scenarios and trials."""

import os
from pathlib import Path

import matplotlib

matplotlib.use("svg")
import matplotlib.pyplot as plt  # noqa: E402
import numpy as np  # noqa: E402
from matplotlib import font_manager  # noqa: E402

from .data import EFFORTS, LANGUAGE_NAMES, MODEL_NAMES  # noqa: E402

COLUMN = 3.4  # inches, one column of the two-column layout
INK = "#222222"
MUTED = "#9a9a9a"
ACCENT = "#c0392b"
MARKERS = {"gpt-6-luna": "o", "gpt-6.1-terra": "s", "claude-haiku-5-5": "D"}


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


def dot_panels(rows, models, values, xlabel, out, name, *, log=False, ref=None, xlim=None,
               extra=None):
    """One panel per model; values[model][language] = (point, lo, hi)."""
    fig, axes = plt.subplots(1, len(models), sharey=True, layout="constrained",
                             figsize=(COLUMN, 0.18 * len(rows) + 0.55))
    axes = np.atleast_1d(axes)
    y = np.arange(len(rows))[::-1]
    for ax, model in zip(axes, models):
        point = np.array([values[model][r][0] for r in rows])
        lo = np.array([values[model][r][1] for r in rows])
        hi = np.array([values[model][r][2] for r in rows])
        if ref is not None:
            ax.axvline(ref, color=MUTED, lw=0.6, ls="--", zorder=0)
        ax.hlines(y, lo, hi, color=INK, lw=0.8)
        ax.plot(point, y, "o", ms=3.2, color=INK, zorder=3)
        if extra:
            extra(ax, model, y)
        ax.set_title(MODEL_NAMES[model], pad=3)
        if log:
            ax.set_xscale("log", base=2)
            ax.xaxis.set_major_locator(matplotlib.ticker.FixedLocator([0.25, 0.5, 1, 2, 4, 8]))
            ax.xaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}×"))
            ax.xaxis.set_minor_locator(matplotlib.ticker.NullLocator())
        if xlim:
            ax.set_xlim(*xlim)
        ax.grid(axis="x", color="#e6e6e6", lw=0.5, zorder=0)
    axes[0].set_yticks(y, [LANGUAGE_NAMES[r] for r in rows])
    axes[0].set_ylim(-0.6, len(rows) - 0.4)
    fig.supxlabel(xlabel, fontsize=8)
    save(fig, out, name)


def heatmap(table, out, name, *, label):
    """Scenario by language grid of pass rates in percent."""
    fig, ax = plt.subplots(figsize=(COLUMN, 2.5), layout="constrained")
    data = table.values
    ax.imshow(data, cmap="Greys", vmin=-10, vmax=130, aspect="auto")
    for i in range(data.shape[0]):
        for j in range(data.shape[1]):
            v = data[i, j]
            ax.text(j, i, f"{v:.0f}", ha="center", va="center", fontsize=7,
                    color="white" if v > 62 else INK)
    ax.set_xticks(range(data.shape[1]), [LANGUAGE_NAMES[c] for c in table.columns], rotation=35,
                  ha="right")
    ax.set_yticks(range(data.shape[0]), table.index)
    ax.tick_params(length=0)
    for s in ax.spines.values():
        s.set_visible(False)
    ax.set_title(label, pad=4)
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


def speed_strips(points, models, rows, out, name):
    """points[model][language] = per-trial runtime ratios to the same-language reference."""
    fig, axes = plt.subplots(1, len(models), sharey=True, layout="constrained",
                             figsize=(COLUMN, 0.18 * len(rows) + 0.6))
    axes = np.atleast_1d(axes)
    y = np.arange(len(rows))[::-1]
    rng = np.random.default_rng(1)
    for ax, model in zip(axes, models):
        ax.axvline(1, color=MUTED, lw=0.6, ls="--", zorder=0)
        for yi, lang in zip(y, rows):
            v = np.asarray(points[model].get(lang, []))
            if len(v) == 0:
                continue
            ax.plot(v, yi + rng.uniform(-0.22, 0.22, len(v)), "o", ms=1.4, color=MUTED,
                    alpha=0.6, mew=0, zorder=2)
            ax.plot([np.exp(np.log(v).mean())] * 2, [yi - 0.32, yi + 0.32], color=INK, lw=1.4,
                    zorder=3)
        ax.set_xscale("log", base=2)
        ax.set_xlim(0.25, 64)
        ax.xaxis.set_major_locator(matplotlib.ticker.FixedLocator([0.25, 1, 4, 16, 64]))
        ax.xaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}×"))
        ax.xaxis.set_minor_locator(matplotlib.ticker.NullLocator())
        ax.set_title(MODEL_NAMES[model], pad=3)
    axes[0].set_yticks(y, [LANGUAGE_NAMES[r] for r in rows])
    fig.supxlabel("runtime / same-language reference (lower is faster)", fontsize=8)
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
                mfc="white", mec=INK, mew=0.8, label=MODEL_NAMES[model])
    ax.set_xscale("log", base=2)
    ax.set_xlim(0.9, 40)
    ax.xaxis.set_major_locator(matplotlib.ticker.FixedLocator([1, 2, 4, 8, 16, 32]))
    ax.xaxis.set_major_formatter(matplotlib.ticker.FuncFormatter(lambda v, _: f"{v:g}×"))
    ax.xaxis.set_minor_locator(matplotlib.ticker.NullLocator())
    ax.grid(axis="x", color="#e6e6e6", lw=0.5, zorder=0)
    ax.set_yticks(y, [LANGUAGE_NAMES[r] for r in rows])
    ax.set_xlabel("runtime / fastest program for the workload, any language")
    fig.legend(frameon=False, fontsize=7, loc="outside upper center", ncol=1 + len(models),
               handletextpad=0.2, columnspacing=1.0)
    save(fig, out, name)
