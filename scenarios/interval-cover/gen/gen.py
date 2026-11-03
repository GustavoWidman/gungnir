"""Writes the cases (examples/ and hidden/) or the workloads/ for interval-cover.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
"""

import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
MIN, MAX = -(2**63), 2**63 - 1

Interval = tuple[int, int]


def render(intervals: list[Interval], queries: list[int]) -> str:
    lines = [f"{len(intervals)} {len(queries)}"]
    lines += [f"{l} {r}" for l, r in intervals]
    lines += map(str, queries)
    return "\n".join(lines) + "\n"


def write(directory: str, name: str, intervals: list[Interval], queries: list[int], expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_text(render(intervals, queries))
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def clamp(v: int) -> int:
    return max(MIN, min(MAX, v))


def random_intervals(rng: random.Random, n: int, lo: int, hi: int, max_len: int | None = None) -> list[Interval]:
    out = []
    for _ in range(n):
        if max_len is None:
            a, b = rng.randint(lo, hi), rng.randint(lo, hi)
            out.append((min(a, b), max(a, b)))
        else:
            l = rng.randint(lo, hi)
            out.append((l, min(hi, l + rng.randint(0, max_len))))
    return out


def probe_queries(rng: random.Random, intervals: list[Interval], q: int, lo: int, hi: int) -> list[int]:
    """Mixes uniform points with endpoints and their neighbours, where off-by-one bugs show."""
    out = []
    for _ in range(q):
        if intervals and rng.random() < 0.5:
            l, r = rng.choice(intervals)
            out.append(clamp(rng.choice((l, r)) + rng.choice((-1, 0, 0, 1))))
        else:
            out.append(rng.randint(lo, hi))
    return out


def edge_cases():
    yield "001-empty", [], []
    yield "002-no-intervals", [], [0, -5, 5, MIN, MAX]
    yield "003-no-queries", [(1, 2), (3, 4), (MIN, MAX)], []
    yield "004-single-point-interval", [(7, 7)], [6, 7, 8]
    yield "005-touching-endpoints", [(1, 3), (3, 5), (5, 7)], list(range(0, 9))
    yield "006-nested", [(-10, 10), (-5, 5), (-1, 1), (0, 0)], list(range(-11, 12))
    yield "007-duplicate-intervals", [(-3, 4)] * 1000, [-4, -3, 0, 4, 5]
    yield "008-full-range-interval", [(MIN, MAX)], [MIN, MIN + 1, -1, 0, 1, MAX - 1, MAX]
    yield "009-extreme-points", [(MIN, MIN), (MAX, MAX)], [MIN, MIN + 1, 0, MAX - 1, MAX]
    yield "010-halves-meet-at-zero", [(MIN, 0), (0, MAX)], [MIN, -1, 0, 1, MAX]
    yield "011-all-negative", [(-100, -50), (-75, -25), (-60, -60)], [-101, -100, -75, -60, -50, -49, -25, -24, 0]
    yield "012-queries-outside", [(10, 20), (15, 30)], [MIN, -1, 9, 31, 1000, MAX]
    yield "013-repeated-query", [(0, 5), (2, 8)], [3] * 50
    yield "014-one-and-one", [(42, 43)], [43]
    yield "015-adjacent-chain", [(k, k + 1) for k in range(0, 200, 2)], list(range(-1, 202))
    yield "016-shared-start", [(0, k) for k in range(100)], list(range(-1, 101))
    yield "017-shared-end", [(-k, 0) for k in range(100)], list(range(-100, 2))
    yield "018-extreme-spans", [(MIN, MIN + 1), (MAX - 1, MAX), (MIN, MAX), (MIN + 1, MAX - 1)], [
        MIN, MIN + 1, MIN + 2, 0, MAX - 2, MAX - 1, MAX,
    ]
    yield "019-unsorted-queries", [(1, 10), (5, 15), (20, 25)], [25, 1, 15, 0, 20, 10, 5, 16, 26, 19]
    yield "020-points-only", [(k % 7, k % 7) for k in range(70)], list(range(-1, 9))


def random_cases(rng: random.Random):
    for i in range(10):
        n, q = rng.randint(1, 20), rng.randint(1, 20)
        iv = random_intervals(rng, n, -10, 10)
        yield f"{21 + i:03d}-random-tiny", iv, probe_queries(rng, iv, q, -12, 12)
    for i in range(8):
        n, q = rng.randint(500, 2000), rng.randint(500, 2000)
        iv = random_intervals(rng, n, -10**6, 10**6, max_len=rng.choice((10, 1000, 10**5)))
        yield f"{31 + i:03d}-random-medium", iv, probe_queries(rng, iv, q, -10**6 - 10, 10**6 + 10)
    for i in range(5):
        n, q = rng.randint(500, 2000), rng.randint(500, 2000)
        iv = random_intervals(rng, n, MIN, MAX)
        yield f"{39 + i:03d}-random-full-range", iv, probe_queries(rng, iv, q, MIN, MAX)
    for i in range(3):
        edge = MIN if i % 2 == 0 else MAX - 1000
        iv = random_intervals(rng, 1000, edge, edge + 1000, max_len=50)
        yield f"{44 + i:03d}-random-near-{'min' if edge == MIN else 'max'}", iv, probe_queries(rng, iv, 1000, edge, edge + 1000)


def large_cases(rng: random.Random):
    iv = random_intervals(rng, 20_000, -10**9, 10**9, max_len=10**6)
    yield "047-large-uniform", iv, probe_queries(rng, iv, 20_000, -10**9, 10**9)
    iv = random_intervals(rng, 10_000, MIN, MAX)
    yield "048-large-full-range", iv, probe_queries(rng, iv, 10_000, MIN, MAX)
    iv = [(rng.randint(-10**6, 0), rng.randint(0, 10**6)) for _ in range(25_000)]
    yield "049-large-all-overlapping", iv, probe_queries(rng, iv, 25_000, -10**6, 10**6)
    iv = [(p, p) for p in (rng.randint(-1000, 1000) for _ in range(25_000))]
    yield "050-large-duplicate-points", iv, [rng.randint(-1001, 1001) for _ in range(25_000)]


def cases():
    write("examples", "touching", [(1, 5), (3, 8), (5, 5)], [0, 3, 5, 9])
    write("examples", "full-range", [(MIN, 0), (0, MAX)], [MIN, 0, MAX])
    write("examples", "empty", [], [1, 2])

    rng = random.Random(20261103)
    for case in [*edge_cases(), *random_cases(rng), *large_cases(rng)]:
        write("hidden", *case)


def workloads():
    rng = random.Random(20261104)
    iv = random_intervals(rng, 700_000, -10**9, 10**9, max_len=10**6)
    write("workloads", "uniform", iv, probe_queries(rng, iv, 700_000, -10**9, 10**9), expect=False)
    iv = random_intervals(rng, 500_000, MIN, MAX)
    write("workloads", "full-range", iv, probe_queries(rng, iv, 500_000, MIN, MAX), expect=False)


def main():
    commands = {"cases": cases, "workloads": workloads}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
