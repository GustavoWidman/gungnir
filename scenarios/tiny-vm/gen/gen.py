"""Writes the cases (examples/ and hidden/) or the workloads/ for tiny-vm.

Inputs come from fixed seeds. Expected outputs come from the Python reference. A hand-written
case also states its expected output, and the generator fails when the reference disagrees, so
regenerating re-checks the reference against the spec. The cases are committed; the workloads
are regenerated on demand and never committed.
"""

import hashlib
import random
import subprocess
import sys
import textwrap
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"
MIN, MAX = -(2**63), 2**63 - 1
STACK_LIMIT = 1_000_000
CALL_LIMIT = 100_000
MEMORY = 1 << 20


def wrap(v: int) -> int:
    return (v - MIN) % 2**64 + MIN


def tdiv(a: int, b: int) -> int:
    q = abs(a) // abs(b)
    return wrap(q if (a < 0) == (b < 0) else -q)


def tmod(a: int, b: int) -> int:
    return a - b * tdiv(a, b) if b != -1 else 0


def vm(source: str, data: str | None = None) -> str:
    """A program from an indented source block, with a data section when data is given."""
    text = textwrap.dedent(source).lstrip("\n")
    return text if data is None else text + ".input\n" + data


def lines(*values: int) -> str:
    return "".join(f"{v}\n" for v in values)


def write(directory: str, name: str, text: str, expect: str | None = None, run: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_bytes(text.encode("ascii"))
    if not run:
        return
    out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
    if expect is not None and out.stdout != expect.encode("ascii"):
        sys.exit(f"{path}: the reference printed\n{out.stdout.decode()[:2000]}\nbut the case expects\n{expect[:2000]}")
    path.with_suffix(".out").write_bytes(out.stdout)


def error(message: str, line: int) -> str:
    return f"error: {message} at line {line}\n"


def fill_stack(count: int) -> str:
    """Source that leaves exactly count copies of 7 on an empty stack, using local 0, and never
    holds more than count values while it runs."""
    rounds, rest = divmod(count, 10)
    out = ""
    if rounds > 1:
        # Each round counts down first and pushes ten values last, so it peaks one value above
        # what it leaves, and the last ten values are pushed one by one.
        out += f"        push {rounds - 1}\n        set 0\nfill:   get 0\n        push 1\n        sub\n        set 0\n"
        out += "        push 7\n" * 10 + "        get 0\n        jnz fill\n"
        rest += 10
    else:
        rest = count
    return out + "        push 7\n" * rest


def numbered(source: str, marker: str) -> int:
    """The line number of the first source line containing marker."""
    for number, text in enumerate(source.split("\n"), 1):
        if marker in text:
            return number
    raise ValueError(marker)


# The examples: the two from the spec and one with calls, locals and memory.

SUM_INPUT = vm("""
    ; Sums the data up to the first 0, then prints the sum and how many it added.
            push 0
            set 0           ; local 0 is the sum
    next:   read
            dup
            jz done
            get 0
            add
            set 0
            get 1           ; local 1 is the count
            push 1
            add
            set 1
            jmp next
    done:   get 0
            print
            get 1
            print
""", "5 -3\n10 0 99\n")

WRAP_AND_ERROR = vm("""
    push 9223372036854775807
    push 1
    add
    print
    push -7
    push 2
    div
    print
    PUSH -7
    Push 2
    mod
    print
    drop
""")

RECURSIVE_CALLS = vm("""
    ; Stores the factorials of 0 to 5 in memory, then prints them back in reverse.
            push 0
            set 0                   ; n
    fill:   get 0
            call fact
            get 0
            store                   ; memory[n] = n!
            get 0
            push 1
            add
            dup
            set 0
            push 6
            lt
            jnz fill
    show:   get 0
            push 1
            sub
            dup
            set 0
            load
            print
            get 0
            jnz show
            halt

    ; fact(n) returns n! and keeps n in its own local 0 across the recursive call.
    fact:   set 0
            get 0
            jz one
            get 0
            push 1
            sub
            call fact
            get 0
            mul
            ret
    one:    push 1
            ret
""")


def instruction_cases():
    yield "push-and-print", vm("""
        push 0
        print
        push 42
        print
        push -17
        print
        push 007
        print
        push -0
        print
        push 0000
        print
    """), lines(0, 42, -17, 7, 0, 0)
    yield "drop-and-dup", vm("""
        push 1
        push 2
        drop
        print
        push 3
        push 4
        push 5
        drop
        drop
        print
        push 5
        dup
        mul
        print
        push -3
        dup
        print
        print
    """), lines(1, 3, 25, -3, -3)
    yield "swap-and-over", vm("""
        push 1
        push 2
        swap
        print
        print
        push 10
        push 3
        swap
        sub
        print
        push 1
        push 2
        over
        print
        print
        print
        push 10
        push 3
        over
        sub
        print
        print
    """), lines(1, 2, -7, 1, 2, 1, -7, 10)
    yield "add-sub-mul", vm("""
        push 2
        push 3
        add
        print
        push 2
        push 3
        sub
        print
        push -5
        push -6
        add
        print
        push -5
        push -6
        sub
        print
        push 6
        push 7
        mul
        print
        push -6
        push 7
        mul
        print
        push -6
        push -7
        mul
        print
        push 0
        push -9
        mul
        print
        push 3037000499
        dup
        mul
        print
    """), lines(5, -1, -11, 1, 42, -42, 42, 0, 3037000499**2)
    pairs = [(7, 2), (-7, 2), (7, -2), (-7, -2), (6, 3), (-6, 3), (0, 5), (1, 2), (-1, 2), (2, 7), (-2, 7)]
    for op, fn in (("div", tdiv), ("mod", tmod)):
        source = "".join(f"push {a}\npush {b}\n{op}\nprint\n" for a, b in pairs)
        yield f"{op}-truncates", source, lines(*(fn(a, b) for a, b in pairs))
    pairs = [(1, 2), (2, 1), (2, 2), (-1, 1), (1, -1), (-5, -3), (0, 0)]
    comparisons = {"eq": int.__eq__, "ne": int.__ne__, "lt": int.__lt__, "le": int.__le__, "gt": int.__gt__, "ge": int.__ge__}
    source, expect = "", []
    for a, b in pairs:
        for op, fn in comparisons.items():
            source += f"push {a}\npush {b}\n{op}\nprint\n"
            expect.append(int(fn(a, b)))
    yield "comparisons", source, lines(*expect)
    yield "jmp-forward-and-back", vm("""
                jmp forward
                push 1
                print
        forward:
                push 2
                print
                jmp end
        back:   push 3
                print
                halt
        end:    jmp back
    """), lines(2, 3)
    yield "jz-and-jnz", vm("""
                push 0
                jz a
                push 100
                print
        a:      push 5
                jz b
                push 1
                print
        b:      push -1
                jnz c
                push 200
                print
        c:      push 0
                jnz d
                push 2
                print
        d:      push 7
                push 0
                jz e
        e:      print
    """), lines(1, 2, 7)
    yield "call-and-ret", vm("""
                push 3
                call double
                print
                push -4
                call double
                print
                halt
        double: dup
                add
                ret
    """), lines(6, -8)
    yield "nested-call-return-addresses", vm("""
                call a
                push 4
                print
                halt
        a:      push 1
                print
                call b
                push 3
                print
                ret
        b:      push 2
                print
                ret
    """), lines(1, 2, 3, 4)
    yield "main-frame-locals", vm("""
        get 0
        print
        push 5
        set 0
        push 6
        set 7
        get 0
        get 7
        add
        print
        get 3
        print
        get 07
        print
        push 9
        set -0
        get 0
        print
    """), lines(0, 11, 0, 6, 9)
    yield "locals-per-frame", vm("""
                push 10
                set 0
                call f
                get 0
                print
                call f
                halt
        f:      get 0
                print
                push 99
                set 0
                get 0
                print
                ret
    """), lines(0, 99, 10, 0, 99)
    yield "load-and-store", vm("""
        push 0
        load
        print
        push 42
        push 7
        store
        push 7
        load
        print
        push -5
        push 1048575
        store
        push 1048575
        load
        print
        push 8
        load
        print
        push 7
        load
        print
    """), lines(0, 42, -5, 0, 42)
    yield "read", vm("""
        read
        read
        add
        print
        read
        print
        read
        print
    """, "  1\t2\n\n-9223372036854775808   \n 9223372036854775807\n"), lines(3, MIN, MAX)
    yield "halt-inside-call", vm("""
                push 1
                print
                call f
                push 3
                print
        f:      push 2
                print
                halt
                push 4
                print
    """), lines(1, 2)
    yield "end-of-program-inside-call", vm("""
                call f
                push 9
                print
        f:      push 1
                print
    """), lines(1)
    yield "return-to-end-of-program", vm("""
                jmp main
        f:      push 5
                print
                ret
        main:   call f
    """), lines(5)


def wraparound_cases():
    def binary(*groups):
        """Each group is an operator, its oracle and the (a, b) pairs to apply it to."""
        source = "".join(f"push {a}\npush {b}\n{op}\nprint\n" for op, _, pairs in groups for a, b in pairs)
        return source, lines(*(fn(a, b) for _, fn, pairs in groups for a, b in pairs))

    yield "add-and-sub-wrap", *binary(
        ("add", lambda a, b: wrap(a + b), [(MAX, 1), (MAX, MAX), (MIN, -1), (MIN, MIN), (MIN, MAX), (MAX, -MAX)]),
        ("sub", lambda a, b: wrap(a - b), [(MIN, 1), (0, MIN), (MAX, -1), (-1, MAX), (MIN, MIN), (-2, MAX)]),
    )
    yield "mul-wraps", *binary(("mul", lambda a, b: wrap(a * b), [
        (MAX, 2), (MIN, -1), (MIN, MIN), (MAX, MAX), (3037000500, 3037000500), (1 << 32, 1 << 32),
        ((1 << 32) + 1, (1 << 32) - 1), (-(1 << 62), 4), (6364136223846793005, 1442695040888963407),
        (123456789123, 987654321987),
    ]))
    yield "div-extremes", *binary(("div", tdiv, [(MIN, -1), (MIN, 1), (MIN, 2), (MIN, MIN), (MAX, MIN), (MIN, MAX), (MAX, -1), (MAX, MAX)]))
    yield "mod-extremes", *binary(("mod", tmod, [(MIN, -1), (MIN, 2), (MIN, 3), (MIN, MAX), (MAX, MIN), (MAX, -3), (MIN, -7), (MIN, MIN)]))
    yield "print-extremes", vm("""
        push -9223372036854775808
        print
        push 9223372036854775807
        print
        push -9223372036854775807
        print
        push 9223372036854775806
        print
        push -1
        print
    """), lines(MIN, MAX, MIN + 1, MAX - 1, -1)
    yield "compare-extremes", *binary(
        ("lt", lambda a, b: int(a < b), [(MIN, MAX), (MAX, MIN), (MIN, 1), (-1, MAX), (MAX, -1), (1, MIN), (MIN, MIN)]),
        ("ge", lambda a, b: int(a >= b), [(MIN, MAX), (MAX, MIN), (MIN, 1), (MAX, -1), (1, MIN), (MIN, MIN)]),
    )
    x, expect = 1, []
    for i in range(1, 1001):
        x = wrap(x * 6364136223846793005 + 1442695040888963407)
        if i % 50 == 0:
            expect.append(x)
    yield "lcg-wraps", vm("""
                push 1
                set 0           ; x
                push 1
                set 1           ; i
        step:   get 0
                push 6364136223846793005
                mul
                push 1442695040888963407
                add
                set 0
                get 1
                push 50
                mod
                jnz next
                get 0
                print
        next:   get 1
                push 1
                add
                dup
                set 1
                push 1000
                le
                jnz step
    """), lines(*expect)
    yield "read-extremes", vm("""
        read
        read
        sub
        print
        read
        read
        mul
        print
        read
        print
    """, f"{MIN} 1 {MAX} {MAX} 0\n"), lines(MAX, 1, 0)


def runtime_error_cases():
    yield "underflow-drop-empty", "drop\n", error("stack underflow", 1)
    yield "underflow-jz-empty", "push 1\njnz skip\nskip: jz skip\n", error("stack underflow", 3)
    yield "underflow-store-one-value", "push 1048576\nstore\n", error("stack underflow", 2)
    yield "underflow-load-print-set", "push 3\nset 0\nget 0\nprint\nset 1\n", "3\n" + error("stack underflow", 5)
    yield "underflow-before-division-by-zero", "push 0\ndiv\n", error("stack underflow", 2)
    yield "underflow-before-bad-address", "push -1\nstore\n", error("stack underflow", 2)

    source = fill_stack(STACK_LIMIT - 10) + "        push 1\n" * 9 + "        push 2\n        print\n        push 3\n        push 4\n"
    yield "overflow-at-exact-limit", source, "2\n" + error("stack overflow", source.count("\n"))
    source = fill_stack(STACK_LIMIT) + "        dup\n"
    source = fill_stack(STACK_LIMIT - 1) + "        over\n        print\n        over\n        over\n"
    yield "overflow-over", source, "7\n" + error("stack overflow", source.count("\n"))
    source = fill_stack(STACK_LIMIT) + "        get 0\n"
    source = fill_stack(STACK_LIMIT) + "        read\n"
    yield "overflow-read", vm(source, "5\n"), error("stack overflow", source.count("\n"))
    yield "end-of-input-before-overflow", vm(source, "\n"), error("end of input", source.count("\n"))

    yield "division-by-zero", "push 1\npush 0\ndiv\n", error("division by zero", 3)
    yield "mod-by-zero", "push -9223372036854775808\nprint\npush -9223372036854775808\npush 0\nmod\n", f"{MIN}\n" + error("division by zero", 5)
    yield "bad-address-negative", "push -1\nload\n", error("bad address", 2)
    yield "bad-address-past-end", "push 1\npush 1048575\nstore\npush 1048575\nload\nprint\npush 1\npush 1048576\nstore\n", "1\n" + error("bad address", 9)
    yield "bad-address-extremes", "push 5\npush -9223372036854775808\nstore\n", error("bad address", 3)

    yield "undefined-label-jmp", "push 1\nprint\njmp nowhere\npush 2\nprint\n", "1\n" + error("undefined label", 3)
    yield "undefined-label-jz-taken", "push 0\njz missing\n", error("undefined label", 2)
    yield "undefined-label-not-taken", vm("""
        push 1
        jz missing
        push 0
        jnz missing
        push 3
        print
    """), lines(3)
    yield "undefined-label-never-runs", vm("""
                jmp start
                jmp missing
                call missing_too
        start:  push 4
                print
                halt
                jmp missing
    """), lines(4)
    yield "undefined-label-case", vm("""
                push 1
                jnz Loop
        loop:   push 2
                print
    """), error("undefined label", 2)

    depth_program = vm("""
                read
                call f
                push 1
                print
                halt
        ; f(n) recurses n more levels below itself, then calls g.
        f:      dup
                jz bottom
                push 1
                sub
                call f          ; recurse
                ret
        bottom: call g
                ret
        g:      ret
    """)
    yield "call-depth-at-limit", depth_program + f".input\n{CALL_LIMIT - 2}\n", lines(1)
    yield "call-depth-over-limit", depth_program + f".input\n{CALL_LIMIT - 1}\n", error("call stack overflow", numbered(depth_program, "bottom: call g"))
    yield "undefined-label-checked-before-depth", vm("""
                read
                call f
                halt
        f:      dup
                jz bottom
                push 1
                sub
                call f
                ret
        bottom: call nowhere
    """, f"{CALL_LIMIT - 1}\n"), error("undefined label", 10)
    yield "return-after-returns", vm("""
                call f
                push 2
                print
                ret
        f:      push 1
                print
                ret
    """), lines(1, 2) + error("return without call", 4)
    yield "end-of-input", vm("read\nprint\nread\nprint\nread\nprint\n", "1 2\n"), lines(1, 2) + error("end of input", 5)
    yield "end-of-input-no-data", "read\n", error("end of input", 1)
    source = vm("""
        ; A runtime error reports the line of the instruction that failed, counting every
        ; line of the input: comments, blank lines and label-only lines included.

        start:
                push 6
                call half

                push 7
                call half
                halt

        half:   ; returns n div 2 after checking that n is even
                dup
                push 2
                mod
                jnz odd
                push 2
                div
                print
                ret
        odd:
                push 0
                push 0
                div             ; fails here
    """)
    yield "error-line-counts-every-line", source, lines(3) + error("division by zero", numbered(source, "fails here"))
    source = "".join(f"push {i}\nprint\n" for i in range(5000)) + "push 1\npush 0\nmod\n"
    yield "error-after-many-prints", source, lines(*range(5000)) + error("division by zero", 10003)


def assembly_error_cases():
    yield "unknown-instruction", "push 1\nprint\nfoo\n", error("unknown instruction", 3)
    yield "unknown-instruction-pop", "push 1\npop\n", error("unknown instruction", 2)
    yield "unknown-instruction-label-without-colon", "loop push 1\n", error("unknown instruction", 1)
    yield "bad-operand-missing", "push 1\nprint\npush\n", error("bad operand", 3)
    yield "bad-operand-extra", "push 1\ndup 1\n", error("bad operand", 2)
    yield "bad-operand-plus-sign", "push +5\n", error("bad operand", 1)
    yield "bad-operand-underscore", "push 1_000\n", error("bad operand", 1)
    yield "bad-operand-hex", "push 0x10\n", error("bad operand", 1)
    yield "bad-operand-too-large", "push 9223372036854775807\npush 9223372036854775808\n", error("bad operand", 2)
    yield "bad-operand-too-small", "push -9223372036854775808\npush -9223372036854775809\n", error("bad operand", 2)
    yield "bad-operand-padded-too-large", "push 0000000000000000000009223372036854775807\nprint\npush 000000000000000000009223372036854775808\n", error("bad operand", 3)
    yield "padded-literals-are-valid", vm("""
        push 00000000000000000000000000000000000000000000000009223372036854775807
        print
        push -000000000000000000000000000000009223372036854775808
        print
        push -00000000000000000000000000000000000000000000000000000000000000
        print
    """), lines(MAX, MIN, 0)
    yield "bad-operand-local-too-large", "get 7\nprint\nget 8\n", error("bad operand", 3)
    yield "bad-operand-local-negative", "push 1\nset -1\n", error("bad operand", 2)
    yield "bad-operand-label-digit", "jmp 1abc\n", error("bad operand", 1)
    yield "bad-operand-label-colon", "a: jmp a:\n", error("bad operand", 1)
    yield "bad-label-digit", "push 1\n1st: print\n", error("bad label", 2)
    yield "bad-label-lone-colon", ": push 1\n", error("bad label", 1)
    yield "duplicate-label", "a: push 1\nb: push 2\n\na: push 3\n", error("duplicate label", 4)
    yield "duplicate-label-same-line", "a: a: halt\n", error("duplicate label", 1)
    yield "first-error-wins", "push 1\npush 1.5\nfoo\n1x:\n", error("bad operand", 2)
    yield "label-checked-before-mnemonic", "push 1\n1x: foo 1 2\n", error("bad label", 2)
    yield "mnemonic-checked-before-operands", "frob +1 2 3\n", error("unknown instruction", 1)
    yield "labels-checked-left-to-right", "a: 9: a: halt\n", error("bad label", 1)
    yield "assembly-error-beats-runtime-error", "drop\npush 1\nprint\n\nbogus\n", error("unknown instruction", 5)


def syntax_cases():
    yield "mnemonics-ignore-case", vm("""
                PUSH 2
                Push 3
                ADD
                pRiNt
                PuSh 1
                JNZ Done
                push 99
                print
        Done:   HALT
    """), lines(5)
    yield "labels-respect-case", vm("""
                jmp Loop
        loop:   push 1
                print
                halt
        Loop:   push 2
                print
                jmp LOOP_
        LOOP_:  jmp loop
    """), lines(2, 1)
    yield "labels-spelled-like-mnemonics", vm("""
                jmp print
        push:   push 2
                print
                halt
        print:  push 1
                print
                jmp push
    """), lines(1, 2)
    yield "several-labels-one-instruction", vm("""
                push 0
                set 0
        a: b:   c:  get 0
                push 1
                add
                dup
                set 0
                dup
                print
                dup
                push 1
                eq
                jnz a
                dup
                push 2
                eq
                jnz b
                push 3
                eq
                jnz c
    """), lines(1, 2, 3, 4)
    yield "label-on-its-own-line", vm("""
                jmp target
                push 1
                print
        target:

        ; the label marks the next instruction, even across blank and comment lines

                push 2
                print
    """), lines(2)
    yield "label-at-end-of-program", vm("""
                push 1
                print
                jmp end
                push 2
                print
        end:
        ; nothing follows
    """), lines(1)
    yield "call-label-at-end-of-program", "push 1\nprint\ncall end\npush 2\nprint\nend:\n", lines(1)
    yield "comments", ";;; header\npush 1;no space\nprint ; one ; two\nloop:;label then comment\n; .input\n  ;indented\npush 2 ;\nprint\n", lines(1, 2)
    yield "tabs-and-spaces", "\tpush\t5\t\n  \t print  \t\nx:\tpush    6\nprint\t\t\n\t\t\n   \n", lines(5, 6)
    yield "no-final-line-feed", "push 1\nprint\npush 2\nprint", lines(1, 2)
    yield "data-without-final-line-feed", vm("read\nread\nadd\nprint\n", "40\n2"), lines(42)
    yield "empty-input", "", ""
    yield "only-separator", ".input\n", ""
    yield "separator-at-end-without-line-feed", "push 1\nprint\n.input", lines(1)
    yield "separator-with-trailing-space", "push 1\nprint\n.input \n5\n", error("unknown instruction", 3)
    yield "data-whitespace", vm("read\nread\nread\nadd\nadd\nprint\n", "\n\n   7\t\t-3\n\n\n  100   \n\n"), lines(104)


GCD_PAIRS = [(48, -18), (-48, 18), (-48, -18), (1071, 462), (MIN, -1), (MIN, 6), (7, MIN)]


def gcd_trace(pairs):
    out = []
    for a, b in pairs:
        while b != 0:
            a, b = b, tmod(a, b)
            out.append(b)
        out.append(a)
    return out


def bug_catcher_cases():
    yield "operand-order", vm("""
        push 10
        push 3
        sub
        print
        push 10
        push 3
        div
        print
        push 10
        push 3
        mod
        print
        push 10
        push 3
        lt
        print
        push 10
        push 3
        gt
        print
        push 10
        push 3
        le
        print
        push 10
        push 3
        ge
        print
    """), lines(7, 3, 1, 0, 1, 0, 1)
    yield "jz-pops-when-not-taken", "push 5\njz nowhere\ndrop\n", error("stack underflow", 3)
    yield "callee-frame-starts-zeroed", vm("""
                call f
                call f
                call g
                halt
        f:      get 3
                print
                push 77
                set 3
                ret
        g:      call f
                ret
    """), lines(0, 0, 0)
    sum_squares = vm("""
                read
                call f
                print
                halt
        ; f(n) = n*n + f(n-1), f(0) = 0, keeping n in local 0 across the call
        f:      set 0
                get 0
                jz zero
                get 0
                push 1
                sub
                call f          ; recurse
                get 0
                get 0
                mul
                add
                ret
        zero:   push 0
                ret
    """)
    yield "locals-survive-recursion", sum_squares + ".input\n20\n", lines(sum(i * i for i in range(21)))
    yield "deep-recursion-with-locals", sum_squares + f".input\n{CALL_LIMIT - 1}\n", lines(sum(i * i for i in range(CALL_LIMIT)))
    yield "mutual-recursion", vm("""
                read
                call even
                print
                read
                call even
                print
                halt
        ; even(n) and odd(n) for n >= 0 return 1 or 0
        even:   dup
                jz yes
                push 1
                sub
                call odd
                ret
        odd:    dup
                jz no
                push 1
                sub
                call even
                ret
        yes:    drop
                push 1
                ret
        no:     drop
                push 0
                ret
    """, "4000\n77777\n"), lines(1, 0)
    yield "ackermann", vm("""
                push 2
                push 3
                call ack
                print
                push 3
                push 4
                call ack
                print
                halt
        ; ack(m, n) with m pushed first
        ack:    set 1           ; n
                set 0           ; m
                get 0
                jnz m_pos
                get 1
                push 1
                add
                ret
        m_pos:  get 1
                jnz both
                get 0
                push 1
                sub
                push 1
                call ack
                ret
        both:   get 0
                push 1
                sub
                get 0
                get 1
                push 1
                sub
                call ack
                call ack
                ret
    """), lines(9, 125)
    primes = [p for p in range(2, 1000) if all(p % d for d in range(2, int(p**0.5) + 1))]
    yield "sieve-small", vm("""
                push 1000
                set 0
                push 2
                set 1
        outer:  get 1
                get 0
                ge
                jnz done
                get 1
                load
                jnz next
                get 1
                print
                get 1
                dup
                add
                set 2
        mark:   get 2
                get 0
                lt
                jz next
                push 1
                get 2
                store
                get 2
                get 1
                add
                set 2
                jmp mark
        next:   get 1
                push 1
                add
                set 1
                jmp outer
        done:
    """), lines(*primes)
    yield "gcd-with-negatives", vm("""
        next:   read
                dup
                jz stop
                read
                call gcd
                print
                jmp next
        stop:   halt
        ; gcd(a, b) = b == 0 ? a : gcd(b, a mod b), printing each remainder
        gcd:    dup
                jz done
                swap
                over
                mod
                dup
                print
                call gcd
                ret
        done:   drop
                ret
    """, "".join(f"{a} {b}\n" for a, b in GCD_PAIRS) + "0\n"), lines(*gcd_trace(GCD_PAIRS))
    out = []
    for i in range(1, 20001):
        out.append(-i if i % 3 == 0 else i)
    yield "many-prints", vm("""
                push 1
                set 0
        loop:   get 0
                dup
                push 3
                mod
                jnz keep
                push -1
                mul
        keep:   print
                get 0
                push 1
                add
                dup
                set 0
                push 20000
                le
                jnz loop
    """), lines(*out)
    yield "fib-recursive", vm("""
                push 0
                set 0
        loop:   get 0
                call fib
                print
                get 0
                push 1
                add
                dup
                set 0
                push 20
                le
                jnz loop
                halt
        fib:    set 0
                get 0
                push 2
                lt
                jz recurse
                get 0
                ret
        recurse:
                get 0
                push 1
                sub
                call fib
                get 0
                push 2
                sub
                call fib
                add
                ret
    """), lines(*[0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89, 144, 233, 377, 610, 987, 1597, 2584, 4181, 6765])
    yield "stack-is-shared-across-calls", vm("""
                push 1
                push 2
                push 3
                call f
                print
                print
                halt
        f:      add
                ret
    """), lines(5, 1)


# Random programs: structured so they always stop, with forward jumps, counted loops in the
# main program and calls only to functions defined later, over values chosen to hit the 64-bit
# edges. A static bound on the instructions each program can run keeps every case small.

POOL = [0, 1, -1, 2, -2, 3, 7, 10, 255, 1 << 31, -(1 << 31), 1 << 32, MAX, MIN, MAX - 1, MIN + 1,
        1 << 62, -(1 << 62), 3037000499, 3037000500, -3037000500]
BINARY = ["add", "sub", "mul", "div", "mod", "eq", "ne", "lt", "le", "gt", "ge"]
WEIGHTS = [6, 6, 5, 3, 3, 1, 1, 2, 1, 1, 1]
ADDRESSES = [0, 1, 2, 3, 100, 4095, MEMORY - 1]
RANDOM_BUDGET = 300_000


def value(rng: random.Random) -> int:
    r = rng.random()
    if r < 0.4:
        return rng.choice(POOL)
    if r < 0.75:
        return rng.randint(-100, 100)
    return rng.randint(MIN, MAX)


class Generator:
    """Emits one random program, tracking how often each instruction can run: its loop
    multiplier in the current body, with calls charged the callee's own bound."""

    def __init__(self, rng: random.Random, style: bool, functions: int, risky: float):
        self.rng = rng
        self.style = style
        self.functions = functions
        self.arity = [rng.randint(0, 2) for _ in range(functions)]
        self.risky = risky
        self.out: list[str] = []
        self.pending: list[str] = []
        self.labels = 0
        self.body = -1
        self.multiplier = 1
        self.cost = [0] * (functions + 1)
        self.reads = [0] * (functions + 1)
        self.calls: list[tuple[int, int, int]] = []

    def label(self) -> str:
        self.labels += 1
        return self.rng.choice(["L", "l", "_l", "Lab_", "x"]) + str(self.labels)

    def emit(self, mnemonic: str, operand=None):
        rng = self.rng
        self.cost[self.body] += self.multiplier
        if self.style and rng.random() < 0.2:
            mnemonic = rng.choice([mnemonic.upper(), mnemonic.capitalize()])
        gap = rng.choice([" ", "\t", "   "]) if self.style else " "
        prefix = "".join(f"{name}:{gap}" for name in self.pending)
        self.pending = []
        text = mnemonic if operand is None else f"{mnemonic}{gap}{operand}"
        indent = rng.choice(["", "  ", "\t", "        "]) if self.style else "    "
        line = indent + prefix + text
        if self.style and rng.random() < 0.1:
            line += rng.choice([" ; note", ";x", "\t; .input", " "])
        self.out.append(line)
        if self.style and rng.random() < 0.05:
            self.out.append(rng.choice(["", "; comment", "   ", "\t;"]))

    def leaf(self, scope: int, free: list[int], depth: int):
        rng = self.rng
        r = rng.random()
        if r < 0.45:
            self.emit("push", value(rng))
        elif r < 0.7:
            self.emit("get", rng.choice(free))
        elif r < 0.8:
            self.reads[self.body] += self.multiplier
            self.emit("read")
        elif r < 0.9:
            self.emit("push", rng.choice(ADDRESSES))
            self.emit("load")
        elif scope < self.functions:
            f = rng.randrange(scope, self.functions)
            for _ in range(self.arity[f]):
                self.expression(depth - 1, scope, free)
            self.calls.append((self.body, f, self.multiplier))
            self.emit("call", f"fn{f}")
        else:
            self.emit("push", value(rng))

    def expression(self, depth: int, scope: int, free: list[int]):
        """Code that pushes exactly one value. scope is the lowest function it may call."""
        rng = self.rng
        if depth <= 0 or rng.random() < 0.3:
            self.leaf(scope, free, depth)
            return
        op = rng.choices(BINARY, WEIGHTS)[0]
        shape = rng.random()
        self.expression(depth - 1, scope, free)
        if op in ("div", "mod"):
            # A divisor computed from random values is 0 so often that it would end most
            # programs early, so divisors are nonzero constants and only risky statements
            # divide by zero.
            self.emit("push", rng.choice([v for v in POOL if v != 0] + [rng.randint(1, 50), -rng.randint(1, 50)]))
            self.emit(op)
            return
        if shape < 0.1:
            self.emit("dup")
            self.emit(op)
            return
        self.expression(depth - 1, scope, free)
        if shape < 0.6:
            self.emit(op)
        elif shape < 0.75:
            self.emit("swap")
            self.emit(op)
        elif shape < 0.9:
            self.emit("over")
            self.emit(op)
            self.emit(rng.choices(BINARY[:3] + BINARY[5:], WEIGHTS[:3] + WEIGHTS[5:])[0])
        else:
            self.emit("drop")

    def statement(self, nesting: int, scope: int, free: list[int]):
        rng = self.rng
        r = rng.random()
        in_function = self.body >= 0
        if r < self.risky:
            self.risky_statement(in_function)
        elif r < 0.35:
            self.expression(rng.randint(0, 3), scope, free)
            self.emit("print")
        elif r < 0.55:
            self.expression(rng.randint(0, 3), scope, free)
            self.emit("set", rng.choice(free))
        elif r < 0.65:
            self.expression(rng.randint(0, 2), scope, free)
            self.emit("push", rng.choice(ADDRESSES))
            self.emit("store")
        elif r < 0.8 and nesting < 2:
            skip = self.label()
            self.expression(rng.randint(0, 2), scope, free)
            self.emit(rng.choice(["jz", "jnz"]), skip)
            for _ in range(rng.randint(1, 4)):
                self.statement(nesting + 1, scope, free)
            self.pending.append(skip)
        elif r < 0.9 and nesting < 2 and not in_function:
            counter, inner = free[-1], free[:-1]
            top = self.label()
            count = rng.randint(1, 12)
            self.emit("push", count)
            self.emit("set", counter)
            outer = self.multiplier
            self.multiplier *= count
            self.pending.append(top)
            for _ in range(rng.randint(1, 4)):
                self.statement(nesting + 1, scope, inner)
            self.emit("get", counter)
            self.emit("push", 1)
            self.emit("sub")
            self.emit("dup")
            self.emit("set", counter)
            self.emit("jnz", top)
            self.multiplier = outer
        else:
            self.expression(rng.randint(0, 3), scope, free)
            self.emit("print")

    def risky_statement(self, in_function: bool):
        rng = self.rng
        choice = rng.randrange(7)
        if choice == 0:
            self.emit("drop")
        elif choice == 1:
            self.emit("push", rng.choice([-1, MEMORY, MIN, MAX]))
            self.emit("load")
        elif choice == 2:
            self.emit("jmp", "missing_label")
        elif choice == 3 and not in_function:
            self.emit("ret")
        elif choice == 4:
            self.emit("push", value(rng))
            self.emit("push", 0)
            self.emit(rng.choice(["div", "mod"]))
            self.emit("print")
        elif choice == 5:
            self.emit("halt")
        else:
            self.reads[self.body] += self.multiplier
            self.emit("read")
            self.emit("print")

    def program(self, statements: int) -> str:
        rng = self.rng
        for _ in range(statements):
            self.statement(0, 0, list(range(8)))
        self.emit("halt")
        for f in range(self.functions):
            self.body = f
            self.pending.append(f"fn{f}")
            # Arguments arrive on the stack, and the function keeps them in its own locals.
            for k in reversed(range(self.arity[f])):
                self.emit("set", k)
            for _ in range(rng.randint(1, 4)):
                self.statement(1, f + 1, list(range(8)))
            self.expression(rng.randint(0, 2), f + 1, list(range(8)))
            self.emit("ret")
        self.out += [f"{name}:" for name in self.pending]
        return "\n".join(self.out) + "\n"

    def bound(self, per_body: list[int]) -> int:
        """The most instructions, or reads, the program can run, counting both sides of every
        branch, given the count in each body for one run of it."""
        total = list(per_body)
        for body in [*reversed(range(self.functions)), -1]:
            total[body] += sum(m * total[f] for caller, f, m in self.calls if caller == body)
        return total[-1]


def random_program(rng: random.Random, statements: int, functions: int, style: bool, risky: float) -> str:
    while True:
        gen = Generator(rng, style, functions, risky)
        source = gen.program(statements)
        if gen.bound(gen.cost) <= RANDOM_BUDGET:
            break
    reads = gen.bound(gen.reads)
    count = reads + 5 if rng.random() < 0.8 else rng.randint(0, reads)
    data = " ".join(str(value(rng)) for _ in range(count))
    return source + ".input\n" + data + "\n"


def random_cases(rng: random.Random):
    for _ in range(4):
        yield "random-straight-line", random_program(rng, rng.randint(5, 25), 0, False, 0.01), None
    for _ in range(4):
        yield "random-styled", random_program(rng, rng.randint(10, 40), rng.randint(0, 3), True, 0.01), None
    for _ in range(3):
        yield "random-with-calls", random_program(rng, rng.randint(30, 80), rng.randint(3, 8), True, 0.003), None
    for _ in range(2):
        yield "random-large", random_program(rng, rng.randint(300, 600), rng.randint(8, 16), True, 0.0), None


def cases():
    write("examples", "sum-input", SUM_INPUT, lines(12, 3))
    write("examples", "wrap-and-error", WRAP_AND_ERROR, lines(MIN, -3, -1) + error("stack underflow", 13))
    write("examples", "recursive-calls", RECURSIVE_CALLS, lines(120, 24, 6, 2, 1, 1))

    rng = random.Random(20261104)
    groups = [instruction_cases(), wraparound_cases(), runtime_error_cases(), assembly_error_cases(),
              syntax_cases(), bug_catcher_cases(), random_cases(rng)]
    number = 0
    for group in groups:
        for name, text, expect in group:
            number += 1
            write("hidden", f"{number:03d}-{name}", text, expect)


WORKLOAD_COLLATZ = vm("""
    ; For n from 1 to N: the n with the longest Collatz chain, its length, and a wrapping
    ; hash of every chain length.
            read
            set 0           ; N
            push 1
            set 1           ; n
    outer:  get 1
            get 0
            gt
            jnz done
            get 1
            push 0
            set 5           ; steps
    step:   dup
            push 1
            eq
            jnz finish
            dup
            push 2
            mod
            jz even
            push 3
            mul
            push 1
            add
            jmp counted
    even:   push 2
            div
    counted:
            get 5
            push 1
            add
            set 5
            jmp step
    finish: drop
            get 4
            push 1000003
            mul
            get 5
            add
            set 4
            get 5
            get 2
            gt
            jz next
            get 5
            set 2
            get 1
            set 3
    next:   get 1
            push 1
            add
            set 1
            jmp outer
    done:   get 3
            print
            get 2
            print
            get 4
            print
""")

WORKLOAD_FIB = vm("""
    ; Naive recursive Fibonacci of each number in the data.
    next:   read
            dup
            push 0
            lt
            jnz stop
            call fib
            print
            jmp next
    stop:   halt
    fib:    set 0
            get 0
            push 2
            lt
            jz recurse
            get 0
            ret
    recurse:
            get 0
            push 1
            sub
            call fib
            get 0
            push 2
            sub
            call fib
            add
            ret
""")

WORKLOAD_SIEVE = vm("""
    ; Sieve of Eratosthenes over the numbers below N, marking composites in memory, then the
    ; count of primes and a wrapping hash of them.
            read
            set 0           ; N
            push 2
            set 1           ; i
    outer:  get 1
            dup
            mul
            get 0
            lt
            jz count
            get 1
            load
            jnz advance
            get 1
            dup
            mul
            set 2           ; j = i * i
    mark:   push 1
            get 2
            store
            get 2
            get 1
            add
            dup
            set 2
            get 0
            lt
            jnz mark
    advance:
            get 1
            push 1
            add
            set 1
            jmp outer
    count:  push 2
            set 1
    scan:   get 1
            get 0
            ge
            jnz done
            get 1
            load
            jnz skip
            get 3
            push 1
            add
            set 3
            get 4
            push 31
            mul
            get 1
            add
            set 4
    skip:   get 1
            push 1
            add
            set 1
            jmp scan
    done:   get 3
            print
            get 4
            print
""")


def workloads():
    write("workloads", "collatz", WORKLOAD_COLLATZ + ".input\n11000\n", run=False)
    write("workloads", "fib", WORKLOAD_FIB + ".input\n28 26 -1\n", run=False)
    write("workloads", "sieve", WORKLOAD_SIEVE + ".input\n600000\n", run=False)


def write_digests():
    """Records the workloads' SHA-256 so a regeneration is checked byte for byte."""
    lines_ = []
    for path in sorted((ROOT / "workloads").glob("*.in")):
        lines_.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    (ROOT / "workloads.sha256").write_text("".join(lines_))


def main():
    commands = {"cases": cases, "workloads": lambda: (workloads(), write_digests())}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
