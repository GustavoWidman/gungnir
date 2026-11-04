import re
import sys

# A number is a run of digits. Any other byte but a space or tab is a token of its own, so a
# character outside the grammar becomes a token the parser rejects.
TOKEN = re.compile(rb"[0-9]+|[^ \t]")
NEG = "neg"
# Binding strength. `^` and unary minus group from the right, the others from the left.
PRECEDENCE = {"+": 1, "-": 1, "*": 2, "/": 2, "%": 2, NEG: 3, "^": 4}
DIVISION_BY_ZERO = b"division by zero"
NEGATIVE_EXPONENT = b"negative exponent"
SYNTAX_ERROR = b"syntax error"


def postfix(line: bytes) -> list | None:
    """Converts a line to postfix order with the shunting-yard algorithm, or returns None when
    it does not match the grammar. Numbers stay bytes and operators become strings."""
    out = []
    stack = []
    want_operand = True
    for token in TOKEN.findall(line):
        if token.isdigit():
            if not want_operand:
                return None
            out.append(token)
            want_operand = False
            continue
        op = token.decode()
        if want_operand:
            if op == "(":
                stack.append(op)
            elif op == "-":
                stack.append(NEG)
            else:
                return None
        elif op == ")":
            while stack and stack[-1] != "(":
                out.append(stack.pop())
            if not stack:
                return None
            stack.pop()
        elif op in PRECEDENCE:
            rank = PRECEDENCE[op]
            while stack and stack[-1] != "(":
                top = PRECEDENCE[stack[-1]]
                if top < rank or (top == rank and op == "^"):
                    break
                out.append(stack.pop())
            stack.append(op)
            want_operand = True
        else:
            return None
    if want_operand or "(" in stack:
        return None
    out.extend(reversed(stack))
    return out


def power(base: int, exponent: int) -> int:
    # The exponent is unbounded only for these bases, whose powers need no multiplication.
    if base == 0:
        return 1 if exponent == 0 else 0
    if base == 1 or base == -1:
        return -1 if base == -1 and exponent & 1 else 1
    return base**exponent


def evaluate(line: bytes) -> bytes:
    items = postfix(line)
    if items is None:
        return SYNTAX_ERROR
    values = []
    for item in items:
        if type(item) is bytes:
            values.append(int(item.lstrip(b"0") or b"0"))
        elif item == NEG:
            values[-1] = -values[-1]
        else:
            b = values.pop()
            a = values[-1]
            if item == "+":
                values[-1] = a + b
            elif item == "-":
                values[-1] = a - b
            elif item == "*":
                values[-1] = a * b
            elif item == "^":
                if b < 0:
                    return NEGATIVE_EXPONENT
                values[-1] = power(a, b)
            elif b == 0:
                return DIVISION_BY_ZERO
            elif item == "/":
                values[-1] = a // b
            else:
                values[-1] = a % b
    return str(values[0]).encode()


def main() -> None:
    sys.set_int_max_str_digits(0)
    lines = sys.stdin.buffer.read().split(b"\n")
    lines.pop()
    sys.stdout.buffer.write(b"".join(evaluate(line) + b"\n" for line in lines))


if __name__ == "__main__":
    main()
