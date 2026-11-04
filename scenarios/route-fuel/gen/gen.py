"""Writes the cases (examples/ and hidden/) or the workloads/ for route-fuel.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
"""

import hashlib
import random
import subprocess
import sys
from dataclasses import dataclass
from math import isqrt
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
MAX_PLACES, MAX_CAPACITY, MAX_ROADS, MAX_TRIPS, MAX_VALUE = 50_000, 100, 300_000, 100_000, 1_000_000

Road = tuple[int, int, int, int]
Trip = tuple[int, int, int]


@dataclass
class Case:
    n: int
    capacity: int
    stations: dict[int, int]
    roads: list[Road]
    trips: list[Trip]

    def render(self) -> str:
        lines = [f"{self.n} {self.capacity} {len(self.stations)} {len(self.roads)} {len(self.trips)}"]
        lines += [f"{v} {p}" for v, p in self.stations.items()]
        lines += [f"{u} {v} {a} {b}" for u, v, a, b in self.roads]
        lines += [f"{s} {f} {t}" for s, f, t in self.trips]
        return "\n".join(lines) + "\n"

    def check(self):
        """Fails on an input the spec does not allow, so no case tests something it never promised."""
        n, c = self.n, self.capacity
        assert 1 <= n <= MAX_PLACES and 1 <= c <= MAX_CAPACITY
        assert len(self.roads) <= MAX_ROADS and len(self.trips) <= MAX_TRIPS
        assert all(1 <= v <= n and 0 <= p <= MAX_VALUE for v, p in self.stations.items())
        assert all(1 <= u <= n and 1 <= v <= n and 0 <= a <= MAX_VALUE and 0 <= b <= MAX_VALUE for u, v, a, b in self.roads)
        assert all(1 <= s <= n and 0 <= f <= c and 1 <= t <= n for s, f, t in self.trips)


def write(directory: str, name: str, case: Case, expect: bool = True):
    case.check()
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_text(case.render())
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def chain(places: list[int], fuel: int, time: int) -> list[Road]:
    return [(u, v, fuel, time) for u, v in zip(places, places[1:])]


def every_fuel(capacity: int, start: int, target: int) -> list[Trip]:
    return [(start, f, target) for f in range(capacity + 1)]


def edge_cases():
    yield "001-single-place", Case(1, 1, {}, [], [(1, 0, 1), (1, 1, 1)])
    yield "002-no-trips", Case(3, 5, {1: 4}, [(1, 2, 1, 1), (2, 3, 1, 1)], [])
    yield "003-start-is-target", Case(3, 5, {2: 7}, [(1, 2, 2, 3), (2, 1, 2, 3), (3, 3, 0, 0)], [
        (1, 0, 1), (1, 5, 1), (2, 0, 2), (3, 2, 3),
    ])
    yield "004-no-roads", Case(3, 4, {1: 2, 2: 2, 3: 2}, [], [(1, 4, 2), (2, 0, 3), (3, 0, 3)])
    yield "005-one-way", Case(2, 5, {1: 1, 2: 1}, [(1, 2, 3, 7)], [(1, 0, 2), (2, 5, 1), (1, 3, 2)])
    yield "006-exact-empty", Case(3, 10, {}, [(1, 2, 4, 5), (2, 3, 6, 5)], [(1, 10, 3), (1, 9, 3), (1, 4, 2), (1, 3, 2)])
    yield "007-zero-fuel-roads", Case(4, 3, {}, [(1, 2, 0, 4), (2, 3, 0, 6), (3, 4, 1, 1), (3, 1, 0, 0)], [
        (1, 0, 3), (1, 0, 4), (1, 1, 4), (4, 0, 1), (2, 0, 1),
    ])
    yield "008-road-needs-capacity", Case(3, 7, {1: 3}, [(1, 2, 7, 2), (2, 3, 0, 1)], [(1, 0, 2), (1, 7, 3), (1, 3, 3)])
    yield "009-road-over-capacity", Case(4, 7, {1: 3, 3: 1}, [(1, 4, 8, 1), (1, 3, 4, 5), (3, 4, 4, 5)], [
        (1, 0, 4), (1, 7, 4), (1, 7, 3),
    ])
    yield "010-capacity-one", Case(5, 1, {1: 2, 2: 3, 3: 1, 4: 5}, [*chain([1, 2, 3, 4, 5], 1, 1), (1, 3, 0, 10)], [
        (1, 0, 5), (1, 1, 5), (2, 0, 5), (4, 1, 5), (5, 1, 1),
    ])
    yield "011-buy-at-start", Case(3, 10, {1: 5}, [(1, 2, 6, 3), (2, 3, 4, 3)], [(1, 0, 3), (1, 4, 3), (1, 10, 3), (1, 0, 2)])
    yield "012-station-only-at-target", Case(3, 10, {3: 1}, [(1, 2, 1, 1), (2, 3, 1, 1), (3, 1, 1, 1)], [
        (1, 0, 3), (1, 1, 3), (1, 2, 3), (3, 0, 2),
    ])
    yield "013-free-station-at-target", Case(3, 10, {1: 4, 3: 0}, [(1, 2, 3, 1), (2, 3, 3, 1)], [(1, 0, 3), (1, 6, 3)])
    yield "014-cheaper-station-ahead", Case(3, 20, {1: 10, 2: 1}, [(1, 2, 5, 1), (2, 3, 15, 1)], [
        (1, 0, 3), (1, 5, 3), (1, 20, 3), (1, 3, 3),
    ])
    yield "015-dearer-station-ahead", Case(3, 20, {1: 1, 2: 10}, [(1, 2, 5, 1), (2, 3, 15, 1)], [(1, 0, 3), (1, 12, 3)])
    yield "016-neither-fill-nor-minimum", Case(4, 10, {1: 5, 2: 3, 3: 10}, [(1, 2, 4, 1), (2, 3, 6, 1), (3, 4, 8, 1)], [
        (1, 0, 4), (2, 0, 4), (1, 10, 4),
    ])
    yield "017-detour-to-cheap-station", Case(3, 10, {1: 10, 2: 1}, [(1, 2, 2, 5), (2, 1, 2, 5), (1, 3, 8, 1)], [
        (1, 2, 3), (1, 0, 3), (1, 1, 3), (1, 10, 3),
    ])
    yield "018-loop-to-cheap-station", Case(5, 8, {1: 9, 3: 2}, [
        (1, 2, 1, 4), (2, 3, 1, 4), (3, 4, 1, 4), (4, 1, 1, 4), (1, 5, 8, 2), (2, 5, 9, 1),
    ], [(1, 3, 5), (1, 0, 5), (2, 0, 5), (4, 1, 5)])
    yield "019-time-breaks-cost-tie", Case(4, 10, {1: 2}, [(1, 2, 3, 10), (2, 4, 3, 10), (1, 3, 3, 7), (3, 4, 3, 9)], [
        (1, 0, 4), (1, 6, 4), (1, 3, 4),
    ])
    yield "020-cost-beats-time", Case(3, 10, {1: 2}, [(1, 3, 9, 1), (1, 2, 2, 100), (2, 3, 2, 100)], [
        (1, 0, 3), (1, 9, 3), (1, 4, 3), (1, 8, 3),
    ])
    yield "021-free-fuel-fast-road", Case(3, 10, {1: 0}, [(1, 3, 9, 1), (1, 2, 2, 100), (2, 3, 2, 100)], [(1, 0, 3), (2, 0, 3)])
    yield "022-save-start-fuel", Case(3, 10, {2: 3}, [(1, 2, 8, 1), (1, 2, 2, 50), (2, 3, 6, 1)], [
        (1, 10, 3), (1, 10, 2), (1, 8, 3), (1, 2, 3),
    ])
    yield "023-parallel-roads", Case(2, 5, {1: 7}, [(1, 2, 5, 1), (1, 2, 3, 4), (1, 2, 1, 9), (1, 2, 0, 20)], [
        (1, 0, 2), (1, 3, 2), (1, 5, 2), (1, 1, 2),
    ])
    yield "024-self-loops", Case(2, 5, {1: 1}, [(1, 1, 1, 1), (1, 1, 0, 0), (2, 2, 0, 0), (1, 2, 2, 3)], [
        (1, 0, 2), (2, 0, 2), (1, 2, 2), (2, 5, 1),
    ])
    yield "025-zero-time-roads", Case(4, 6, {1: 1, 2: 1}, [*chain([1, 2, 3, 4], 2, 0), (1, 4, 6, 0)], [
        (1, 0, 4), (2, 2, 4), (1, 6, 4),
    ])
    yield "026-zero-weight-cycle", Case(4, 3, {2: 0}, [(1, 2, 0, 0), (2, 3, 0, 0), (3, 1, 0, 0), (3, 4, 3, 5)], [
        (1, 0, 4), (4, 0, 1), (3, 0, 4),
    ])
    yield "027-separate-islands", Case(6, 5, {1: 1, 4: 1}, [*chain([1, 2, 3, 1], 1, 1), *chain([4, 5, 6, 4], 1, 1)], [
        (1, 5, 6), (4, 0, 6), (6, 5, 3), (2, 5, 1),
    ])
    yield "028-gap-over-capacity", Case(4, 5, {1: 1, 3: 1}, [(1, 3, 5, 1), (3, 2, 3, 1), (2, 4, 3, 1)], [
        (1, 0, 4), (1, 5, 2), (3, 5, 4),
    ])
    yield "029-arrive-empty-at-station", Case(3, 10, {2: 1}, [(1, 2, 5, 1), (2, 3, 10, 1)], [(1, 5, 3), (1, 4, 3), (1, 5, 2)])
    yield "030-cheaper-arrival-with-less-fuel", Case(4, 10, {1: 3}, [(1, 2, 2, 1), (2, 3, 8, 1), (1, 4, 1, 1), (4, 2, 0, 0)], [
        (1, 0, 3), (1, 0, 2), (1, 2, 3),
    ])
    yield "031-dearer-arrival-with-more-fuel", Case(5, 10, {1: 1, 2: 5}, [(1, 3, 1, 1), (2, 3, 1, 1), (1, 2, 1, 1), (3, 4, 6, 1), (4, 5, 4, 1)], [
        (1, 0, 5), (2, 0, 5), (1, 0, 4),
    ])
    yield "032-free-stations-still-bounded", Case(4, 5, {1: 0, 2: 0, 3: 0}, [(1, 2, 5, 1), (2, 3, 5, 1), (3, 4, 6, 1), (2, 4, 6, 1)], [
        (1, 0, 3), (1, 0, 4), (4, 5, 1),
    ])
    yield "033-only-unusable-roads", Case(3, 100, {1: 1, 2: 1}, [(1, 2, 101, 1), (2, 3, MAX_VALUE, 0), (1, 3, 1000, 5)], [
        (1, 100, 2), (1, 0, 3), (2, 100, 3), (3, 0, 3),
    ])
    yield "034-same-price-everywhere", Case(9, 4, {v: 3 for v in range(1, 10)}, [
        *grid_roads(3, 3, lambda u, v: (1 + (u + v) % 2, 2 + (u * v) % 5)),
    ], [(1, 0, 9), (9, 0, 1), (3, 2, 7), (5, 4, 1)])
    yield "035-every-start-fuel", Case(6, 12, {1: 6, 3: 2, 5: 4}, [
        (1, 2, 3, 2), (2, 3, 2, 2), (1, 3, 6, 1), (3, 4, 7, 3), (4, 6, 4, 2), (3, 5, 2, 9), (5, 6, 8, 1), (2, 1, 3, 2),
    ], every_fuel(12, 1, 6))
    yield "036-one-start-every-target", Case(6, 12, {1: 6, 3: 2, 5: 4}, [
        (1, 2, 3, 2), (2, 3, 2, 2), (1, 3, 6, 1), (3, 4, 7, 3), (4, 6, 4, 2), (3, 5, 2, 9), (5, 6, 8, 1), (2, 1, 3, 2),
    ], [(1, 0, t) for t in range(1, 7)] + [(4, 0, t) for t in range(1, 7)])
    yield "037-last-place", Case(7, 3, {7: 2, 1: 5}, [(7, 1, 3, 4), (1, 7, 3, 4), (7, 7, 1, 1), (6, 7, 0, 1)], [
        (7, 0, 1), (1, 0, 7), (6, 0, 7), (7, 3, 7), (1, 3, 6),
    ])
    yield "038-max-values", Case(4, MAX_CAPACITY, {1: MAX_VALUE, 2: MAX_VALUE, 3: 0}, [
        (1, 2, MAX_CAPACITY, MAX_VALUE), (2, 3, MAX_CAPACITY, MAX_VALUE), (1, 3, MAX_VALUE, 0), (3, 4, MAX_CAPACITY, MAX_VALUE),
        (2, 4, MAX_CAPACITY + 1, 0),
    ], [(1, 0, 4), (1, MAX_CAPACITY, 4), (2, 0, 4), (1, 0, 3)])
    yield "039-cost-over-32-bits", Case(60, MAX_CAPACITY, {v: MAX_VALUE for v in range(1, 61)}, chain(list(range(1, 61)), MAX_CAPACITY, 1), [
        (1, 0, 60), (1, MAX_CAPACITY, 60), (1, 0, 22), (1, 0, 44), (60, 0, 1), (1, 0, 1),
    ])
    yield "040-time-over-32-bits", Case(5000, 1, {1: 1}, [*chain(list(range(1, 5001)), 0, MAX_VALUE), (1, 5000, 1, 0)], [
        (1, 0, 5000), (1, 1, 5000), (2, 0, 5000), (1, 0, 2148), (1, 0, 4296), (5000, 1, 1),
    ])
    yield "041-max-places", Case(MAX_PLACES, 10, {1: 3, MAX_PLACES: 1}, [
        (1, MAX_PLACES, 10, 7), (MAX_PLACES, 25_000, 4, 5), (25_000, 1, 6, 9), (MAX_PLACES, MAX_PLACES - 1, 0, 1),
    ], [(1, 0, MAX_PLACES), (MAX_PLACES, 0, 1), (MAX_PLACES - 1, 0, 1), (25_000, 10, MAX_PLACES), (1, 0, 2)])
    yield "042-long-chain", Case(MAX_PLACES, 1, {}, chain(list(range(1, MAX_PLACES + 1)), 0, 3), [
        (1, 0, MAX_PLACES), (MAX_PLACES, 1, 1), (17, 1, 40_000),
    ])


def grid_roads(rows: int, cols: int, weight) -> list[Road]:
    """Roads both ways between orthogonal neighbours of a rows x cols grid, numbered row by row."""
    roads = []
    for r in range(rows):
        for c in range(cols):
            u = r * cols + c + 1
            for v in ([u + 1] if c + 1 < cols else []) + ([u + cols] if r + 1 < rows else []):
                roads.append((u, v, *weight(u, v)))
                roads.append((v, u, *weight(v, u)))
    return roads


def random_stations(rng: random.Random, n: int, rate: float, prices: tuple[int, int]) -> dict[int, int]:
    return {v: rng.randint(*prices) for v in range(1, n + 1) if rng.random() < rate}


def random_trips(rng: random.Random, n: int, capacity: int, count: int) -> list[Trip]:
    return [(rng.randint(1, n), rng.randint(0, capacity), rng.randint(1, n)) for _ in range(count)]


def walk_trips(rng: random.Random, n: int, capacity: int, roads: list[Road], count: int) -> list[Trip]:
    """Trips whose target is, half the time, the end of a short random walk along the roads from
    the start, so that not nearly every target of a sparse graph is out of reach."""
    out = [[] for _ in range(n + 1)]
    for u, v, _, _ in roads:
        out[u].append(v)
    trips = []
    for _ in range(count):
        start = target = rng.randint(1, n)
        if rng.random() < 0.5:
            for _ in range(rng.randint(1, 8)):
                if out[target]:
                    target = rng.choice(out[target])
        else:
            target = rng.randint(1, n)
        trips.append((start, rng.randint(0, capacity), target))
    return trips


def random_case(rng: random.Random, n: int, capacity: int, roads: int, trips: int, rate: float, fuels, times, prices) -> Case:
    """A random road graph; fuels, times and prices are pools to draw from, so small pools force ties."""
    stations = {v: rng.choice(prices) for v in range(1, n + 1) if rng.random() < rate}
    edges = [(rng.randint(1, n), rng.randint(1, n), rng.choice(fuels), rng.choice(times)) for _ in range(roads)]
    return Case(n, capacity, stations, edges, walk_trips(rng, n, capacity, edges, trips))


def random_cases(rng: random.Random):
    number = 43
    for _ in range(12):
        n, c = rng.randint(1, 6), rng.randint(1, 6)
        case = random_case(rng, n, c, rng.randint(2, 14), rng.randint(2, 8), 0.5,
                           (0, 1, 1, 2, 3, c, c + 1), (0, 1, 2, 5, 10), (0, 1, 2, 3, 5, 9))
        yield f"{number:03d}-random-tiny", case
        number += 1
    for _ in range(10):
        n, c = rng.randint(5, 12), rng.randint(2, 10)
        case = random_case(rng, n, c, rng.randint(2 * n, 4 * n), rng.randint(8, 15), 0.5,
                           tuple(range(0, c // 2 + 2)), tuple(range(0, 20)), tuple(range(0, 10)))
        yield f"{number:03d}-random-small", case
        number += 1
    for _ in range(6):
        n, c = rng.randint(5, 10), rng.randint(2, 6)
        case = random_case(rng, n, c, rng.randint(2 * n, 4 * n), 12, 0.5, (0, 1, 2), (0, 1, 2), (0, 1, 2))
        yield f"{number:03d}-random-ties", case
        number += 1
    for _ in range(8):
        n, c = rng.randint(100, 600), rng.randint(5, 60)
        case = random_case(rng, n, c, rng.randint(2 * n, 5 * n), rng.randint(50, 150), rng.choice((0.05, 0.2, 0.5)),
                           tuple(range(0, c // 2 + 1)), tuple(range(0, 1000)), tuple(range(1, 200)))
        yield f"{number:03d}-random-medium", case
        number += 1
    for _ in range(4):
        n, c = rng.randint(100, 400), rng.randint(5, 30)
        case = random_case(rng, n, c, rng.randint(2 * n, 3 * n), 100, 0.25,
                           tuple(range(0, c + 5)), tuple(range(0, 50)), tuple(range(0, 50)))
        yield f"{number:03d}-random-sparse", case
        number += 1
    for _ in range(6):
        rows, cols = rng.randint(10, 30), rng.randint(10, 30)
        yield f"{number:03d}-random-city", city(rng, rows, cols, rng.randint(10, 60), 0.08, rng.randint(50, 150), 8)
        number += 1
    for _ in range(4):
        yield f"{number:03d}-random-highway", highway(rng, rng.randint(200, 500), MAX_CAPACITY, 0.25, rng.randint(10, 20))
        number += 1


def large_cases(rng: random.Random):
    yield "093-large-city", city(rng, 60, 60, 50, 0.06, 300, 15)
    yield "094-large-highway", highway(rng, 1200, MAX_CAPACITY, 0.2, 12)
    yield "095-large-maze", maze(rng, 1500, 20, 60)
    yield "096-many-trips", random_case(rng, 30, 5, 120, 10_000, 0.3, (0, 1, 2, 3), tuple(range(0, 10)), tuple(range(0, 10)))
    yield "097-many-roads", random_case(rng, 200, 40, 20_000, 100, 0.2, tuple(range(0, 30)), tuple(range(0, 10**4)), tuple(range(1, 100)))


def city(rng: random.Random, rows: int, cols: int, capacity: int, rate: float, trips: int, radius: int) -> Case:
    """A street grid with fast, thirsty avenues every eighth row and column, a few one-way
    streets and stations at a share of corners; trips go between nearby corners."""
    n = rows * cols
    roads = []
    for r in range(rows):
        for c in range(cols):
            u = r * cols + c + 1
            for v, avenue in ((u + 1, r % 8 == 0), (u + cols, c % 8 == 0)):
                if v == u + 1 and c + 1 == cols or v == u + cols and r + 1 == rows:
                    continue
                fuel, time = (rng.randint(2, 4), rng.randint(3, 6)) if avenue else (rng.randint(1, 3), rng.randint(10, 30))
                kind = rng.random()
                if kind < 0.95:
                    roads.append((u, v, fuel, time))
                if kind < 0.9 or kind >= 0.95:
                    roads.append((v, u, fuel, time))
    stations = random_stations(rng, n, rate, (50, 150))
    result = []
    for _ in range(trips):
        r, c = rng.randrange(rows), rng.randrange(cols)
        r2 = min(rows - 1, max(0, r + rng.randint(-radius, radius)))
        c2 = min(cols - 1, max(0, c + rng.randint(-radius, radius)))
        result.append((r * cols + c + 1, rng.randint(0, capacity), r2 * cols + c2 + 1))
    return Case(n, capacity, stations, roads, result)


def highway(rng: random.Random, n: int, capacity: int, rate: float, trips: int) -> Case:
    """Towns scattered on a plane, each joined both ways to its three nearest towns by roads whose
    fuel grows with length; fuel prices drift across the map. Trips cross the whole map."""
    side = 1_000_000
    towns = [(rng.randrange(side), rng.randrange(side)) for _ in range(n)]
    cell = side // max(1, isqrt(n // 4))
    buckets: dict[tuple[int, int], list[int]] = {}
    for i, (x, y) in enumerate(towns):
        buckets.setdefault((x // cell, y // cell), []).append(i)
    pairs = set()
    for i, (x, y) in enumerate(towns):
        near, reach = [], 1
        while len(near) < 4:
            near = [j for dx in range(-reach, reach + 1) for dy in range(-reach, reach + 1)
                    for j in buckets.get((x // cell + dx, y // cell + dy), ())]
            reach += 1
        near.sort(key=lambda j: ((towns[j][0] - x) ** 2 + (towns[j][1] - y) ** 2, j))
        for j in near[1:4]:
            pairs.add((min(i, j), max(i, j)))
    roads = []
    for i, j in sorted(pairs):
        length = isqrt((towns[i][0] - towns[j][0]) ** 2 + (towns[i][1] - towns[j][1]) ** 2)
        fuel = max(1, length * 12 // cell)
        for u, v in ((i, j), (j, i)):
            roads.append((u + 1, v + 1, fuel, fuel * rng.randint(8, 12)))
    rng.shuffle(roads)
    stations = {}
    for i, (x, y) in enumerate(towns):
        if rng.random() < rate:
            stations[i + 1] = 100 + 50 * (x + 2 * y) // (3 * side) + rng.randint(0, 40)
    return Case(n, capacity, stations, roads, random_trips(rng, n, capacity, trips))


def maze(rng: random.Random, n: int, capacity: int, trips: int) -> Case:
    """One-way roads between random places, with free roads, instant roads and free stations
    common, so many trips tie on cost and many targets are out of reach."""
    roads = []
    for u in range(1, n + 1):
        for _ in range(rng.choice((0, 1, 2, 2, 3, 3, 4, 5))):
            v = rng.randint(1, n)
            roads.append((u, v, rng.choice((0, 0, 1, 2, 3, 5, 8, capacity, capacity + 1)), rng.choice((0, 0, 1, 10, 100, 1000))))
    rng.shuffle(roads)
    stations = random_stations(rng, n, 0.15, (0, 3))
    return Case(n, capacity, stations, roads, walk_trips(rng, n, capacity, roads, trips))


def cases():
    write("examples", "basic", Case(4, 10, {1: 5, 3: 2}, [
        (1, 2, 4, 10), (2, 3, 3, 10), (1, 3, 8, 12), (3, 4, 6, 5), (4, 1, 0, 1),
    ], [(1, 0, 4), (1, 10, 4), (1, 7, 3), (4, 0, 3), (2, 0, 4), (2, 0, 2)]))
    write("examples", "detour", Case(4, 6, {1: 4, 2: 1}, [
        (1, 2, 2, 3), (2, 1, 2, 3), (1, 4, 6, 1), (1, 3, 3, 5), (3, 4, 3, 5),
    ], [(1, 6, 4), (1, 2, 4), (1, 0, 4), (3, 0, 4)]))
    write("examples", "random", random_case(random.Random(5), 8, 6, 20, 10, 0.4, tuple(range(0, 5)), tuple(range(0, 10)), tuple(range(0, 6))))

    rng = random.Random(20261104)
    for name, case in [*edge_cases(), *random_cases(rng), *large_cases(rng)]:
        write("hidden", name, case)


def workloads():
    rng = random.Random(20261105)
    write("workloads", "city", city(rng, 200, 200, 50, 0.06, 240, 20), expect=False)
    write("workloads", "highway", highway(rng, 8_000, MAX_CAPACITY, 0.2, 5), expect=False)


def write_digests():
    """Records the workloads' SHA-256 so a regeneration is checked byte for byte."""
    lines = []
    for path in sorted((ROOT / "workloads").glob("*.in")):
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    (ROOT / "workloads.sha256").write_text("".join(lines))


def main():
    commands = {"cases": cases, "workloads": lambda: (workloads(), write_digests())}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
