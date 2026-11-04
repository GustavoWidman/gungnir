"""Writes the cases (examples/ and hidden/) or the workloads/ for line-diff.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
"""

import hashlib
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"

Pair = tuple[str, str]

NAMES = ["count", "total", "index", "value", "result", "buffer", "offset", "length", "node", "item", "key", "left", "right", "size"]
CALLS = ["read", "write", "parse", "check", "update", "insert", "remove", "lookup", "flush", "merge"]
# Lines that recur all through real source files, so a line often has several equal partners.
COMMON = ["", "", "", "", "}", "}", "    }", "    }", "        }", "{", "    } else {", "    return result;", "        break;", "        continue;", "#endif"]


def render(pairs: list[Pair]) -> bytes:
    out = [f"{len(pairs)}\n"]
    for first, second in pairs:
        out.append(f"{len(first)} {len(second)}\n{first}{second}")
    return "".join(out).encode("ascii")


def write(directory: str, name: str, pairs: list[Pair], expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(render(pairs))
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def doc(lines: list[str], final_newline: bool = True) -> str:
    """Joins lines into a document. Without a final newline the last line is left unterminated."""
    if not lines:
        return ""
    return "\n".join(lines) + ("\n" if final_newline else "")


def code_line(rng: random.Random) -> str:
    if rng.random() < 0.3:
        return rng.choice(COMMON)
    indent = " " * 4 * rng.randint(1, 3)
    a, b = rng.sample(NAMES, 2)
    match rng.randrange(4):
        case 0:
            return f"{indent}{a} = {rng.choice(CALLS)}({b}, {rng.randrange(100)});"
        case 1:
            return f"{indent}if ({a} > {b} + {rng.randrange(10)}) {{"
        case 2:
            return f"{indent}{a} += {b}[{rng.randrange(50)}];"
        case _:
            return f"{indent}{rng.choice(CALLS)}_{a}(&{b});"


def code(rng: random.Random, n: int) -> list[str]:
    return [code_line(rng) for _ in range(n)]


def short_line(rng: random.Random) -> str:
    """A terse line, so the large hidden cases stay small enough to commit."""
    if rng.random() < 0.3:
        return rng.choice(COMMON)
    return f"{rng.choice(NAMES)}{rng.randrange(1000)}"


def short(rng: random.Random, n: int) -> list[str]:
    return [short_line(rng) for _ in range(n)]


def edit(rng: random.Random, lines: list[str], sites: int, fresh) -> list[str]:
    """Applies `sites` edits at scattered places: deletions, insertions, replacements,
    duplicated lines and short moves. `fresh(rng)` makes each new line."""
    out = list(lines)
    for pos in sorted((rng.randint(0, len(lines)) for _ in range(sites)), reverse=True):
        pos = min(pos, len(out))
        match rng.randrange(5):
            case 0:
                del out[pos : pos + rng.randint(1, 3)]
            case 1:
                out[pos:pos] = [fresh(rng) for _ in range(rng.randint(1, 3))]
            case 2:
                out[pos : pos + rng.randint(1, 3)] = [fresh(rng) for _ in range(rng.randint(1, 3))]
            case 3:
                if out:
                    src = min(pos, len(out) - 1)
                    out.insert(pos, out[src])
            case _:
                block = out[pos : pos + rng.randint(1, 3)]
                del out[pos : pos + len(block)]
                to = min(len(out), pos + rng.randint(1, 30))
                out[to:to] = block
    return out


def vocabulary(rng: random.Random, size: int):
    """A line source drawing from `size` lines with Zipf weights, as in logs and data files."""
    words = [f"{rng.choice(CALLS)} {rng.choice(NAMES)} {i}" for i in range(size)]
    weights = [1 / (rank + 1) for rank in range(size)]
    return lambda rng: rng.choices(words, weights)[0]


def tiny(rng: random.Random, alphabet: list[str]) -> str:
    lines = [rng.choice(alphabet) for _ in range(rng.randint(0, 6))]
    return doc(lines, final_newline=rng.random() < 0.75)


def edge_cases(rng: random.Random):
    fn = code(rng, 40)
    ten = [f"line {i}" for i in range(1, 11)]
    yield "001-no-pairs", []
    yield "002-both-empty", [("", "")]
    yield "003-first-empty", [("", "alpha\nbeta\ngamma\n")]
    yield "004-second-empty", [("alpha\nbeta\n", "")]
    yield "005-identical", [(doc(fn), doc(fn))]
    yield "006-identical-no-final-newline", [("one\ntwo\nthree", "one\ntwo\nthree")]
    yield "007-one-line-changed", [("x\n", "y\n")]
    yield "008-blank-lines-only", [("\n\n\n", "\n\n"), ("\n", "\n\n\n\n")]
    yield "009-blank-line-vs-empty", [("\n", ""), ("", "\n")]
    yield "010-final-newline-added", [("a\nb", "a\nb\n")]
    yield "011-final-newline-removed", [("a\nb\n", "a\nb")]
    yield "012-single-line-final-newline", [("a", "a\n"), ("a\n", "a"), ("a", "a")]
    yield "013-unterminated-repeat", [("x\nx", "x\n"), ("x\n", "x\nx"), ("x\nx\n", "x\nx"), ("x\nx", "x\nx\n")]
    yield "014-unterminated-line-moves", [("a\nb\nc", "c\na\nb\n"), ("c\na\nb\n", "a\nb\nc")]
    yield "015-trailing-blank-line", [("a\n\n", "a\n"), ("a\n", "a\n\n"), ("a\n\n", "a\n\n\n")]
    yield "016-crlf-vs-lf", [("a\r\nb\r\nc\r\n", "a\nb\nc\n"), ("a\nb\r\nc\n", "a\nb\nc\n")]
    yield "017-carriage-return-inside-line", [("a\rb\nc\n", "a\nb\nc\n"), ("\r\n\r\n", "\n\r\n")]
    yield "018-trailing-carriage-return", [("a\r", "a"), ("a\r\n", "a\r"), ("a\r", "a\r")]
    yield "019-whitespace-is-significant", [("a \nb\n\tc\n", "a\nb \n    c\n"), ("x\n", " x\n")]
    yield "020-swap-two-lines", [("x\na\n", "a\nx\n")]
    yield "021-repeated-line-inserted", [("a\n", "a\na\n")]
    yield "022-repeated-line-deleted", [("a\na\n", "a\n")]
    yield "023-same-line-grows", [("a\n" * 5, "a\n" * 8)]
    yield "024-same-line-shrinks", [("a\n" * 8, "a\n" * 3)]
    yield "025-common-suffix-not-kept-in-place", [("p\ns\nt\n", "p\ns\ns\nt\n"), ("p\ns\ns\nt\n", "p\ns\nt\n")]
    yield "026-prefix-and-suffix-overlap", [("a\na\n", "a\na\na\n"), ("a\nb\na\n", "a\nb\na\nb\na\n")]
    yield "027-reversed", [(doc(ten), doc(ten[::-1]))]
    yield "028-swap-equal-blocks", [(doc(ten[:4] + ten[4:8]), doc(ten[4:8] + ten[:4]))]
    yield "029-swap-unequal-blocks", [(doc(ten[:3] + ten[3:9]), doc(ten[3:9] + ten[:3]))]
    yield "030-completely-different", [("a\nb\nc\n", "d\ne\n"), (doc(ten[:5]), doc(ten[5:]))]
    yield "031-insert-at-start", [(doc(ten), doc(["new"] + ten))]
    yield "032-insert-at-end", [(doc(ten), doc(ten + ["new", "newer"]))]
    yield "033-delete-at-start", [(doc(ten), doc(ten[2:]))]
    yield "034-delete-at-end", [(doc(ten), doc(ten[:-1]))]
    yield "035-all-but-one-replaced", [("a\nb\nc\nd\ne\n", "f\ng\nc\nh\ni\n")]
    yield "036-interleaved", [("a\nb\na\nb\na\nb\n", "b\na\nb\na\nb\na\n")]
    yield "037-replace-first-of-repeated", [("b\nb\n", "a\nb\n"), ("b\nb\nb\n", "a\nb\nb\n")]
    yield "038-duplicate-line-delete", [("a\nx\na\n", "a\n"), ("x\na\na\n", "a\n")]
    yield "039-duplicate-block-inserted", [("f {\n  x\n}\n", "f {\n  x\n}\nf {\n  x\n}\n")]
    body = ["f() {", "    x;", "}", "", "g() {", "    y;", "}"]
    added = body[:4] + ["h() {", "    z;", "}", ""] + body[4:]
    yield "040-brace-ambiguity", [(doc(body), doc(added)), (doc(added), doc(body))]
    spaced = [line for i in range(1, 9) for line in (f"item {i}", "")]
    yield "041-alternating-blank-lines", [(doc(spaced), doc(spaced[:6] + ["item x", ""] + spaced[6:])), (doc(spaced), doc(spaced[2:]))]
    yield "042-one-line-documents", [(f, s) for f in ("", "a", "a\n", "b\n") for s in ("", "a", "a\n", "b\n")]
    yield "043-pairs-without-final-newlines", [("1\n2\n3", "1\n3"), ("x", "y"), ("", "z"), ("q\nr", "q\nr")]
    yield "044-empty-pairs-between", [("", ""), ("a\n", "b\n"), ("", ""), ("", ""), ("c\n", "c\n"), ("", "")]
    yield "045-long-line", [("x" * 50_000 + "\n", "x" * 49_999 + "y\n"), ("y" * 30_000, "y" * 30_000)]


def random_cases(rng: random.Random):
    n = 46
    for i, alphabet in enumerate([["a", "b"], ["a", "b", ""], ["a", "b", "c", ""], ["a", "a\r", "a\rb", "b", ""]]):
        yield f"{n + i:03d}-random-tiny-pairs", [(tiny(rng, alphabet), tiny(rng, alphabet)) for _ in range(400)]
    n += 4
    for i in range(8):
        pairs = []
        for _ in range(20):
            base = code(rng, rng.randint(0, 150))
            pairs.append((doc(base, rng.random() < 0.8), doc(edit(rng, base, rng.randint(0, 12), code_line), rng.random() < 0.8)))
        yield f"{n + i:03d}-random-small", pairs
    n += 8
    for i in range(8):
        pairs = []
        for _ in range(rng.randint(1, 2)):
            base = code(rng, rng.randint(500, 1500))
            pairs.append((doc(base), doc(edit(rng, base, rng.randint(20, 120), code_line))))
        yield f"{n + i:03d}-random-medium", pairs
    n += 8
    for i in range(6):
        fresh = vocabulary(rng, rng.randint(2, 8))
        base = [fresh(rng) for _ in range(rng.randint(200, 1500))]
        yield f"{n + i:03d}-random-repetitive", [(doc(base), doc(edit(rng, base, rng.randint(10, 80), fresh)))]
    n += 6
    for i, size in enumerate((100, 300, 500)):
        yield f"{n + i:03d}-random-unrelated", [(doc(code(rng, size)), doc(code(rng, size + 7 * i)))]
    n += 3
    for i in range(4):
        fresh = vocabulary(rng, 3)
        yield f"{n + i:03d}-random-unrelated-repetitive", [(doc([fresh(rng) for _ in range(400)]), doc([fresh(rng) for _ in range(450)]))]


def large_cases(rng: random.Random):
    n = 80
    shared = short(rng, 15_000)
    yield f"{n:03d}-large-common-prefix", [(doc(shared + short(rng, 30)), doc(shared + short(rng, 25)))]
    yield f"{n + 1:03d}-large-common-suffix", [(doc(short(rng, 30) + shared), doc(short(rng, 40) + shared))]
    head, tail = shared[:7_000], shared[7_000:]
    yield f"{n + 2:03d}-large-prefix-and-suffix", [(doc(head + short(rng, 20) + tail), doc(head + short(rng, 10) + tail))]
    # The longer document repeats the first lines of the common suffix, so cutting the suffix
    # before diffing keeps the wrong copies.
    yield f"{n + 3:03d}-large-suffix-repeats", [(doc(head + tail), doc(head + tail[:1] + tail)), (doc(head + tail[:3] + tail), doc(head + tail))]
    yield f"{n + 4:03d}-large-insert-at-start", [(doc(shared), doc(["// header"] + shared))]
    yield f"{n + 5:03d}-large-delete-at-end", [(doc(shared), doc(shared[:-2], final_newline=False))]
    base = short(rng, 15_000)
    yield f"{n + 6:03d}-large-scattered", [(doc(base), doc(edit(rng, base, 300, short_line)))]
    fresh = vocabulary(rng, 6)
    base = [fresh(rng) for _ in range(5_000)]
    yield f"{n + 7:03d}-large-repetitive", [(doc(base), doc(edit(rng, base, 120, fresh)))]
    pairs = []
    for _ in range(150):
        base = short(rng, rng.randint(50, 250))
        pairs.append((doc(base), doc(edit(rng, base, rng.randint(1, 8), short_line))))
    yield f"{n + 8:03d}-large-many-pairs", pairs


def examples():
    yield "replace-and-repeat", [("a\nb\nc\nd\n", "a\nx\nc\nd\ne\n"), ("a\nb\n", "a\na\nb\n")]
    yield "newline-and-swap", [("one\ntwo", "one\ntwo\n"), ("x\na\n", "a\nx\n"), ("", "")]
    yield "empty-documents", [("", "a\nb\n"), ("a\nb\n", ""), ("", "")]


def cases():
    for name, pairs in examples():
        write("examples", name, pairs)
    rng = random.Random(20261104)
    for case in [*edge_cases(rng), *random_cases(rng), *large_cases(rng)]:
        write("hidden", *case)


def workloads():
    rng = random.Random(20261105)
    base = code(rng, 80_000)
    write("workloads", "scattered", [(doc(base), doc(edit(rng, base, 1150, code_line)))], expect=False)
    pairs = []
    for _ in range(200):
        base = code(rng, 3000)
        pairs.append((doc(base), doc(edit(rng, base, 60, code_line))))
    write("workloads", "many-pairs", pairs, expect=False)
    fresh = vocabulary(rng, 8)
    base = [fresh(rng) for _ in range(80_000)]
    write("workloads", "repetitive", [(doc(base), doc(edit(rng, base, 1000, fresh)))], expect=False)


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
