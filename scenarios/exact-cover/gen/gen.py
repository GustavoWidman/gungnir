"""Writes the cases (examples/ and hidden/) or the workloads/ for exact-cover.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are regenerated on demand and never committed.
"""

import hashlib
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
MAX_RANDOM_ANSWER = 5_000

Piece = list[str]

MONOMINO = ["#"]
DOMINO = ["##"]
TROMINOES = {"I3": ["###"], "L3": ["##", "#."]}
TETROMINOES = {
    "I4": ["####"],
    "O4": ["##", "##"],
    "T4": ["###", ".#."],
    "L4": ["###", "#.."],
    "S4": [".##", "##."],
}
PENTOMINOES = {
    "F": [".##", "##.", ".#."],
    "I": ["#####"],
    "L": ["####", "#..."],
    "N": ["##..", ".###"],
    "P": ["##", "##", "#."],
    "T": ["###", ".#.", ".#."],
    "U": ["#.#", "###"],
    "V": ["#..", "#..", "###"],
    "W": ["#..", "##.", ".##"],
    "X": [".#.", "###", ".#."],
    "Y": [".#..", "####"],
    "Z": ["##.", ".#.", ".##"],
}


def render(board: list[str], pieces: list[Piece]) -> str:
    lines = [f"{len(board)} {len(board[0])}", *board, str(len(pieces))]
    for piece in pieces:
        lines += [f"{len(piece)} {len(piece[0])}", *piece]
    return "\n".join(lines) + "\n"


def answer(board: list[str], pieces: list[Piece]) -> bytes:
    out = subprocess.run([sys.executable, REFERENCE], input=render(board, pieces).encode(), capture_output=True, check=True)
    return out.stdout


def write(directory: str, name: str, board: list[str], pieces: list[Piece], expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_text(render(board, pieces))
    if expect:
        out = answer(board, pieces)
        assert int(out) < 2**63, f"{name}: the answer {int(out)} breaks the spec's limit"
        path.with_suffix(".out").write_bytes(out)


def rect(h: int, w: int, blocked: set[tuple[int, int]] = frozenset()) -> list[str]:
    return ["".join("#" if (r, c) in blocked else "." for c in range(w)) for r in range(h)]


def cells(piece: Piece) -> list[tuple[int, int]]:
    return [(r, c) for r, row in enumerate(piece) for c, ch in enumerate(row) if ch == "#"]


def draw(points, margin: tuple[int, int, int, int] = (0, 0, 0, 0)) -> Piece:
    """Draws cells in their bounding box, padded by (top, bottom, left, right) empty lines."""
    top, bottom, left, right = margin
    r0 = min(r for r, _ in points) - top
    c0 = min(c for _, c in points) - left
    h = max(r for r, _ in points) - r0 + 1 + bottom
    w = max(c for _, c in points) - c0 + 1 + right
    grid = [["."] * w for _ in range(h)]
    for r, c in points:
        grid[r - r0][c - c0] = "#"
    return ["".join(row) for row in grid]


def turned(piece: Piece, turns: int, mirror: bool) -> Piece:
    points = cells(piece)
    if mirror:
        points = [(r, -c) for r, c in points]
    for _ in range(turns):
        points = [(c, -r) for r, c in points]
    return draw(points)


def disguise(rng: random.Random, piece: Piece, margins: bool = False) -> Piece:
    """The same free piece drawn in a random orientation, sometimes with empty margins."""
    points = cells(turned(piece, rng.randrange(4), rng.random() < 0.5))
    pad = tuple(rng.choice((0, 0, 1)) for _ in range(4)) if margins else (0, 0, 0, 0)
    out = draw(points, pad)
    if max(len(out), len(out[0])) > 8:
        out = draw(points)
    return out


def carve(rng: random.Random, board: list[str], sizes: tuple[int, int]) -> list[Piece] | None:
    """Splits the free cells of board into random connected pieces, or None when a cell is left over."""
    free = {(r, c) for r, row in enumerate(board) for c, ch in enumerate(row) if ch == "."}
    order = sorted(free)
    rng.shuffle(order)
    taken: set[tuple[int, int]] = set()
    pieces = []
    for start in order:
        if start in taken:
            continue
        target = rng.randint(*sizes)
        piece = [start]
        taken.add(start)
        while len(piece) < target:
            frontier = [
                (r + dr, c + dc)
                for r, c in piece
                for dr, dc in ((1, 0), (-1, 0), (0, 1), (0, -1))
                if (r + dr, c + dc) in free and (r + dr, c + dc) not in taken
            ]
            if not frontier:
                break
            cell = rng.choice(frontier)
            taken.add(cell)
            piece.append(cell)
        if max(max(r for r, _ in piece) - min(r for r, _ in piece), max(c for _, c in piece) - min(c for _, c in piece)) >= 8:
            return None
        pieces.append(draw(piece))
    return pieces


def random_board(rng: random.Random, h: int, w: int, blocked: float) -> list[str]:
    return ["".join("#" if rng.random() < blocked else "." for _ in range(w)) for _ in range(h)]


def carved_case(rng: random.Random, h: int, w: int, blocked: float, sizes: tuple[int, int], margins: bool = False):
    """A random board and the pieces carved from it, redrawn until the answer is at most
    MAX_RANDOM_ANSWER, which keeps every case quick for a search that finds tilings one by one."""
    while True:
        board = random_board(rng, h, w, blocked)
        pieces = carve(rng, board, sizes)
        if pieces is not None and any("." in row for row in board):
            pieces = [disguise(rng, p, margins) for p in pieces]
            rng.shuffle(pieces)
            if int(answer(board, pieces)) <= MAX_RANDOM_ANSWER:
                return board, pieces


def edge_cases():
    yield "001-one-free-cell-one-monomino", ["."], [MONOMINO]
    yield "002-no-free-cells-no-pieces", ["#"], []
    yield "003-free-cell-no-pieces", ["."], []
    yield "004-no-free-cells-one-piece", ["##", "##"], [MONOMINO]
    yield "005-all-blocked-large-board", ["#" * 20] * 20, []
    yield "006-piece-area-too-small", rect(2, 3), [TROMINOES["I3"], MONOMINO, MONOMINO]
    yield "007-piece-area-too-large", rect(2, 3), [TROMINOES["I3"], TROMINOES["I3"], MONOMINO]
    yield "008-pieces-fit-but-leave-a-gap", rect(3, 3), [TETROMINOES["O4"], TETROMINOES["L4"]]
    yield "009-extra-piece-left-over", rect(1, 4), [TROMINOES["I3"], MONOMINO, DOMINO]
    yield "010-area-matches-no-tiling", rect(2, 2), [TROMINOES["I3"], MONOMINO]
    yield "011-labeled-copies-of-l-tromino", rect(2, 3), [["##", "#."], ["#.", "##"]]
    yield "012-symmetric-straight-tromino", rect(1, 4), [TROMINOES["I3"], MONOMINO]
    yield "013-square-on-square", rect(2, 2), [TETROMINOES["O4"]]
    yield "014-x-on-x-region-inside-blocks", ["#####", "##.##", "#...#", "##.##", "#####"], [PENTOMINOES["X"]]
    yield "015-x-pentomino-on-3x3", rect(3, 3, {(0, 0), (0, 2), (2, 0), (2, 2)}), [PENTOMINOES["X"]]
    yield "016-monominoes-only", rect(2, 3), [MONOMINO] * 6
    yield "017-twenty-monominoes", rect(4, 5), [MONOMINO] * 20
    yield "018-dominoes-on-2x4", rect(2, 4), [DOMINO] * 4
    yield "019-dominoes-on-ring", rect(3, 3, {(1, 1)}), [DOMINO] * 4
    yield "020-needs-rotation", rect(4, 1), [TETROMINOES["I4"]]
    yield "021-needs-reflection", ["..#", "#..", "#.#"], [PENTOMINOES["F"]]
    yield "022-l-and-its-mirror-image", rect(2, 4), [TETROMINOES["L4"], ["###", "..#"]]
    yield "023-piece-larger-than-board", rect(2, 2), [TETROMINOES["I4"]]
    yield "024-piece-taller-than-board-but-fits-turned", rect(1, 5), [["#"] * 5]
    yield "025-blank-margins", rect(2, 3), [["....", ".##.", ".#..", "...."], ["..", "#.", "##"]]
    yield "026-blank-margins-on-monomino", rect(1, 2), [["...", ".#.", "..."], ["#.", ".."]]
    yield "027-blocked-centre", rect(3, 3, {(1, 1)}), [TROMINOES["L3"], TROMINOES["L3"], DOMINO]
    yield "028-checkerboard-blocked", ["".join(".#"[(r + c) % 2] for c in range(6)) for r in range(6)], [MONOMINO] * 18
    yield "029-disconnected-regions", ["..#..", "..#..", "#####", "....."], [TETROMINOES["O4"], TETROMINOES["O4"], ["#####"]]
    yield "030-region-sizes-wrong", ["...#..", "...#.."], [TETROMINOES["T4"], TETROMINOES["T4"], DOMINO]
    yield "031-mixed-sizes-with-monomino", rect(3, 4), [TETROMINOES["T4"], TETROMINOES["L4"], TROMINOES["I3"], MONOMINO]
    yield "032-free-cells-far-corner", ["#" * 20] * 17 + ["#" * 17 + "..."] * 3, [TROMINOES["I3"]] * 3
    yield "033-big-board-big-pieces", rect(20, 20), [["#" * 5] * 5] * 16
    yield "034-big-pieces-on-rectangle", rect(8, 16), [["#" * 8] * 8, ["#" * 8] * 8]
    yield "035-long-strip", rect(1, 20), [["#"] * 4, ["#"] * 5, ["#"] * 5, ["#"] * 6]
    yield "036-tall-strip", rect(20, 1), [["#"] * 4, ["#"] * 5, ["#"] * 5, ["#"] * 6]
    yield "037-same-shape-drawn-differently", rect(4, 4), [TETROMINOES["L4"], ["#.", "#.", "##"], ["..#", "###"], ["##", ".#", ".#"]]
    yield "038-s-and-z-are-one-shape", ["#....#", "..##.."], [TETROMINOES["S4"], ["##.", ".##"]]
    yield "039-answer-above-2-to-the-53", rect(4, 5), [MONOMINO] * 18 + [DOMINO]
    yield "040-answer-near-2-to-the-63", rect(3, 7), [MONOMINO] * 19 + [DOMINO]
    yield "041-many-dominoes-2x16", rect(2, 16), [DOMINO] * 16
    yield "042-impossible-with-big-factorials", rect(6, 8, {(0, 1), (1, 0)}), [DOMINO] * 23
    yield "043-pieces-largest-first", rect(2, 5), [PENTOMINOES["I"], TROMINOES["I3"], DOMINO]
    yield "044-one-row-board", rect(1, 7), [DOMINO, DOMINO, TROMINOES["I3"]]
    yield "045-snake-region", [".....", "####.", ".....", ".####", "....."], [TROMINOES["I3"]] * 3 + [TETROMINOES["L4"], DOMINO, ["#"] * 2]
    yield "046-hole-shaped-like-piece", rect(4, 4, {(1, 1), (1, 2), (2, 1)}), [TROMINOES["L3"], TETROMINOES["L4"], TETROMINOES["L4"], DOMINO]
    yield "047-five-pentominoes-5x5", rect(5, 5), [PENTOMINOES[n] for n in "ILPYN"]
    yield "048-pentominoes-area-off-by-one", rect(3, 10, {(1, 5)}), [PENTOMINOES[n] for n in "FILPUY"]


def random_cases(rng: random.Random):
    for i in range(16):
        h, w = rng.randint(1, 5), rng.randint(1, 5)
        yield f"{49 + i:03d}-random-tiny", *carved_case(rng, h, w, rng.choice((0.0, 0.1, 0.3)), (1, 4), margins=i % 2 == 1)
    for i in range(12):
        h, w = rng.randint(4, 5), rng.randint(4, 5)
        yield f"{65 + i:03d}-random-small", *carved_case(rng, h, w, rng.choice((0.0, 0.1, 0.2)), (3, 6), margins=i % 3 == 0)
    for i in range(8):
        board, pieces = carved_case(rng, rng.randint(3, 5), rng.randint(3, 5), 0.1, (3, 5))
        match i % 3:
            case 0:
                pieces.pop()
            case 1:
                pieces.append(MONOMINO)
            case 2:
                pieces[0] = DOMINO if len(cells(pieces[0])) != 2 else TROMINOES["I3"]
        yield f"{77 + i:03d}-random-mismatch", board, pieces
    for i in range(6):
        h, w = rng.randint(12, 20), rng.randint(12, 20)
        region, pieces = carved_case(rng, rng.randint(3, 5), rng.randint(3, 6), 0.1, (4, 7), margins=True)
        top, left = rng.randint(0, h - len(region)), rng.randint(0, w - len(region[0]))
        board = ["#" * w] * top + ["#" * left + row + "#" * (w - left - len(row)) for row in region]
        board += ["#" * w] * (h - len(board))
        yield f"{85 + i:03d}-random-region-in-large-board", board, pieces


def pentomino_cases(rng: random.Random):
    for i in range(6):
        yield f"{91 + i:03d}-random-pentomino-regions", *carved_case(rng, rng.randint(4, 5), 5, 0.1, (5, 5))
    yield "097-pentominoes-5x6-with-duplicates", rect(5, 6), [disguise(rng, PENTOMINOES[n]) for n in "PPLLYN"]
    yield "098-tetrominoes-with-copies-4x6", rect(4, 6), [disguise(rng, TETROMINOES[n]) for n in ("I4", "O4", "L4", "L4", "S4", "S4")]


def cases():
    write("examples", "labeled-copies", rect(2, 3), [["##", "#."], ["#.", "##"]])
    write("examples", "symmetric-piece", ["....", "#..#"], [["###"], ["#"], ["#"], ["#"]])
    write("examples", "area-mismatch", rect(2, 2), [["##"], ["#"]])

    rng = random.Random(20261104)
    for case in [*edge_cases(), *random_cases(rng), *pentomino_cases(rng)]:
        write("hidden", *case)


def workloads():
    rng = random.Random(20261105)
    write("workloads", "pentominoes-3x20", rect(3, 20), [disguise(rng, p) for p in PENTOMINOES.values()], expect=False)
    tetrominoes = [disguise(rng, p) for p in TETROMINOES.values() for _ in range(2)]
    rng.shuffle(tetrominoes)
    write("workloads", "tetrominoes-twice-4x10", rect(4, 10), tetrominoes, expect=False)


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
