"""Writes the cases (examples/ and hidden/) or the workloads/ for bigint-calc.

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
LIMIT_DIGITS = 200_000
MAX_LINE = 1_000_000

# Binding strength of a rendered expression, as in the spec's grammar.
SUM, PRODUCT, UNARY, POWER, ATOM = 1, 2, 3, 4, 5


class Failed(Exception):
    """The evaluation of a generated expression stops with this error message."""


def write(directory: str, name: str, lines: list[str], expect: bool = True):
    data = "".join(line + "\n" for line in lines).encode("ascii")
    assert all(len(line) <= MAX_LINE for line in lines), name
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(data)
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], input=data, capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def digits(rng: random.Random, n: int) -> str:
    """A random n-digit number with no leading zero."""
    return rng.choice("123456789") + "".join(rng.choices("0123456789", k=n - 1))


def number(rng: random.Random, n: int) -> int:
    return int(digits(rng, n))


def carry_digits(rng: random.Random, n: int) -> str:
    """An n-digit number built from runs of 9s, 0s and limb-sized blocks, where carries and
    borrows travel far."""
    blocks = ["9" * 9, "0" * 9, "999999999999999999", "000000001", "500000000", "4294967295", "18446744073709551615"]
    parts = []
    while sum(map(len, parts)) < n:
        parts.append(rng.choice(blocks) if rng.random() < 0.7 else digits(rng, rng.randint(1, 30)))
    text = "".join(parts)[:n]
    return ("9" + text[1:]) if text[0] == "0" else text


# Expressions are (text, binding, value) triples, rendered with the fewest parentheses the
# grammar needs, so the text parses back to the same tree. A value is an int, or a Failed
# error when evaluation stops inside the expression.


def lit(value: int, zeros: int = 0) -> tuple:
    node = ("0" * zeros + str(abs(value)), ATOM, abs(value))
    return neg(node) if value < 0 else node


def group(node: tuple, need: int) -> str:
    text, binding, _ = node
    return text if binding >= need else f"({text})"


def paren(node: tuple) -> tuple:
    return (f"({node[0]})", ATOM, node[2])


def neg(node: tuple) -> tuple:
    value = node[2]
    return ("-" + group(node, UNARY), UNARY, value if isinstance(value, Failed) else -value)


def power(base: int, exponent: int) -> int:
    if exponent < 0:
        raise Failed("negative exponent")
    if base in (0, 1, -1):
        return (1 if exponent == 0 else 0) if base == 0 else (-1 if base == -1 and exponent & 1 else 1)
    return base**exponent


def apply(op: str, a: int, b: int) -> int:
    if op in "/%" and b == 0:
        raise Failed("division by zero")
    match op:
        case "+":
            return a + b
        case "-":
            return a - b
        case "*":
            return a * b
        case "/":
            return a // b
        case "%":
            return a % b
    return power(a, b)


def binary(op: str, left: tuple, right: tuple, space: str = "") -> tuple:
    if op == "^":
        text, binding = group(left, ATOM) + space + "^" + space + group(right, UNARY), POWER
    else:
        binding = SUM if op in "+-" else PRODUCT
        text = group(left, binding) + space + op + space + group(right, binding + 1)
    a, b = left[2], right[2]
    if isinstance(a, Failed):
        return text, binding, a
    if isinstance(b, Failed):
        return text, binding, b
    try:
        return text, binding, apply(op, a, b)
    except Failed as failed:
        return text, binding, failed


def spacing(rng: random.Random) -> str:
    return rng.choices(["", " ", "  ", "\t", " \t "], weights=[50, 40, 4, 3, 3])[0]


def random_expr(rng: random.Random, leaves: int, size: int, errors: bool = False) -> tuple:
    """A random expression tree over `leaves` numbers of up to `size` digits, using every
    operator. Without `errors`, operands are redrawn until no operator fails."""
    if leaves == 1:
        node = lit(int(digits(rng, rng.randint(1, size))) if rng.random() < 0.9 else rng.randint(0, 9), zeros=rng.choice([0] * 20 + [1, 3]))
        return neg(node) if rng.random() < 0.15 else node
    split = rng.randint(1, leaves - 1)
    left = random_expr(rng, split, size, errors)
    for _ in range(20):
        right = random_expr(rng, leaves - split, size, errors)
        op = rng.choices("+-*/%", weights=[3, 3, 3, 2, 2])[0]
        node = binary(op, left, right, spacing(rng))
        if errors or not isinstance(node[2], Failed):
            break
    else:
        node = binary("+", left, lit(1), spacing(rng))
    if rng.random() < 0.12 and not isinstance(node[2], Failed) and abs(node[2]) < 10**60:
        node = binary("^", node, lit(rng.randint(0, 4)), spacing(rng))
    if rng.random() < 0.08:
        node = neg(node)
    if rng.random() < 0.05:
        node = paren(node)
    return node


def text(node: tuple) -> str:
    return node[0]


def checked(node: tuple) -> str:
    """The text of an expression whose every value stays within the spec's bounds."""
    value = node[2]
    assert isinstance(value, Failed) or len(str(abs(value))) <= LIMIT_DIGITS
    return node[0]


# Divisions where Knuth's Algorithm D, run in base B, estimates a quotient limb one too large
# after its two-limb test and must add the divisor back, or must clamp an estimate of B.
# Different implementations pick different bases, so each base gets its own operands.
ADD_BACK_BASES = {
    "1e9": 10**9,
    "2-32": 2**32,
    "2-64": 2**64,
    "other": None,
}
OTHER_BASES = [10**4, 10**8, 10**18, 10**19, 2**16, 2**30, 2**31, 10]


def add_back(rng: random.Random, base: int, limbs: int, extra: int) -> tuple[int, int]:
    """Returns (u, v) such that the first quotient limb of u / v in base `base` is estimated
    one too large: v = (top, second, 0, ..., 0, 1) is normalized, and u = (q + 1) v - 1 shifted
    up by `extra` limbs with random low limbs, so u's top limbs equal (q + 1) times v's."""
    top = rng.randint(base // 2, base - 1)
    second = rng.randint(0, base - 1)
    v = (top * base + second) * base ** (limbs - 2) + 1
    q = rng.randint(1, base - 2)
    u = ((q + 1) * v - 1) * base**extra + rng.randint(0, base**extra - 1)
    return u, v


def clamp(rng: random.Random, base: int, limbs: int, extra: int) -> tuple[int, int]:
    """Returns (u, v) whose first quotient limb in base `base` is base - 1 while the top limbs of
    u and v are equal, so the first estimate is base or more and must be clamped."""
    v = rng.randint(base // 2, base - 1) * base ** (limbs - 1) + rng.randint(0, base ** (limbs - 1) - 1)
    u = (v * base - rng.randint(1, min(v, base))) * base**extra + rng.randint(0, base**extra - 1)
    return u, v


def division_lines(pairs: list[tuple[int, int]], rng: random.Random) -> list[str]:
    lines = []
    for u, v in pairs:
        su, sv = rng.choice([1, -1]), rng.choice([1, -1])
        for op in "/%":
            lines.append(text(binary(op, lit(su * u), lit(sv * v), " ")))
    return lines


def edge_cases(rng: random.Random):
    nines = "9" * LIMIT_DIGITS
    yield "001-single-numbers", ["0", "7", "-0", "00000", "-000", "123456789", "1000000000", "999999999999999999", "-42", "  42", "42  ", "\t42\t", "- 7"]
    yield "002-leading-zeros", ["007", "0000000000000000000001", "-00012 + 0013", "000 * 5", "0" * 5000 + "1", "0" * 9 + "1" + "0" * 9, "10 - 010", "0^000"]
    yield "003-whitespace", ["1\t+\t2", "( 1 + 2 ) * 3", " -  5", "1+2", "1 +2", "--  5", "\t \t2\t*\t \t3\t \t", "( ( 4 ) )", "2 ^ 3 ^ 2", "  - ( 3 )  -  - 3  "]
    yield "004-binary-precedence", ["2+3*4", "2*3+4", "(2+3)*4", "2*(3+4)", "10-4-3", "10-(4-3)", "100/10/5", "100/(10/5)", "7%4%3", "7%(4%3)", "2*3%4", "2+3%2", "20/3*3", "20*3/7", "1-2*3+4/2-5%3", "2*3^2", "(2*3)^2", "2^3*2"]
    yield "005-unary-minus", ["-7/2", "-(7/2)", "-7%2", "-(7%2)", "-2^2", "(-2)^2", "-2^3", "(-2)^3", "2*-3", "-2*-3", "2--3", "2---3", "----7", "-(-(-5))", "-5^2", "5 - -5", "- 0 - - 0", "-(1-3)*-(2-7)", "-3*-3^2"]
    yield "006-power-grouping", ["2^3^2", "(2^3)^2", "2^2^2^2", "3^2^0", "0^0^0", "0^0^0^0", "2^0", "0^0", "0^5", "1^0", "10^18", "(-3)^3", "(-3)^4", "-3^4", "2^10^0^5", "(2^2)^(2^2)"]
    yield "007-power-base-zero-or-one", [
        "1^" + "9" * 60,
        "0^" + "1" + "0" * 100,
        "(-1)^" + "8" * 99,
        "(-1)^" + "9" * 99,
        "-1^" + "9" * 99,
        "0^" + "0" * 1000,
        "1^" + nines,
        "(-1)^(10^199999 + 1)",
        "(-1)^(10^199999 * 9 + (10^199999 - 2))",
        "0^(10^199999)",
        "(0-0)^(1-1)",
        "(2-3)^(10^1000+1)",
        "(-1)^(2^64)",
        "(-1)^(2^64+1)",
    ]
    yield "008-negative-exponent", ["2^-1", "0^-1", "1^-1", "(-1)^-1", "2^-0", "5^(3-4)", "2^-(1)", "(2^-1)", "2^--1", "2^-3^2", "-2^-2", "2^-" + "9" * 40, "0^-0", "1^-(10^199999)", "0^-(2^64)"]
    yield "009-division-small-signs", [f"{a} {op} {b}" for a in ("7", "-7", "6", "-6", "1", "-1", "0", "-0") for b in ("2", "-2", "3", "-3", "5", "-5", "1", "-1") for op in "/%"]
    yield "010-division-by-zero", ["1/0", "0/0", "1%0", "0%0", "5/(2-2)", "5%-0", "5/-0", "(1/0)", "-(1/0)", "10^20/(3*0)", "2^(1/0)", "1/0/0", "1/(0)", "(10^50)%(10^50-10^50)", "3/(2%2)", "5/(4/5)"]
    yield "011-first-error-wins", ["1/0 + 2^-1", "2^-1 + 1/0", "(1/0)^-1", "2^-(1/0)", "2^-1/0", "(2^-1)%0", "1%0*2^-1", "-(2^-1) - 1/0", "0^-1/0", "(5-5)^-(1%0)", "2^(3-4)^(1/0)", "1 + 2 + 3/0 + 4^-1"]
    yield "012-syntax-errors", ["", " ", "\t", "+1", "1+", "1 2", "()", "(1", "1)", ")1(", "(1))", "((1)", "2(3)", "(2)(3)", "(2)3", "2**3", "2//3", "1e5", "0x1F", "1.5", "1,000", "1_000", "abc", "1 + a", "#", "2^", "^2", "*2", "/2", "%2", "-", "--", "-()", "1 - ", "1 ++ 2", "1 + + 2", "2 3 +", "=", "1=1", "[1]", "{1}", "1 000", "- -", "(-)", "2^^3", "2%%3", "(1)+", "+", ")", "(", "~1", "1!", "'1'", '"1"', "1;", "2 ^ * 3", "12 34 56"]
    yield "013-syntax-error-before-evaluation", ["1/0 +", "1/0)", "(1/0", "2^-1 +", "9^9^9^9 )", "9^9^9^9^9 +", "1/0 2", "(9^999999999)(1)", "-9^99999999999999 *", "2^-1 1", "(1/0", "1/0 + 9^9^9^9 ("]
    yield "014-stop-at-first-error", ["1/0 + 9^9^9^9", "2^-1 * 9^9^9^9", "(1/0) * (999^999^999)", "1 % 0 - 7^7^7^7", "(7-7)^-1 + 99999^99999^99999", "-(2^-5) / (8^8^8^8 % 0)", "1/0 + (10^199999)^(10^199999)", "2^-1 - (10^199999 - 1)^(10^199999)^(10^199999)"]
    yield "015-addition-carries", ["999999999 + 1", "999999999999999999 + 1", "1 + 999999999999999999999999999", "10^27 - 1 + 1", "(10^900 - 1) + 1", "1 + (10^9000 - 1)", "999999999 + 999999999", "(10^90-1) + (10^90-1)", "-999999999 - 1", "-(10^900-1) - 1", "18446744073709551615 + 1", "4294967295 + 1"]
    yield "016-subtraction-borrows", ["1000000000 - 1", "10^18 - 1", "10^900 - 1", "1 - 10^900", "10^900 - 10^899", "-(10^50) + 1", "123456789123456789 - 123456789123456789", "1000000000000000000 - 999999999999999999", "10^9 - 10^9 - 1", "(10^99) - (10^99 - 1)", "0 - 1", "-1 + 1", "10^1000 - 10^999 - 10^998"]
    yield "017-multiplication-carries", [f"(10^{k}-1)*(10^{k}-1)" for k in (1, 8, 9, 10, 17, 18, 19, 27, 36, 100, 360, 361, 999, 4000)] + ["999999999*999999999", "4294967295*4294967295", "18446744073709551615*18446744073709551615", "(10^9000-1)^2", "(10^3600-1)*(10^7200-1)"]
    yield "018-multiplication-signs", ["0*-5", "-0*5", "-1*-1", "-3*0", "-5*-5*-5", "-(10^30)*(10^30)", "(-10^30)*(-10^30)", "-2*-3*-4*-5", "-0*-0", "0*(10^1000)", "-(10^1000)*0"]
    yield "019-word-size-overflow", ["9223372036854775807 + 1", "-9223372036854775808 - 1", "9223372036854775807 * 2", "-9223372036854775808 / -1", "-9223372036854775808 % -1", "18446744073709551615 + 1", "4294967295 * 4294967297", "2147483647 + 1", "-2147483648 / -1", "2^63", "2^64", "2^127 - 1", "-(2^128)", "(2^64) / 3", "(2^64+1) % (2^32+1)", "340282366920938463463374607431768211456 / 18446744073709551616"]
    yield "020-limb-boundaries", [f"{a} {op} {b}" for a, b in [("1000000000", "999999999"), ("10^18", "10^9"), ("10^18+1", "10^9+1"), ("10^27", "10^9-1"), ("2^32", "2^32-1"), ("2^64", "2^64-1"), ("10^19", "10^19-1")] for op in "+-*/%"]
    yield "021-max-values", [
        nines,
        "-(10^199999) * 9 - (10^199999 - 1)",
        "((10^199999 - 1) * 10 + 9) % 1000000007",
        "(10^199999) % 7",
        "(-(10^199999) * 9 - (10^199999 - 1)) / (10^199999 - 1)",
    ]
    yield "022-long-leading-zeros", ["0" * 100_000 + "1 + 1" + "0", "-" + "0" * 50_000, "0" * 1000 + "/" + "0" * 1000 + "1"]
    yield "023-big-powers", ["2^664385", "-2^664385 % 1000000007", "(-2)^664385 % 1000000007", "3^419180 % (10^30 + 57)", "7^236000 / 7^235990", "999999999^22222 % 10^30", "(10^9)^22222 / 10^199990", "(-7)^236000 % 999999999989", "2^664385 % 999999999989", "2^664385 / 3^419180"]
    yield "024-deep-parentheses", ["(" * 60_000 + "7" + ")" * 60_000, "(" * 1000 + "1" + ")" * 999, "(" * 999 + "1" + ")" * 1000, "(1+" * 20_000 + "1" + ")" * 20_000]
    yield "025-long-unary-chains", ["-" * (MAX_LINE - 1) + "7", "2^" + "-" * 50_000 + "3", "1" + "-" * 50_001 + "1"]
    yield "026-long-power-chains", ["1^" * 50_000 + "1", "0" + "^0" * 40_000, "0" + "^0" * 39_999, "(-1)^" * 10_000 + "3", "2^" * 6 + "0", "2^2^2^2^2"]
    yield "027-right-nested", ["1-(" * 30_000 + "1" + ")" * 30_000, "2*(" * 10_000 + "1" + ")" * 10_000, "-(" * 30_000 + "5" + ")" * 30_000]
    yield "028-long-flat-chains", ["+".join(["1"] * 50_000), "-".join(["1"] * 30_000), "*".join(["2"] * 20_000), "%".join(["7"] + ["10"] * 20_000)]
    yield "029-factorials", ["*".join(map(str, range(1, n + 1))) for n in (1, 10, 20, 21, 100, 1000, 3000)]
    yield "030-empty-input", []
    yield "031-blank-and-error-lines", ["", "1", "", "", "  ", "2", "\t", "x"]


def generated_cases(rng: random.Random):
    for name, base in ADD_BACK_BASES.items():
        pairs = []
        for _ in range(12):
            b = base or rng.choice(OTHER_BASES)
            pairs.append(add_back(rng, b, rng.randint(3, 6), rng.randint(0, 4)))
            pairs.append(clamp(rng, b, rng.randint(2, 6), rng.randint(0, 4)))
        yield f"knuth-{name}", division_lines(pairs, rng)
    yield "big-divisions-near-equal", [text(binary(op, lit(sa * a), lit(sb * b), " ")) for b in [number(rng, 500)] for a in (b - 1, b, b + 1, 2 * b - 1, 2 * b, 2 * b + 1, b * b - 1) for sa in (1, -1) for sb in (1, -1) for op in "/%"]
    yield "big-divisions-single-limb", [
        text(binary(op, lit(s * number(rng, n)), lit(d), " "))
        for d in (1, -1, 2, -7, 999_999_999, 10**9, -(10**9 + 1), 2**32 - 1, 2**32, 2**64 - 1, -(2**64 + 1))
        for n, s in ((30, 1), (1000, -1))
        for op in "/%"
    ]
    yield "carry-heavy-products", [text(binary("*", lit(int(carry_digits(rng, a))), lit(int(carry_digits(rng, b))))) for a, b in [(9, 9), (18, 18), (100, 100), (361, 359), (1000, 1000), (5000, 4000), (20000, 300), (15000, 15000)]]
    yield "karatsuba-sizes", [text(binary("*", lit(number(rng, a)), lit(number(rng, b)))) for a, b in [(1, 1), (9, 9), (10, 10), (300, 300), (350, 370), (360, 361), (700, 1500), (2000, 2000), (3000, 6001), (6000, 12500), (9999, 10001), (20_000, 20_000), (1, 60_000)]]
    yield "huge-products", [
        "((3^209590 + 1) * (7^117000 - 1)) % (10^40 + 9)",
        "((3^209590 + 1) * (7^117000 - 1) - 3^209590 * 7^117000 - 7^117000 + 3^209590) % 1000000007",
        "(-(2^332192 - 1) * (5^143067 + 3)) % (2^127 - 1)",
        "((10^99999 - 1) * (10^99999 + 1)) % (10^50 + 151)",
        "((10^60000 + 7) * (3^150000 + 11)) / (10^60000 + 7) - 3^150000",
        "(7^40000 * 11^60000) % (2^521 - 1)",
    ]
    yield "big-sums", [text(binary(op, lit(int(carry_digits(rng, a))), lit(int(carry_digits(rng, b))), " ")) for a, b in [(100, 100), (1000, 999), (10_000, 10_000), (25_000, 3)] for op in "+-"]
    for i, (a, b, count) in enumerate([(100, 30, 6), (4000, 2000, 2), (20_000, 10_000, 1)]):
        yield f"big-divisions-{i + 1}", division_lines([(number(rng, a), number(rng, b)) for _ in range(count)], rng)
    yield "huge-divisions", [
        "(3^417000 + 12345) / (7^117000 + 1)",
        "((3^417000 + 12345) % (7^117000 + 1)) % (10^40 + 9)",
        "(-(2^600000 + 1) / (3^50000 - 2)) % (10^40 + 9)",
        "((10^180000 + 7) % -(5^100000 + 3)) % (10^40 + 9)",
        "(-(7^200000) / (2^3000 + 1)) % -(10^40 + 9)",
        "(-(7^200000) % (2^3000 + 1))",
        "((11^150000) / (13^1000 + 1)) % (2^127 - 1)",
    ]
    yield "modular-powers", ["(3^200000) % 1000000007", "(7^100000) % (10^30+7)", "-(5^150000) % (2^89-1)", "(2^100003 - 1) % (2^61 - 1)", "(12345^30000) / (12345^29990)"]
    yield "pow-then-divide", ["(2^500000) / (3^300000) % (10^40 + 9)", "(3^400000) % (7^200000) % (10^40 + 9)", "-(2^600000) / (10^100000 + 1) % (10^40 + 9)", "((10^50000 + 1)^3) / (10^50000 + 1) - 10^100000", "(99^50000) % 98", "(2^100000)^3 / 2^299990"]


def random_cases(rng: random.Random):
    for i in range(8):
        yield f"random-tiny-{i + 1}", [text(random_expr(rng, rng.randint(1, 8), 2, errors=True)) for _ in range(rng.randint(20, 80))]
    for i in range(8):
        yield f"random-small-{i + 1}", [checked(random_expr(rng, rng.randint(1, 15), 20)) for _ in range(rng.randint(50, 300))]
    for i in range(6):
        yield f"random-medium-{i + 1}", [checked(random_expr(rng, rng.randint(1, 12), 300)) for _ in range(rng.randint(10, 30))]
    for i in range(3):
        yield f"random-large-{i + 1}", [checked(random_expr(rng, rng.randint(2, 6), 4000)) for _ in range(rng.randint(2, 5))]
    yield "many-small-lines", [checked(random_expr(rng, rng.randint(1, 6), 12)) for _ in range(5_000)]


def cases():
    rng = random.Random(20261105)
    for stale in [*(ROOT / "examples").glob("*.*"), *(ROOT / "hidden").glob("*.*")]:
        stale.unlink()
    write("examples", "precedence", ["1 + 2 * 3", "-2^2", "(-2)^2", "2^3^2", "-7 / 2", "-7 % 2", "7 % -2", "--5 - -5"])
    write("examples", "errors-and-big-numbers", ["2^100", "123456789012345678901234567890 * -987654321", "1 / (3 - 3)", "2 ^ -1", "1 2", "1/0 +", "0^0", "  0007\t- 7"])
    write("examples", "floor-division", ["100 / 7", "-100 / 7", "100 / -7", "-100 / -7", "100 % 7", "-100 % 7", "100 % -7", "-100 % -7", "(10^30 + 5) / -(10^15)", "(10^30 + 5) % -(10^15)", "-1 / 1000000000000000000000"])

    numbered = [*edge_cases(rng), *generated_cases(rng), *random_cases(rng)]
    for i, (name, lines) in enumerate(numbered):
        stem = name if name[:3].isdigit() else f"{i + 1:03d}-{name}"
        write("hidden", stem, lines)


def workloads():
    rng = random.Random(20261106)
    write("workloads", "small-expressions", [checked(random_expr(rng, rng.randint(2, 14), 30)) for _ in range(100_000)], expect=False)
    lines = []
    for _ in range(12):
        lines.append(f"{digits(rng, rng.randint(80_000, 100_000))} * {digits(rng, rng.randint(80_000, 100_000))}")
        lines.append(f"({digits(rng, 40_000)} + {digits(rng, 40_000)}) * ({digits(rng, 60_000)} - {digits(rng, 50_000)})")
        lines.append(f"{rng.randint(2, 10**6)} ^ {rng.randint(20_000, 30_000)}")
    write("workloads", "big-products", lines, expect=False)
    lines = []
    for _ in range(4):
        lines.append(f"{digits(rng, 150_000)} / {digits(rng, rng.randint(50_000, 100_000))}")
    for _ in range(6):
        lines.append(f"-{digits(rng, 120_000)} % {digits(rng, rng.randint(1_000, 20_000))}")
        lines.append(f"({digits(rng, 30)} ^ {rng.randint(2_000, 6_000)}) / -{digits(rng, rng.randint(10, 20_000))}")
    write("workloads", "big-divisions", lines, expect=False)


def write_digests():
    """Records the workloads' SHA-256 so a regeneration is checked byte for byte."""
    lines = []
    for path in sorted((ROOT / "workloads").glob("*.in")):
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    (ROOT / "workloads.sha256").write_text("".join(lines))


def main():
    sys.set_int_max_str_digits(0)
    sys.setrecursionlimit(10_000)
    commands = {"cases": cases, "workloads": lambda: (workloads(), write_digests())}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
