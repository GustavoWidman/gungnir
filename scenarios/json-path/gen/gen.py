"""Writes the cases (examples/ and hidden/) or the workloads/ for json-path.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.

A document is built as a model and then rendered to bytes. In the model an object is an `Obj`
(a list of (name, value) pairs, so names may repeat), an array is a list, a string is a str
whose surrogate code points stand for lone surrogates, and integers, booleans and None are
themselves. Rendering picks whitespace and escapes at random, so the same model can be
written many ways.
"""

import hashlib
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
MIN, MAX = -(2**63), 2**63 - 1
MAX_DEPTH = 1000


class Obj(list):
    pass


SHORT = {'"': '\\"', "\\": "\\\\", "\b": "\\b", "\f": "\\f", "\n": "\\n", "\r": "\\r", "\t": "\\t"}
IDENTIFIER_START = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ_")
IDENTIFIER = IDENTIFIER_START | set("0123456789")
WORDS = ["alpha", "beta", "gamma", "delta", "id", "name", "value", "items", "data", "x", "y", "a", "b", "k1", "_t"]
NAMES = WORDS + ["true", "null", "a b", "a.b", "é", "😀", "", '"q"', "\\", "[0]", "$", "*", "-", "0", "\ud800", "\x7f", "tab\t"]
TEXTS = ["", "plain", "with space", "é", "ß", "中文", "😀", "𝄞", "Ω", "\u2028", "\u0000", "\x7f", "\t", '"', "\\", "/",
         "\ud800", "\udfff", "\udc00\ud800", "\uffff", "\u0080", "\u07ff", "\u0800", "\U0010ffff", "a/b", "line\nbreak"]
WHITESPACE = [" ", "  ", "\n", "\t", "\r\n", "\n    ", " \t"]


class Style:
    """How a model is rendered: how often whitespace appears and how often characters are escaped."""

    def __init__(self, rng: random.Random, space: float = 0.0, escape: float = 0.0, upper: float = 0.0):
        self.rng, self.space, self.escape, self.upper = rng, space, escape, upper

    def ws(self) -> str:
        return self.rng.choice(WHITESPACE) if self.space and self.rng.random() < self.space else ""

    def unit(self, code: int) -> str:
        text = f"\\u{code:04x}"
        return text.upper().replace("\\U", "\\u") if self.upper and self.rng.random() < self.upper else text

    def string(self, s: str) -> str:
        out = ['"']
        for c in s:
            o = ord(c)
            escape = self.escape and self.rng.random() < self.escape
            if c in SHORT and c not in "\b\f\n\r\t":
                out.append(SHORT[c])
            elif o < 0x20:
                out.append(self.unit(o) if escape or c not in SHORT else SHORT[c])
            elif 0xD800 <= o <= 0xDFFF:
                out.append(self.unit(o))
            elif o > 0xFFFF and escape:
                o -= 0x10000
                out.append(self.unit(0xD800 + (o >> 10)) + self.unit(0xDC00 + (o & 0x3FF)))
            elif c == "/" and escape:
                out.append("\\/")
            elif escape:
                out.append(self.unit(o))
            else:
                out.append(c)
        out.append('"')
        return "".join(out)

    def render(self, value) -> str:
        out: list[str] = []
        self._render(value, out)
        return "".join(out)

    def _render(self, value, out: list[str]):
        if isinstance(value, Obj):
            out.append("{")
            for j, (name, v) in enumerate(value):
                out.append(("," if j else "") + self.ws() + self.string(name) + self.ws() + ":" + self.ws())
                self._render(v, out)
                out.append(self.ws())
            out.append("}")
        elif isinstance(value, list):
            out.append("[")
            for j, v in enumerate(value):
                out.append(("," if j else "") + self.ws())
                self._render(v, out)
                out.append(self.ws())
            out.append("]")
        elif isinstance(value, str):
            out.append(self.string(value))
        elif value is None or isinstance(value, bool):
            out.append({None: "null", True: "true", False: "false"}[value])
        elif value == 0 and self.rng.random() < 0.2:
            out.append("-0")
        else:
            out.append(str(value))


def document(value, style: Style | None = None) -> bytes:
    style = style or Style(random.Random(0))
    return (style.ws() + style.render(value) + style.ws()).encode("utf-8", "surrogatepass")


def literal(name: str, rng: random.Random | None = None) -> str:
    """A query string literal for name, printable ASCII, escaping at random when rng is given."""
    out = ['"']
    for c in name:
        o = ord(c)
        if c in SHORT:
            out.append(SHORT[c] if rng is None or rng.random() < 0.7 else f"\\u{o:04x}")
        elif o < 0x20 or o > 0x7E:
            if o > 0xFFFF:
                o -= 0x10000
                out.append(f"\\u{0xD800 + (o >> 10):04x}\\u{0xDC00 + (o & 0x3FF):04x}")
            else:
                out.append(f"\\u{o:04X}" if rng and rng.random() < 0.3 else f"\\u{o:04x}")
        elif rng and rng.random() < 0.1:
            out.append("\\/" if c == "/" else f"\\u{o:04x}")
        else:
            out.append(c)
    out.append('"')
    return "".join(out)


def name_segment(name: str, rng: random.Random, descendant: bool = False) -> str:
    dots = ".." if descendant else "."
    if name and name[0] in IDENTIFIER_START and all(c in IDENTIFIER for c in name) and rng.random() < 0.7:
        return dots + name
    return ("..[" if descendant else "[") + literal(name, rng) + "]"


def nesting(doc: bytes) -> int:
    """The most `[` and `{` outside strings that are open at once, as the spec bounds it."""
    depth = deepest = 0
    in_string = escaped = False
    for byte in doc:
        if in_string:
            if escaped:
                escaped = False
            elif byte == 0x5C:
                escaped = True
            elif byte == 0x22:
                in_string = False
        elif byte == 0x22:
            in_string = True
        elif byte in b"[{":
            depth += 1
            deepest = max(deepest, depth)
        elif byte in b"]}":
            depth -= 1
    return deepest


def check_constraints(queries: list[str], doc: bytes):
    assert len(queries) <= 100_000
    for q in queries:
        assert len(q) <= 1000 and all(32 <= ord(c) <= 126 for c in q), q
    assert len(doc) <= 100_000_000
    assert nesting(doc) <= MAX_DEPTH


def run_reference(path: Path) -> bytes:
    return subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True).stdout


def block_sizes(output: bytes) -> list[int]:
    """The byte length of each query's block in the output of a valid document."""
    lines = output.split(b"\n")[:-1]
    sizes, i = [], 0
    while i < len(lines):
        height = 1 if lines[i] == b"invalid query" else int(lines[i]) + 1
        sizes.append(sum(len(line) + 1 for line in lines[i : i + height]))
        i += height
    return sizes


def write(directory: str, name: str, queries: list[str], doc: bytes, expect: bool = True, budget: int | None = None):
    """Writes NAME.in and, when expect is set, NAME.out from the reference.

    With a budget, queries whose output block is longer than budget bytes are dropped first, so a
    random query that prints a large subtree many times cannot bloat the committed cases.
    """
    check_constraints(queries, doc)
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)

    def put(queries: list[str]):
        head = "".join(q + "\n" for q in queries)
        path.write_bytes(f"{len(queries)}\n{head}".encode("ascii") + doc)

    put(queries)
    if budget is not None:
        output = run_reference(path)
        if not output.startswith(b"invalid document"):
            put([q for q, size in zip(queries, block_sizes(output), strict=True) if size <= budget])
    if expect:
        path.with_suffix(".out").write_bytes(run_reference(path))


# Random models and queries.


def random_value(rng: random.Random, depth: int, budget: list[int], names: list[str], texts: list[str]):
    budget[0] -= 1
    r = rng.random()
    if depth > 0 and budget[0] > 0 and r < 0.45:
        n = min(budget[0], rng.choice((0, 1, 2, 3, 4, 6, 10)))
        if rng.random() < 0.5:
            return Obj((rng.choice(names), random_value(rng, depth - 1, budget, names, texts)) for _ in range(n))
        return [random_value(rng, depth - 1, budget, names, texts) for _ in range(n)]
    if r < 0.65:
        return rng.choice(texts) if rng.random() < 0.7 else "".join(rng.choices(texts, k=rng.randint(2, 5)))
    if r < 0.85:
        return rng.choice((0, 1, -1, 7, 42, -0, 255, 65536, MIN, MAX, rng.randint(-10**6, 10**6), rng.randint(MIN, MAX)))
    return rng.choice((True, False, None))


def random_model(rng: random.Random, nodes: int, depth: int, names=NAMES, texts=TEXTS):
    budget = [nodes]
    root = Obj() if rng.random() < 0.6 else []
    while budget[0] > 0:
        value = random_value(rng, depth, budget, names, texts)
        if isinstance(root, Obj):
            root.append((rng.choice(names), value))
        else:
            root.append(value)
    return root


def random_bound(rng: random.Random, length: int) -> str:
    return str(rng.choice((0, 1, 2, -1, -2, length, -length, length + 1, -length - 1, rng.randint(-length - 2, length + 2))))


def random_selector(rng: random.Random, node, names: list[str], descendant: bool):
    """Returns (segment text, the node it leads to or None when it picks none or many)."""
    r = rng.random()
    if isinstance(node, Obj) and node and r < 0.6:
        name, value = rng.choice(node)
        return name_segment(name, rng, descendant), None if descendant else value
    if isinstance(node, list) and not isinstance(node, Obj) and r < 0.6:
        n = len(node)
        if rng.random() < 0.6 and n:
            i = rng.randrange(n)
            index = i - n if rng.random() < 0.4 else i
            return ("..[" if descendant else "[") + f"{index}]", None if descendant else node[i]
        parts = [random_bound(rng, n) if rng.random() < 0.7 else "", random_bound(rng, n) if rng.random() < 0.7 else ""]
        if rng.random() < 0.4:
            parts.append(str(rng.choice((1, 2, 3, n or 1, 7))) if rng.random() < 0.8 else "")
        return ("..[" if descendant else "[") + ":".join(parts) + "]", None
    if r < 0.75:
        return rng.choice(("..*", "..[*]")) if descendant else rng.choice((".*", "[*]")), None
    if r < 0.9:
        return name_segment(rng.choice(names), rng, descendant), None
    index = rng.choice((0, 1, -1, 2, 5, -3))
    return ("..[" if descendant else "[") + f"{index}]", None


def random_query(rng: random.Random, root, names: list[str]) -> str:
    query, node = "$", root
    for _ in range(rng.choice((0, 1, 1, 2, 2, 3, 3, 4, 5, 6))):
        descendant = rng.random() < 0.25
        segment, node = random_selector(rng, node, names, descendant)
        query += segment
    return query


MUTATIONS = [
    lambda q, rng: q + " ",
    lambda q, rng: " " + q,
    lambda q, rng: q.replace('"', "'"),
    lambda q, rng: q.replace("[", "[0", 1),
    lambda q, rng: q.replace("[", "[+", 1),
    lambda q, rng: q.replace("]", "", 1),
    lambda q, rng: q.replace(":", "::", 1) + "[1:2:3:4]",
    lambda q, rng: q + ".",
    lambda q, rng: q + "..",
    lambda q, rng: q.replace("$", "", 1),
    lambda q, rng: q + "[::0]",
    lambda q, rng: q + "[::-1]",
    lambda q, rng: q + ".[0]",
    lambda q, rng: q + "[]",
    lambda q, rng: q + '["\\x"]',
    lambda q, rng: q + "[9223372036854775808]",
    lambda q, rng: q + ".1a",
    lambda q, rng: q + "[1.5]",
]


def mutate_query(rng: random.Random, query: str) -> str:
    return rng.choice(MUTATIONS)(query, rng)


def random_queries(rng: random.Random, root, count: int, names=NAMES, invalid: float = 0.05) -> list[str]:
    queries = []
    for _ in range(count):
        q = random_query(rng, root, names)
        queries.append(mutate_query(rng, q) if rng.random() < invalid else q)
    return queries


# Hand-written cases.


def nest(depth: int, inner, kind: str = "array"):
    value = inner
    for level in range(depth):
        if kind == "array" or (kind == "mixed" and level % 2):
            value = [value]
        else:
            value = Obj([("k" if kind == "mixed" else "a", value)])
    return value


def raw(*parts) -> bytes:
    return b"".join(p.encode("utf-8", "surrogatepass") if isinstance(p, str) else p for p in parts)


STORE = (
    b'{"store": {"book": [{"title": "Sayings", "price": 8, "author": "Rees"},\n'
    b'                    {"title": "Moby Dick", "price": 9, "author": "Melville"},\n'
    b'                    {"title": "Lord", "price": -0, "author": "Tolkien"}],\n'
    b'           "bicycle": {"color": "red", "price": 19}},\n'
    b' "price": 1}\n'
)
STRINGS = raw(
    '{"x": {"a": 1}, "a": {"a": {"a": 2}},\n',
    ' "k": "first", "k": "second",\n',
    ' "café": [], "s": "tab\\there \\/ \\u00E9\\ud83d\\ude00\\udc00", "t": "\\u007f"}\n',
)

INVALID_DOCUMENTS = [
    ("empty", b""),
    ("only-whitespace", b" \t\r\n "),
    ("second-value", b'{"a":1} {"b":2}'),
    ("fraction", b"[1.5]"),
    ("exponent", b"[1e3]"),
    ("leading-zero", b"[01]"),
    ("negative-leading-zero", b"[-01]"),
    ("plus-sign", b"[+1]"),
    ("lone-minus", b"[-]"),
    ("integer-above-range", b"[9223372036854775808]"),
    ("integer-below-range", b"[-9223372036854775809]"),
    ("integer-many-digits", b'{"n": 123456789012345678901234567890}'),
    ("nan", b"[NaN]"),
    ("infinity", b"[-Infinity]"),
    ("capital-literal", b"[True]"),
    ("truncated-literal", b"[nul]"),
    ("literal-extra-letters", b"[nulls]"),
    ("single-quotes", b"{'a': 1}"),
    ("unquoted-name", b"{a: 1}"),
    ("trailing-comma-array", b"[1, 2,]"),
    ("trailing-comma-object", b'{"a": 1,}'),
    ("missing-comma", b"[1 2]"),
    ("missing-colon", b'{"a" 1}'),
    ("missing-value", b'{"a":}'),
    ("unclosed-array", b'[1, [2, 3]'),
    ("unclosed-object", b'{"a": {"b": 1}'),
    ("unclosed-string", b'["abc]'),
    ("extra-close", b"[1]]"),
    ("mismatched-close", b'{"a": [1}]'),
    ("unknown-escape", b'["\\x41"]'),
    ("capital-u-escape", b'["\\U0041"]'),
    ("short-u-escape", b'["\\u12"]'),
    ("non-hex-u-escape", b'["\\u12G4"]'),
    ("raw-tab-in-string", b'["a\tb"]'),
    ("raw-nul-in-string", b'["a\x00b"]'),
    ("byte-order-mark", b'\xef\xbb\xbf{"a": 1}'),
    ("form-feed-whitespace", b"[1,\x0c2]"),
    ("no-break-space-whitespace", b"[1,\xc2\xa02]"),
    ("lone-continuation-byte", b'["a\x80b"]'),
    ("overlong-two-byte", b'["\xc0\xaf"]'),
    ("overlong-three-byte", b'["\xe0\x80\xaf"]'),
    ("utf8-surrogate", b'["\xed\xa0\x80"]'),
    ("above-max-code-point", b'["\xf4\x90\x80\x80"]'),
    ("byte-ff", b'["\xff"]'),
    ("truncated-sequence", b'["\xe2\x82"]'),
    ("truncated-at-end", b'["\xf0\x9f\x98'),
    ("deep-unclosed", b"[" * MAX_DEPTH),
    ("valid-then-garbage", b'{"a": [1, 2, 3]}x'),
]


def edge_cases(rng: random.Random):
    every_escape = b'["\\"", "\\\\", "\\/", "\\b", "\\f", "\\n", "\\r", "\\t", "\\u0041", "\\u00e9", "\\u00E9", "\\uFFFF", "\\u0000", "\\u001f", "\\u007F", "\\u0020", "\\u002f", "\\u005C", "\\u0022"]'
    yield "every-escape", ["$[*]", "$", "$[0:5]", "$[-1]"], every_escape

    utf8 = ["\u0080", "\u07ff", "\u0800", "\u0fff", "\u1000", "\ucfff", "\ud000", "\ud7ff", "\ue000", "\uffff",
            "\U00010000", "\U0003ffff", "\U00040000", "\U000fffff", "\U00100000", "\U0010ffff", "\x7f", "é", "中文", "😀"]
    yield "raw-utf8-boundaries", ["$[*]", "$"], document(utf8)

    yield "surrogate-pairs", ['$["\\ud83d\\ude00"]', '$["\\uD83D\\uDE00"]', '$..["\\ud834\\udd1e"]', "$.*", "$"], raw(
        '{"😀": "raw key", "\\ud83d\\ude00": "escaped key", "\\uD834\\uDD1E": "\\ud834\\udd1e", "x": ["𝄞", "\\udbff\\udfff"]}'
    )

    yield "lone-surrogates", [
        '$["\\ud800"]', '$["\\udc00"]', '$["\\ud800\\ud800"]', '$["\\udc00\\ud800"]', "$.*", '$["x\\udfff"]', '$["\\ud800"][0]',
    ], raw(
        '{"\\ud800": ["high"], "\\udc00": "low", "\\ud800\\ud800": "two highs", "\\udc00\\ud800": "low then high",',
        ' "x\\udfff": "after x", "e": "\\ud800x\\udbff", "f": "\\ud800\\u0041", "g": "end\\ud83d"}',
    )

    yield "escaped-names", [
        "$.a", '$["a"]', '$["\\u0061"]', '$["a\\"b"]', '$["/"]', '$["\\/"]', '$["\\\\"]', '$["tab\\t"]', '$["\\u0009x"]',
        '$["caf\\u00e9"]', '$["caf\\u00E9"]', "$.cafe",
    ], raw(
        '{"\\u0061": 1, "a\\"b": 2, "\\/": 3, "\\\\": 4, "tab\\t": 5, "\\tx": 6, "café": 7, "caf\\u00e9": 8}'
    )

    yield "empty-and-blank-names", ['$[""]', '$[" "]', '$["  "]', "$.*", '$..[""]'], b'{"": 1, " ": 2, "  ": {"": [3]}}'

    yield "control-characters", ["$[*]"], document(["".join(chr(c) for c in range(32)), "\x7f\x80\x9f", "\u2028\u2029", "\ufeff"])

    yield "integer-extremes", ["$[*]", "$[0]", "$[1]", "$"], b"[9223372036854775807, -9223372036854775808, 0, -0, 1, -1, 10, -10]"

    yield "literals", ["$", "$[*]", "$[0]"], b"[true,false,null]"
    for name, text in (("root-string", b'"just a string"'), ("root-integer", b" -0 "), ("root-null", b"null"), ("root-true", b"\ntrue\n")):
        yield name, ["$", "$.a", '$["a"]', "$[0]", "$[:]", "$.*", "$..*", "$..a", "$..[0]"], text

    yield "whitespace-everywhere", ["$", "$.a[1]", "$..b"], b' \t\r\n{ \t"a"\r\n:\n[ 1 ,\t2\r, 3 ] , "b" : { "b" :\tnull } }\r\n\t '
    yield "no-trailing-newline", ["$", "$.k"], b'{"k":[]}'
    yield "crlf-document", ["$..*"], b'{\r\n  "a": [\r\n    1,\r\n    {"b": "c"}\r\n  ]\r\n}\r\n'

    empties = b'{"o": {}, "a": [], "n": [[], {}, [{}], {"e": []}], "s": ""}'
    yield "empty-containers", [
        "$", "$.o", "$.o.*", "$.a[*]", "$.a[0]", "$.a[-1]", "$.a[:]", "$.n[*]", "$.n[*][*]", "$..*", "$..e", "$.n..[0]", "$.s",
    ], empties
    yield "empty-root-object", ["$", "$.*", "$..*", "$[0]", "$.a"], b"{}"
    yield "empty-root-array", ["$", "$[*]", "$..*", "$[0]", "$[-1]", "$[:]", "$[0:0]"], b"[ ]"

    duplicates = b'{"a": 1, "b": {"a": 3, "a": [3]}, "a": 2, "a": {"a": 4}, "c": [{"a": 5}, {"b": 6, "a": 7}]}'
    yield "duplicate-names", ["$", "$.a", '$["a"]', "$.*", "$..a", "$.b.a", "$.b.*", "$..a..a", "$.c[*].a", "$.a.a"], duplicates
    yield "duplicate-names-equal-values", ["$.k", "$.*", "$", "$..k"], b'{"k": 1, "k": 1, "k": 1, "\\u006b": 1}'

    deep_array = nest(MAX_DEPTH - 1, Obj([("leaf", "bottom")]), "array")
    yield "deep-arrays", [
        "$", "$..leaf", '$..["leaf"]', "$" + "[0]" * 333, "$" + "[-1]" * 248 + "..leaf", "$..[1]", "$..[0]..leaf", "$..*..*..leaf",
    ], document(deep_array)
    deep_object = nest(MAX_DEPTH - 1, Obj([("leaf", "bottom")]), "object")
    yield "deep-objects", ["$", "$" + ".a" * 499, "$..leaf", "$..a..leaf", "$..a..a..a..leaf", "$..b", "$..*..*..*..leaf"], document(deep_object)
    deep_mixed = nest(MAX_DEPTH - 1, Obj([("k", "end"), ("z", -0)]), "mixed")
    yield "deep-mixed", ["$", "$..z", "$..k..z", "$..*..z", "$..[0]..[0]..z", "$" + ".k[0]" * 199 + "..z", "$..[1]"], document(
        deep_mixed, Style(rng, space=0.3)
    )

    order = b'{"b": {"a": "deep first"}, "a": "root member", "c": [{"a": {"a": "nested"}}, {"x": 1}]}'
    yield "descendant-document-order", ["$..a", "$..a.a", "$..*", "$.c..a", "$..a..a", "$..*..a"], order

    trap = b'{"x": {"y": {"z": 1}, "z": 2}, "y": {"z": 3}, "z": 4}'
    yield "child-after-descendant-order", ["$..y.z", "$..*.z", "$..*..z", "$..z", "$..*", "$.*.*", "$..*[*]", "$..*.*"], trap

    dedupe = b'[[[1, [2]], [3]], [[4]]]'
    yield "descendant-dedupe", ["$..*", "$..*..*", "$..[0]", "$..[0]..[0]", "$..[0][0]", "$..*..*..*", "$..[*]..[-1]", "$[0]..*..[0]"], dedupe

    array = b"[10, 11, 12, 13, 14, 15, 16, 17, 18, 19]"
    yield "index-selectors", [
        "$[0]", "$[9]", "$[10]", "$[-1]", "$[-10]", "$[-11]", "$[-0]", "$[9223372036854775807]", "$[-9223372036854775808]",
        "$[3][0]",
    ], array
    yield "slice-selectors", [
        "$[:]", "$[::]", "$[2:5]", "$[5:2]", "$[-3:]", "$[:-3]", "$[-100:100]", "$[::3]", "$[1::4]", "$[1:8:2]", "$[3:3]",
        "$[:0]", "$[9:]", "$[10:]", "$[-1:-2]", "$[0:1:]", "$[::9223372036854775807]", "$[5::9223372036854775807]",
        "$[-9223372036854775808:9223372036854775807]", "$[9223372036854775807:]", "$[:-9223372036854775808]",
        "$[-9223372036854775808::2]", "$[2:-2:3]",
    ], array
    yield "wildcards", ["$.*", "$[*]", "$.o.*", "$.o[*]", "$.a.*", "$.s.*", "$.n[*]", "$.*.*", "$..*", "$..[*]"], (
        b'{"o": {"p": 1, "q": [2]}, "a": [3, {"r": 4}], "s": "str", "n": null}'
    )
    yield "wrong-types", ['$.a[0]', '$.b.x', '$.b["0"]', "$.a.length", "$.b[0:1]", "$.c[0]", "$.c.x", "$.c[:]", "$.d.*", "$.e[*]"], (
        b'{"a": {"0": "zero"}, "b": ["x", "y"], "c": "text", "d": 5, "e": false}'
    )
    yield "name-shorthand-forms", ["$._", "$.__x", "$.A1_b", "$.true", "$.null", "$.false", "$.Z", "$.z9"], (
        b'{"_": 1, "__x": 2, "A1_b": 3, "true": 4, "null": 5, "false": 6, "Z": 7, "z9": 8}'
    )
    yield "names-with-punctuation", [
        '$["a.b"]', '$["a"]', '$["[0]"]', '$["$"]', '$["*"]', '$["\'"]', '$["\\""]', '$[":"]', '$["]"]', '$["a b"]', "$.a.b",
        '$["\\\\"]', '$["{}"]', '$["1"]',
    ], raw('{"a.b": 1, "a": {"b": 2}, "[0]": 3, "$": 4, "*": 5, "\'": 6, "\\"": 7, ":": 8, "]": 9, "a b": 10, "\\\\": 11, "{}": 12, "1": 13}')

    invalid_queries = [
        "", " ", "$ ", " $", "$$", "@", "a", "$a", "$.", "$..", "$...a", "$.[0]", "$..[]", "$[]", "$[", "$[0", "$0]", "$[0]]",
        "$['a']", "$[01]", "$[+1]", "$[1.0]", "$[1e2]", "$[ 0]", "$[0 ]", "$[--1]", "$[-]", "$.1a", "$.a-b", "$.a$",
        '$["a]', '$["a"', '$["\\x"]', '$["\\u12"]', '$["\\U0041"]', '$["a\\"]', '$["a"b"]', "$[9223372036854775808]",
        "$[-9223372036854775809]", "$[::0]", "$[::-1]", "$[1:2:3:4]", "$[1:2:-0]", "$[:9223372036854775808]", "$[*", "$*",
        "$.*.", "$.a[*]x", '$.["a"]', "$..*.", "$[*][", "$.a..", "$[:-]", "$[a]", "$[\"a\",\"b\"]", "$[0,1]", "$[?@.a]",
    ]
    look_alikes = ["$", "$.a", "$..a", "$.a[0]", "$.a[-0]", "$.a[::1]", "$.a[1:2:]", '$["a"]', '$[""]', "$..*", "$.a[*]", "$.*"]
    mixed = [q for pair in zip(invalid_queries, look_alikes * 5) for q in pair] + invalid_queries[len(look_alikes) * 5 :]
    yield "invalid-queries", mixed, b'{"a": [1, 2], "": 3}'

    yield "no-queries", [], b'{"a": 1}'
    yield "no-queries-invalid-document", [], b'{"a": 1'
    yield "invalid-document-invalid-queries", ["$[", "$.a", ""], b"[1, 2"
    yield "root-only", ["$", "$", "$"], b' [1, {"a": [true, null]}, "x"] '

    for name, doc in INVALID_DOCUMENTS:
        yield f"invalid-{name}", ["$", "$..*"], doc


def random_cases(rng: random.Random):
    for i in range(8):
        model = random_model(rng, rng.randint(1, 30), 4)
        style = Style(rng, space=rng.choice((0, 0.2, 0.6)), escape=rng.choice((0, 0.1, 0.4)), upper=0.3)
        yield f"random-small-{i}", random_queries(rng, model, 25), document(model, style), 4000
    for i in range(5):
        model = random_model(rng, rng.randint(300, 1500), 8)
        style = Style(rng, space=rng.choice((0, 0.1)), escape=rng.choice((0, 0.05)), upper=0.2)
        yield f"random-medium-{i}", random_queries(rng, model, 120), document(model, style), 4000
    for i in range(3):
        names = ["k", "k", "k", "j", "m"]
        model = random_model(rng, rng.randint(100, 400), 5, names=names)
        yield f"random-duplicates-{i}", random_queries(rng, model, 60, names=names), document(model, Style(rng, space=0.1)), 4000
    for i in range(3):
        texts = [t for t in TEXTS if t] + ["\ud800\udc00", "\udbff", "x\ud83d", "\U0001f600\U0001f600"]
        model = random_model(rng, rng.randint(100, 300), 4, texts=texts)
        style = Style(rng, space=0.1, escape=0.5, upper=0.5)
        yield f"random-unicode-{i}", random_queries(rng, model, 80), document(model, style), 4000
    for i in range(8):
        model = random_model(rng, rng.randint(5, 60), 4)
        data = bytearray(document(model, Style(rng, space=0.3, escape=0.2)))
        for _ in range(rng.randint(1, 2)):
            at = rng.randrange(len(data))
            choice = rng.random()
            if choice < 0.4:
                data[at] = rng.choice(b'{}[],:"\\ -0123456789eE.tfnu\x00\x1f\x80\xc3\xed\xff')
            elif choice < 0.7:
                del data[at]
            else:
                data.insert(at, rng.choice(b'{}[],:"\\ 0-x'))
        yield f"random-mutated-{i}", random_queries(rng, model, 10), bytes(data), 4000


def record(rng: random.Random, i: int, texts: list[str]) -> Obj:
    tags = [rng.choice(WORDS) for _ in range(rng.randint(0, 5))]
    history = [Obj([("at", rng.randint(0, 10**9)), ("event", rng.choice(WORDS)), ("ok", rng.random() < 0.8)]) for _ in range(rng.randint(0, 4))]
    fields = [
        ("id", i),
        ("name", rng.choice(texts) + " " + rng.choice(WORDS)),
        ("tags", tags),
        ("score", rng.randint(MIN, MAX) if rng.random() < 0.1 else rng.randint(-1000, 100000)),
        ("active", rng.random() < 0.5),
        ("owner", Obj([("name", rng.choice(WORDS)), ("id", rng.randint(0, 10**6)), ("note", None)])),
        ("history", history),
    ]
    if rng.random() < 0.1:
        fields.append(("name", "duplicate " + str(i)))
    return Obj(fields)


INDEX = 1000


def records_model(rng: random.Random, n: int, texts: list[str]) -> Obj:
    items = [record(rng, i, texts) for i in range(n)]
    meta = Obj([("count", n), ("source", "gen"), ("tags", WORDS), ("nested", nest(30, Obj([("id", "deep")]), "mixed"))])
    return Obj([("meta", meta), ("items", items), ("index", Obj((f"k{i}", i) for i in range(0, n, 7)[:INDEX]))])


def records_queries(rng: random.Random, n: int, count: int, full_scans: int) -> list[str]:
    paths = [
        lambda: f"$.items[{rng.randrange(n)}]",
        lambda: f"$.items[{rng.randrange(n)}].name",
        lambda: f"$.items[{-rng.randint(1, n)}].tags[*]",
        lambda: f"$.items[{rng.randrange(n)}].history[-1].event",
        lambda: f'$["items"][{rng.randrange(n)}]["owner"]["id"]',
        lambda: f"$.items[{rng.randrange(n)}]..name",
        lambda: f"$.items[{rng.randrange(n)}]..*",
        lambda: f"$.items[{(start := rng.randrange(n))}:{start + rng.randint(0, 300)}:{rng.randint(1, 5)}].id",
        lambda: f"$.items[{-rng.randint(1, 20)}:].score",
        lambda: f"$.items[{rng.randrange(n)}].tags[{rng.randint(-6, 6)}]",
        lambda: f"$.index.k{7 * rng.randrange(min(INDEX, n // 7))}",
        lambda: "$.meta.*",
        lambda: f"$.items[{rng.randrange(n)}].history[*].at",
    ]
    queries = [rng.choice(paths)() for _ in range(count)]
    scans = ["$..id", "$..event", "$..name", "$.items[*].owner.id", "$..history[0]", "$.items[*].tags[0]", "$..nested..id", "$..note"]
    for _ in range(full_scans):
        queries.insert(rng.randrange(len(queries) + 1), rng.choice(scans))
    return queries


def large_cases(rng: random.Random):
    model = records_model(rng, 2000, TEXTS)
    yield "large-records", records_queries(rng, 2000, 300, 4), document(model, Style(rng, space=0.05, escape=0.02)), 100_000
    model = random_model(rng, 15_000, 12)
    yield "large-random", random_queries(rng, model, 300), document(model, Style(rng, space=0.1, escape=0.1, upper=0.3)), 8000
    texts = [t for t in TEXTS] + ["\ud83d\ude00" * 3, "é" * 20, "\u0001\u0002"]
    model = random_model(rng, 8000, 6, texts=texts)
    yield "large-unicode", random_queries(rng, model, 200), document(model, Style(rng, space=0.0, escape=0.6, upper=0.5)), 8000


def cases():
    write("examples", "store", [
        "$.store.book[*].title", "$..price", "$.store.book[-1]", "$.store.book[:2].author", '$["store"]["bicycle"]',
        "$.store.book[5]", "$.store.book[0]..*", "$.store..[0]",
    ], STORE)
    write("examples", "strings-and-order", ["$..a", "$..a..a", "$.k", '$["caf\\u00e9"]', "$.s", "$.t"], STRINGS)
    write("examples", "invalid-document", ["$", "$.a"], b'{"a": [1, 2.5]}\n')

    rng = random.Random(20261105)
    edges = [(name, queries, doc, None) for name, queries, doc in edge_cases(rng)]
    for number, (name, queries, doc, budget) in enumerate([*edges, *random_cases(rng), *large_cases(rng)], start=1):
        write("hidden", f"{number:03d}-{name}", queries, doc, budget=budget)


UNICODE_TEXTS = TEXTS + ["Grüße aus Köln", "東京都千代田区", "😀😁😂🤖", "naïve café", "Ελληνικά", "שלום", "a\u0000b", "\ud83d"]


def nested_model(rng: random.Random, sections: int, nodes: int) -> Obj:
    """Sections of irregular random trees, with a rare member name planted in a few of them."""
    model = Obj((f"s{i}", random_model(rng, nodes, 8, texts=UNICODE_TEXTS)) for i in range(sections))
    for _ in range(12):
        section = model[rng.randrange(sections)][1]
        planted = Obj([("needle", rng.choice(UNICODE_TEXTS)), ("x", [1, 2, 3])])
        if isinstance(section, Obj):
            section.append(("needle", planted))
        else:
            section.append(planted)
    return model


def nested_queries(rng: random.Random, model: Obj, count: int) -> list[str]:
    def section():
        i = rng.randrange(len(model))
        return f"$.s{i}", model[i][1]

    def path():
        prefix, root = section()
        return prefix + random_query(rng, root, NAMES)[1:]

    def named():
        return section()[0] + name_segment(rng.choice(NAMES), rng, descendant=True)

    def chained():
        first = rng.choice(("..*", "..[0]", "..[*]", name_segment(rng.choice(NAMES), rng, descendant=True)))
        return section()[0] + first + name_segment(rng.choice(NAMES), rng, descendant=True)

    kinds = [path] * 5 + [named] * 6 + [chained] * 3 + [lambda: section()[0]]
    queries = [rng.choice(kinds)() for _ in range(count)]
    for scan in ("$..needle", "$..needle..x", '$.*..["\\ud800"]'):
        queries.insert(rng.randrange(len(queries) + 1), scan)
    return queries


def workloads():
    rng = random.Random(20261106)
    model = records_model(rng, 100_000, TEXTS)
    doc = document(model, Style(rng, space=0.02, escape=0.01))
    write("workloads", "records", records_queries(rng, 100_000, 4000, 3), doc, expect=False)
    model = nested_model(rng, 2000, 600)
    doc = document(model, Style(rng, space=0.05, escape=0.3, upper=0.2))
    write("workloads", "nested", nested_queries(rng, model, 2500), doc, expect=False)


def write_digests():
    """Records the workloads' SHA-256 so a regeneration is checked byte for byte."""
    lines = []
    for path in sorted((ROOT / "workloads").glob("*.in")):
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    (ROOT / "workloads.sha256").write_text("".join(lines))


def main():
    sys.setrecursionlimit(10_000)
    commands = {"cases": cases, "workloads": lambda: (workloads(), write_digests())}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
