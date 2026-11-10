"""How the gungnir index ranks the languages under other weights of its five dimensions: with
each dimension left out or counted twice, and over weightings drawn uniformly at random from all
weightings that sum to one.

    python -m analysis.weights INDEX OUT

Writes OUT/data/weights.json.
"""

import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd

DIMENSIONS = ["pass", "tokens", "time", "speed", "simplicity"]
DRAWS = 100_000
SEED = 2026


def main(index, out):
    scores = pd.read_csv(Path(index, "scores.csv"))
    overall = scores[(scores.scope == "all") & scores.dimension.isin(DIMENSIONS)]
    # The index is the weighted geometric mean of the dimension scores, so it is linear in logs.
    logs = np.log(overall.pivot(index="language", columns="dimension", values="score")[DIMENSIONS] / 100)
    languages = list(logs.index)

    def ranking(weights):
        w = np.asarray(weights, float)
        index = pd.Series(100 * np.exp(logs.to_numpy() @ (w / w.sum())), index=languages)
        return index.sort_values(ascending=False)

    shifted = {}
    for i, dimension in enumerate(DIMENSIONS):
        for name, weight in (("without", 0), ("double", 2)):
            weights = [1] * len(DIMENSIONS)
            weights[i] = weight
            r = ranking(weights)
            shifted[f"{name}-{dimension}"] = {"order": list(r.index), "index": r.to_dict()}

    weights = np.random.default_rng(SEED).dirichlet(np.ones(len(DIMENSIONS)), DRAWS)
    index = logs.to_numpy() @ weights.T
    order = np.argsort(-index, axis=0)
    rank = np.argsort(order, axis=0) + 1
    share = lambda hits: {l: float(hits[i].mean()) for i, l in enumerate(languages)}
    drawn = {
        "draws": DRAWS,
        "first": share(rank == 1),
        "top_two": share(rank <= 2),
        "last": share(rank == len(languages)),
        "bottom_two": share(rank >= len(languages) - 1),
    }

    Path(out, "data").mkdir(parents=True, exist_ok=True)
    Path(out, "data", "weights.json").write_text(json.dumps({"shifted": shifted, "drawn": drawn}, indent=1) + "\n")


if __name__ == "__main__":
    main(*sys.argv[1:3])
