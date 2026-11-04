"""Writes the cases (examples/ and hidden/) or the workloads/ for regex-lite.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
"""

import hashlib
import random
import string
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
PRINTABLE = bytes(range(32, 127)).decode()
ESCAPABLE = "\\.*+?|()[]^$-"


def write(directory: str, name: str, pattern: str, lines: list[str], expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes("".join(f"{line}\n" for line in [pattern, *lines]).encode("ascii"))
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def basic_cases():
    yield "literal", "cat", ["cat", "concatenate cats", "c a t", "CAT Cat", "", "catcatcat", "ccatt"]
    yield "literal-no-overlap", "aba", ["ababa", "abababa", "aba aba", "ab", ""]
    yield "spaces-in-pattern", "a b ", ["a b a b ", "a b", "a  b ", "ab ", " a b  "]
    yield "braces-are-literals", "a{2}", ["aa", "a{2}", "a{2}a{2}", "{2}", "a{2,3}"]
    yield "close-bracket-and-dash-literals", "]-]", ["]-]", "]]--]]", "]-]-]", "[-]"]
    yield "dot", "a.c", ["abc", "a c", "a.c", "ac", "abbc", "a-ca+c", "a~ca"]
    yield "dot-every-character", ".", [PRINTABLE, "", " ", "~~"]
    yield "no-text-lines", "abc", []
    yield "empty-pattern", "", ["", "a", "abc", "  ", PRINTABLE]
    yield "empty-group", "()", ["", "ab", "()"]
    yield "empty-alternative-last", "a|", ["aba", "", "bb", "aa"]
    yield "empty-alternative-first", "|ab", ["abab", "", "aab", "ba"]
    yield "empty-alternative-in-group", "(|b)c", ["bc", "c", "bbc", "cc", "abcbc"]


def quantifier_cases():
    yield "star", "ab*c", ["ac", "abc", "abbbbc", "abbb", "acabc", "abcbc"]
    yield "plus", "ab+c", ["ac", "abc", "abbbbc", "abbb", "acabc", "abcabbc"]
    yield "question", "ab?c", ["ac", "abc", "abbc", "acabc", "abac"]
    yield "star-empty-matches", "a*", ["baaac", "", "aaa", "bbb", "aab", "aba"]
    yield "plus-takes-longest", "a+", ["aaa", "aaabaa", "", "b", "abababa"]
    yield "question-one-at-a-time", "a?", ["aaa", "", "ba", "bab"]
    yield "star-on-group", "(ab)*", ["ababab", "abaab", "", "ba", "aabb"]
    yield "question-on-group", "(ab)?c", ["abc", "c", "ac", "abcc", "ababc"]
    yield "star-on-class", "[0-9]*", ["12a345", "", "abc", "7"]
    yield "dot-star", ".*", ["abc", "", " ", PRINTABLE]
    yield "nested-quantified-groups", "((a|b)+c)*d", ["abcbacd", "d", "abd", "cd", "acbcdabd", "ccd"]


def alternation_cases():
    yield "alternation", "cat|dog|bird", ["cat dog bird", "catdog", "birdcage", "do g", "cadogt"]
    yield "concatenation-binds-tighter", "ab|cd", ["abcd", "acd", "abd", "a b c d", "cdab"]
    yield "quantifier-binds-tightest", "ab+", ["abab", "abbb", "a", "bbb"]
    yield "quantifier-on-group-only", "(ab)+", ["abab", "abbb", "a", "bbb", "abxab"]
    yield "star-inside-alternation", "ab*|c", ["abbbc", "c", "ac", "bc", "abcab"]
    yield "leftmost-longest-not-first", "a|ab|bc", ["abc", "abcabc", "aabc", "bcab"]
    yield "alternative-order-short-first", "x|xyz|xy", ["xyz", "xyxyz", "xxy", "xyzxy"]
    yield "alternative-order-long-first", "xyz|xy|x", ["xyz", "xyxyz", "xxy", "xyzxy"]
    yield "longest-across-concatenation", "(a|ab)(c|bcd)(d*)", ["abcd", "abcdd", "abcbcd", "acd", "abcdabcd"]
    yield "leftmost-beats-longest", "b|abcdefg|cdef", ["abcdef", "abcdefg", "xbcdef"]
    yield "deep-nesting", "(" * 450 + "ab" + ")" * 450, ["ab", "abab", "a b", ""]
    yield "quantified-alternation", "(a|bc)+d", ["abcad", "bcbcd", "ad", "d", "abcbd"]


def escape_cases():
    yield "escaped-dot", "a\\.b", ["a.b", "axb", "a.ba.b", "a\\.b"]
    yield "escaped-quantifiers-and-grouping", "\\*\\+\\?\\(\\|\\)", ["*+?(|)", "*+?(|)*+?(|)", "*+?()", "a*+?(|)b"]
    yield "escaped-brackets-and-anchors", "\\[\\]\\^\\$", ["[]^$", "[]$^", "a[]^$b[]^$", ""]
    yield "escaped-backslash", "\\\\", ["\\", "a\\b\\\\", "", "/"]
    yield "escaped-dash-and-close", "\\-\\]", ["-]", "--]]", "-]-]", "]-"]
    yield "escapes-in-class", "[\\]\\[\\\\\\-\\^]+", ["][\\-^a", "abc", "^-^", "[x]"]


def class_cases():
    yield "class", "[abc]+", ["aabbcc", "abcd", "xyz", "cxa"]
    yield "class-ranges", "[a-z0-9]+", ["abc123 DEF def_45", "", "A-Z", "zz00"]
    yield "class-negated", "[^a-z]+", ["abc123 DEF def_45", "", "abc", "ABC"]
    yield "class-negated-caret", "[^^]", ["^a^b^^", "^^^", "", "ab"]
    yield "class-caret-not-first", "[a^]+", ["a^b^^a", "bbb", "^"]
    yield "class-dash-first", "[-a]+", ["a-b--a", "b", "-"]
    yield "class-dash-last", "[a-]+", ["a-b--a", "b", "-"]
    yield "class-lone-dash", "[-]", ["a-b--a", "", "abc"]
    yield "class-metacharacters-literal", "[.*+?()|$[]+", ["a.*b+?(c)|$d[", "abc", "....", "^"]
    yield "class-all-printable", "[ -~]", [PRINTABLE, "", "  "]
    yield "class-negated-all-printable", "[^ -~]", [PRINTABLE, "", "  "]
    yield "class-one-character-ranges", "[a-a~-~ - ]", ["a~ b", "", "!}"]
    yield "class-edge-ranges", "[ -/:-@[-`{-~]+", ["ab, cd! [x] {y} @z", PRINTABLE, "abcXYZ019"]
    yield "class-range-across-cases", "[A-z]", ["[\\]^_`az@{", "AZaz", ""]
    yield "class-escaped-endpoints", "[\\[-\\]+-\\-]", ["[\\]Z^", "+,-.*", "abc", ""]
    yield "class-quantified", "[0-9]+(\\.[0-9]+)?", ["3.14 and 42 and 1.2.3", "1.", ".5", ""]


def anchor_cases():
    yield "anchor-start", "^ab", ["abab", "ab", "xab", "", " ab"]
    yield "anchor-end", "ab$", ["abab", "abx", "ab", "", "ab "]
    yield "anchor-both", "^ab$", ["ab", "abab", "", " ab"]
    yield "anchor-start-alone", "^", ["", "abc", " "]
    yield "anchor-empty-line", "^$", ["", "a", " ", ""]
    yield "anchor-start-not-after-advance", "^a", ["aaa", "a", "ba"]
    yield "anchor-mid-pattern", "a^b|a$b", ["ab", "a^b", "a$b", ""]
    yield "anchor-after-optional", "a*^b", ["bab", "b", "aab", "ab"]
    yield "anchor-quantified", "^*a|(^b)*c", ["aaa", "", "ba", "bcbc", "c"]
    yield "anchor-alternation", "^a|b$|c", ["abc", "acb", "cab", "a", "b"]
    yield "star-before-end", "x*$", ["axx", "", "x", "xax", "aaa"]


def empty_match_cases():
    yield "empty-match-after-match", "b*", ["abba", "", "bbb", "babab", "aaa"]
    yield "optional-group-empty", "(ab)?", ["abxab", "", "xx", "abab"]
    yield "nested-empty-star", "(a*)*", ["aab", "", "b", "aaa"]
    yield "optional-anchor-end", "a$?", ["aa", "", "ba", "a"]


def invalid_cases():
    text = ["abc", "(", ""]
    yield "invalid-open-paren", "(ab", text
    yield "invalid-close-paren", "ab)", text
    yield "invalid-leading-star", "*a", text
    yield "invalid-quantifier-after-open", "(+a)", text
    yield "invalid-quantifier-after-bar", "a|?b", text
    yield "invalid-double-quantifier", "a**", text
    yield "invalid-lazy-quantifier", "a+?", text
    yield "invalid-trailing-backslash", "ab\\", text
    yield "invalid-escape-letter", "\\d+", text
    yield "invalid-unterminated-class", "[abc", text
    yield "invalid-empty-class", "a[]", text
    yield "invalid-close-first-in-class", "[]a]", text
    yield "invalid-reversed-range", "[z-a]", text
    yield "invalid-chained-range", "[a-c-e]", text
    yield "invalid-dash-endpoint", "[a--]", text
    yield "invalid-no-text-lines", "(", []
    yield "invalid-many-lines", "a|*", ["abc"] * 1000


def trap_cases():
    """Patterns that match a span in exponentially many ways, at sizes that stop a matcher which
    tries the ways one by one, and that any polynomial matcher finishes at once."""
    runs = [30, 45, 60, 80, 100]
    yield "trap-alternation-star", "(a|aa)*b", [*("a" * k + "c" for k in runs), "a" * 60 + "b", "a" * 100]
    yield "trap-nested-star", "(a*)*b", [*("a" * k for k in runs), "a" * 50 + "cb", "a" * 40 + "b"]
    yield "trap-nested-plus", "(x+x+)+y", [*("x" * k for k in runs), "x" * 60 + "y" + "x" * 60]
    yield "trap-optional-chain", "a?" * 40 + "a" * 40, ["a" * 40, "a" * 80, "a" * 39, "a" * 79]
    yield "trap-dot-star-chain", "(.*a)" * 12 + "x", ["a" * 80, "ab" * 40, "a" * 30 + "x"]
    yield "trap-longest-of-many-parses", "(a|aa)*", ["a" * 100, "a" * 99 + "b" + "a" * 50, ""]
    yield "trap-anchored", "^(a+)+$", ["a" * 60 + "b", "a" * 80, "b" + "a" * 80]
    yield "trap-overlapping-alternatives", "(a|b|ab)*c", ["ab" * 50, "ab" * 50 + "c", "ba" * 50 + "d"]
    sentence = " ".join(["lorem", "ipsum", "dolor", "sit", "amet", "a", "est"] * 9)
    yield "trap-words", "([a-z]+ ?)+[.!?]", [sentence, sentence + ".", sentence + " ,", sentence + "! " + sentence]


def random_pattern(rng: random.Random, alphabet: str, depth: int) -> str:
    def literal() -> str:
        c = rng.choice(alphabet * 4 + ESCAPABLE + "]{} -")
        return "\\" + c if c in ESCAPABLE and (c not in "]-" or rng.random() < 0.5) else c

    def bracket() -> str:
        body = []
        for _ in range(rng.randint(1, 3)):
            a, b = sorted(rng.choices(alphabet + "^.[*", k=2))
            body.append(f"{a}-{b}" if rng.random() < 0.4 else a)
        if rng.random() < 0.15:
            body.insert(0, "-")
        if rng.random() < 0.15:
            body.append(rng.choice(["-", "\\]", "\\\\", "\\-"]))
        text = "".join(body)
        if rng.random() < 0.3:
            return f"[^{text}]"
        return f"[{text}]" if not text.startswith("^") else f"[{alphabet[0]}{text}]"

    def node(d: int) -> str:
        roll = rng.random()
        if d <= 0 or roll < 0.3:
            return rng.choice([literal, literal, literal, lambda: ".", bracket, lambda: rng.choice("^$")])()
        if roll < 0.55:
            return "".join(node(d - 1) for _ in range(rng.randint(0, 4)))
        if roll < 0.7:
            return "|".join(node(d - 1) for _ in range(rng.randint(2, 3)))
        if roll < 0.85:
            return "(" + node(d - 1) + ")" + rng.choice(["", "*", "+", "?"])
        inner = node(d - 1)
        return (inner if len(inner) == 1 and inner not in "|" else f"({inner})") + rng.choice("*+?")

    return node(depth)


def random_text(rng: random.Random, alphabet: str, count: int, low: int, high: int) -> list[str]:
    noise = alphabet + "-]{}. ^$\\[*()|+?"
    return [
        "".join(rng.choice(noise if rng.random() < 0.2 else alphabet) for _ in range(rng.randint(low, high)))
        for _ in range(count)
    ]


def random_cases(rng: random.Random):
    for _ in range(6):
        yield "random-tiny", random_pattern(rng, "ab", 3), random_text(rng, "ab", 6, 0, 8)
    for _ in range(6):
        alphabet = rng.choice(["abc", "abcd", "xyz01"])
        yield "random-medium", random_pattern(rng, alphabet, 5), random_text(rng, alphabet, rng.randint(50, 200), 0, 200)
    for _ in range(3):
        pattern = "".join(rng.choice("ab()[]|*+?\\^$-.") for _ in range(rng.randint(1, 12)))
        yield "random-noise-pattern", pattern, random_text(rng, "ab", 8, 0, 12)


def words_alternation(rng: random.Random, length: int) -> tuple[list[str], str]:
    words: list[str] = []
    pattern = ""
    while True:
        word = "".join(rng.choices(string.ascii_lowercase, k=rng.randint(3, 8)))
        if word in words:
            continue
        candidate = f"{pattern}|{word}" if pattern else word
        if len(candidate) > length:
            return words, pattern
        words.append(word)
        pattern = candidate


def large_cases(rng: random.Random):
    words, pattern = words_alternation(rng, 1000)
    filler = ["".join(rng.choices(string.ascii_lowercase, k=rng.randint(2, 9))) for _ in range(500)]
    lines = [" ".join(rng.choices(words if rng.random() < 0.3 else filler, k=12)) for _ in range(100)]
    yield "large-long-pattern", pattern, lines
    line = "".join(rng.choice("0123456789abc ") for _ in range(100_000))
    yield "large-long-line", "[0-9]+|[a-c]+[0-9]", [line, line[::-1]]
    yield "large-many-lines", "[a-z]+@[a-z]+\\.(com|org)", [
        rng.choice(["mail ", "", "to: "]) + "".join(rng.choices("abcxyz@.", k=rng.randint(0, 30))) + rng.choice([".com", ".org", ""])
        for _ in range(10_000)
    ]
    yield "large-max-count", "y*", ["x" * 100_000, "y" * 100_000, "xy" * 50_000]


def workload_logs(rng: random.Random, lines: int) -> list[str]:
    methods = ["GET", "GET", "GET", "POST", "PUT", "DELETE", "HEAD"]
    paths = ["api", "v1", "v2", "users", "orders", "items", "static", "img", "search", "login", "health"]
    notes = ["ok", "slow upstream", "warning: retrying", "error: timeout", "warn cache miss", "user error", ""]
    out = []
    for _ in range(lines):
        ip = ".".join(str(rng.randint(0, 255)) for _ in range(4))
        path = "/" + "/".join(rng.choices(paths, k=rng.randint(1, 4)))
        if rng.random() < 0.4:
            path += f"/{rng.randint(1, 99999)}"
        if rng.random() < 0.2:
            path += rng.choice([".html", ".png", "_old", "-v2"])
        stamp = f"2026-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}T{rng.randint(0, 23):02d}:{rng.randint(0, 59):02d}"
        out.append(
            f"{stamp} {ip} {rng.choice(methods)} {path} {rng.choice([200, 200, 201, 304, 404, 500])} "
            f"{rng.randint(0, 99999)} {rng.choice(notes)}"
        )
    return out


def workload_sentences(rng: random.Random, lines: int, tails: tuple[int, int], enders: list[str]) -> list[str]:
    """Lowercase words in runs, some ended by punctuation and some left open, so a run is a near
    match for a pattern that needs the punctuation."""
    vocab = ["".join(rng.choices(string.ascii_lowercase, k=rng.randint(1, 9))) for _ in range(2000)]
    out = []
    for _ in range(lines):
        parts = []
        for _ in range(rng.randint(1, 3)):
            length = rng.randint(*tails)
            words, size = [], 0
            while size < length:
                word = rng.choice(vocab)
                words.append(word)
                size += len(word) + 1
            parts.append(" ".join(words) + rng.choice(enders))
        out.append(" ".join(parts))
    return out


def cases():
    write("examples", "alternation", "a[0-9]*|ab", ["ab a12 ba", "xyz", "a7a7"])
    write("examples", "empty-matches", "b*", ["abba", "", "bbb"])
    write("examples", "invalid", "a(b|c", ["abc", "ac"])

    rng = random.Random(20261105)
    groups = [
        basic_cases(), quantifier_cases(), alternation_cases(), escape_cases(), class_cases(),
        anchor_cases(), empty_match_cases(), invalid_cases(), trap_cases(), random_cases(rng), large_cases(rng),
    ]
    for number, (name, pattern, lines) in enumerate((case for group in groups for case in group), 1):
        write("hidden", f"{number:03d}-{name}", pattern, lines)


def workloads():
    rng = random.Random(20261106)
    write(
        "workloads", "logs",
        "(GET|POST|PUT|DELETE) /[a-z0-9/._-]*|[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+|err(or)?|warn(ing)?",
        workload_logs(rng, 150_000), expect=False,
    )
    write(
        "workloads", "nested", "([a-z]+ ?)+[.!?]",
        workload_sentences(rng, 32_000, (20, 400), [".", "!", "?", ",", ";", " 42", ""]), expect=False,
    )
    write(
        "workloads", "labels", "[a-z]+( [a-z]+)*:|[a-z]+",
        workload_sentences(rng, 20_000, (20, 400), [":", ",", ";", ".", " 42", ""]), expect=False,
    )


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
