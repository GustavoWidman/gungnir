"""Writes the cases (examples/ and hidden/) or the workloads/ for word-freq.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
"""

import random
import string
import subprocess
import sys
from itertools import accumulate
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
PRINTABLE = bytes(range(32, 127)).decode()
SEPARATORS = [" "] * 30 + [", ", ". ", "\n", "\n\n", "-", "'", "\t", "\r\n", "; ", "!", "? ", " (", ") ", "_", '"', "42", " 7 ", "/"]


def write(directory: str, name: str, k: int, text: str, expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(f"{k}\n{text}".encode("ascii"))
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def vocabulary(rng: random.Random, size: int, max_len: int = 12) -> list[str]:
    words = set()
    while len(words) < size:
        n = min(max_len, max(1, int(rng.expovariate(1 / 5))))
        words.add("".join(rng.choices(string.ascii_lowercase, k=n)))
    ordered = sorted(words)
    rng.shuffle(ordered)
    return ordered


def zipf_text(rng: random.Random, vocab: list[str], tokens: int, exponent: float = 1.0) -> str:
    """Draws words with Zipf weights, varies their case, and joins them with mixed separators."""
    cum = list(accumulate(1 / (rank + 1) ** exponent for rank in range(len(vocab))))
    picks = rng.choices(vocab, cum_weights=cum, k=tokens)
    seps = rng.choices(SEPARATORS, k=tokens)
    styles = rng.choices((str.lower, str.capitalize, str.upper), weights=(80, 15, 5), k=tokens)
    return "".join(style(w) + sep for w, sep, style in zip(picks, seps, styles))


def edge_cases(rng: random.Random):
    yield "001-empty-text", 3, ""
    yield "002-only-separators", 5, "123 ,.;!? \t\r\n___ 456\n"
    yield "003-single-word-no-newline", 1, "hello"
    yield "004-k-exceeds-distinct", 100, "one two two three three three\n"
    yield "005-k-one", 1, "b a b a c\n"
    yield "006-all-ties", 4, "delta alpha echo charlie bravo foxtrot\n"
    yield "007-prefix-ties", 10, "abc ab a b ba abc ab a b ba\n"
    yield "008-case-folding", 5, "Hello HELLO hello hElLo World WORLD\n"
    yield "009-digits-split", 5, "abc123def456abc 7a8b9c\n"
    yield "010-apostrophes-hyphens", 10, "Don't stop. It's e-mail, re-read the cat's toy; don't!\n"
    yield "011-crlf-and-tabs", 5, "one\ttwo\r\nthree\r\none\r\n\ttwo\r\n"
    yield "012-edge-separators", 5, "...word,,,word---WORD!!!"
    yield "013-long-word", 3, "x" * 100_000 + " short " + "X" * 100_000 + "\n"
    yield "014-every-printable", 60, PRINTABLE + "\n" + PRINTABLE[::-1] + "\n"
    yield "015-only-newlines", 2, "\n\n\n\n"
    yield "016-k-equals-distinct", 3, "c b a c b c\n"
    yield "017-single-letters", 3, "a A a Z z b\n"
    yield "018-huge-k", 1_000_000, "the quick brown fox jumps over the lazy dog\n"
    yield "019-underscores-and-brackets", 5, "snake_case [bracket] {brace} `tick` ~tilde^caret@at\n"
    yield "020-letter-boundaries", 10, "@A Z[ `a z{ AZaz\n"
    yield "021-one-word-many-times", 2, "Repeat " * 10_000
    yield "022-all-distinct", 20, " ".join(vocabulary(rng, 500)) + "\n"


def random_cases(rng: random.Random):
    for i in range(10):
        vocab = vocabulary(rng, rng.randint(1, 30), max_len=4)
        yield f"{23 + i:03d}-random-tiny", rng.randint(1, 10), zipf_text(rng, vocab, rng.randint(1, 60))
    for i in range(10):
        vocab = vocabulary(rng, rng.randint(100, 5000))
        yield f"{33 + i:03d}-random-medium", rng.randint(1, 200), zipf_text(rng, vocab, rng.randint(1000, 20_000))
    for i in range(5):
        vocab = vocabulary(rng, rng.randint(2, 50), max_len=3)
        yield f"{43 + i:03d}-random-heavy-ties", rng.randint(5, 100), zipf_text(rng, vocab, 5000, exponent=0.0)


def large_cases(rng: random.Random):
    yield "048-large-zipf", 1000, zipf_text(rng, vocabulary(rng, 20_000), 100_000)
    yield "049-large-distinct", 10_000, zipf_text(rng, vocabulary(rng, 40_000, max_len=10), 60_000, exponent=0.3)
    yield "050-large-long-words", 50, zipf_text(rng, vocabulary(rng, 2000, max_len=200), 20_000)


def cases():
    write("examples", "ties-and-apostrophe", 3, "The cat and the hat. THE END, and the cat's hat!\n")
    write("examples", "fewer-than-k", 10, "x1y2z3 x-ray X\n")
    write("examples", "no-words", 5, "123 -- 456 ... !!!\n")

    rng = random.Random(20261103)
    for case in [*edge_cases(rng), *random_cases(rng), *large_cases(rng)]:
        write("hidden", *case)


def workloads():
    rng = random.Random(20261104)
    write("workloads", "zipf", 100, zipf_text(rng, vocabulary(rng, 100_000), 5_000_000), expect=False)
    write("workloads", "many-distinct", 1000, zipf_text(rng, vocabulary(rng, 250_000, max_len=9), 2_500_000, exponent=0.5), expect=False)


def main():
    commands = {"cases": cases, "workloads": workloads}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
