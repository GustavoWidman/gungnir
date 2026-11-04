import re
import sys

MIN, MAX = -(1 << 63), (1 << 63) - 1
STACK_LIMIT = 1_000_000
CALL_LIMIT = 100_000
LOCALS = 8
MEMORY = 1 << 20

(PUSH, DROP, DUP, SWAP, OVER, ADD, SUB, MUL, DIV, MOD, EQ, NE, LT, LE, GT, GE,
 JMP, JZ, JNZ, CALL, RET, GET, SET, LOAD, STORE, READ, PRINT, HALT) = range(28)

# Each mnemonic's opcode and the kind of operand it takes, if any.
INSTRUCTIONS = {
    "push": (PUSH, "integer"), "drop": (DROP, None), "dup": (DUP, None),
    "swap": (SWAP, None), "over": (OVER, None), "add": (ADD, None), "sub": (SUB, None),
    "mul": (MUL, None), "div": (DIV, None), "mod": (MOD, None), "eq": (EQ, None),
    "ne": (NE, None), "lt": (LT, None), "le": (LE, None), "gt": (GT, None), "ge": (GE, None),
    "jmp": (JMP, "label"), "jz": (JZ, "label"), "jnz": (JNZ, "label"), "call": (CALL, "label"),
    "ret": (RET, None), "get": (GET, "local"), "set": (SET, "local"), "load": (LOAD, None),
    "store": (STORE, None), "read": (READ, None), "print": (PRINT, None), "halt": (HALT, None),
}
LABEL = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
INTEGER = re.compile(r"-?[0-9]+")
TOKEN_SPLIT = re.compile(r"[ \t]+")


class VMError(Exception):
    def __init__(self, message: str, line: int):
        super().__init__(message)
        self.message = message
        self.line = line


def integer(token: str) -> int | None:
    if not INTEGER.fullmatch(token):
        return None
    if len(token.lstrip("-").lstrip("0")) > 19:
        return None
    value = int(token)
    return value if MIN <= value <= MAX else None


def assemble(source: list[str]) -> tuple[list[int], list[int], list[int]]:
    """Turns source lines into parallel opcode, operand and line lists with labels resolved
    to instruction indices, or -1 for a label the program never defines."""
    ops, args, lines, targets = [], [], [], []
    labels: dict[str, int] = {}
    for number, text in enumerate(source, 1):
        tokens = [t for t in TOKEN_SPLIT.split(text.partition(";")[0]) if t]
        while tokens and tokens[0].endswith(":"):
            name = tokens.pop(0)[:-1]
            if not LABEL.fullmatch(name):
                raise VMError("bad label", number)
            if name in labels:
                raise VMError("duplicate label", number)
            labels[name] = len(ops)
        if not tokens:
            continue
        found = INSTRUCTIONS.get(tokens[0].lower())
        if found is None:
            raise VMError("unknown instruction", number)
        op, kind = found
        operands = tokens[1:]
        if len(operands) != (kind is not None):
            raise VMError("bad operand", number)
        arg, target = 0, None
        if kind == "integer":
            arg = integer(operands[0])
            if arg is None:
                raise VMError("bad operand", number)
        elif kind == "local":
            arg = integer(operands[0])
            if arg is None or not 0 <= arg < LOCALS:
                raise VMError("bad operand", number)
        elif kind == "label":
            if not LABEL.fullmatch(operands[0]):
                raise VMError("bad operand", number)
            target = operands[0]
        ops.append(op)
        args.append(arg)
        lines.append(number)
        targets.append(target)
    for i, target in enumerate(targets):
        if target is not None:
            args[i] = labels.get(target, -1)
    # Running off the end halts, and a label after the last instruction points here.
    ops.append(HALT)
    args.append(0)
    lines.append(0)
    return ops, args, lines


def wrap(value: int) -> int:
    return ((value - MIN) & 0xFFFFFFFFFFFFFFFF) + MIN


def execute(ops: list[int], args: list[int], data: list[int], out: list[str]) -> int:
    """Runs the program, appending printed lines to out. Returns the index of the instruction
    that halted it, raising VMError(message, index) on a runtime error."""
    stack: list[int] = []
    push, pop = stack.append, stack.pop
    calls: list[int] = []
    frames = [0] * (LOCALS * (CALL_LIMIT + 1))
    memory = [0] * MEMORY
    base = 0
    cursor = 0
    pc = 0
    while True:
        op = ops[pc]
        arg = args[pc]
        pc += 1
        if op == GET:
            if len(stack) >= STACK_LIMIT:
                raise VMError("stack overflow", pc - 1)
            push(frames[base + arg])
        elif op == PUSH:
            if len(stack) >= STACK_LIMIT:
                raise VMError("stack overflow", pc - 1)
            push(arg)
        elif op == SET:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            frames[base + arg] = pop()
        elif op <= GE and op >= ADD:
            if len(stack) < 2:
                raise VMError("stack underflow", pc - 1)
            b = pop()
            a = stack[-1]
            if op == ADD:
                v = a + b
                stack[-1] = v if MIN <= v <= MAX else wrap(v)
            elif op == SUB:
                v = a - b
                stack[-1] = v if MIN <= v <= MAX else wrap(v)
            elif op == LT:
                stack[-1] = 1 if a < b else 0
            elif op == MUL:
                v = a * b
                stack[-1] = v if MIN <= v <= MAX else wrap(v)
            elif op == EQ:
                stack[-1] = 1 if a == b else 0
            elif op == GT:
                stack[-1] = 1 if a > b else 0
            elif op == LE:
                stack[-1] = 1 if a <= b else 0
            elif op == GE:
                stack[-1] = 1 if a >= b else 0
            elif op == NE:
                stack[-1] = 1 if a != b else 0
            else:
                if b == 0:
                    raise VMError("division by zero", pc - 1)
                # Truncate toward zero, so the remainder takes the dividend's sign.
                q = abs(a) // abs(b)
                if (a < 0) != (b < 0):
                    q = -q
                stack[-1] = wrap(q) if op == DIV else a - b * q
        elif op == JZ:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            if pop() == 0:
                if arg < 0:
                    raise VMError("undefined label", pc - 1)
                pc = arg
        elif op == JNZ:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            if pop() != 0:
                if arg < 0:
                    raise VMError("undefined label", pc - 1)
                pc = arg
        elif op == JMP:
            if arg < 0:
                raise VMError("undefined label", pc - 1)
            pc = arg
        elif op == DUP:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            if len(stack) >= STACK_LIMIT:
                raise VMError("stack overflow", pc - 1)
            push(stack[-1])
        elif op == LOAD:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            address = stack[-1]
            if not 0 <= address < MEMORY:
                raise VMError("bad address", pc - 1)
            stack[-1] = memory[address]
        elif op == STORE:
            if len(stack) < 2:
                raise VMError("stack underflow", pc - 1)
            address = pop()
            if not 0 <= address < MEMORY:
                raise VMError("bad address", pc - 1)
            memory[address] = pop()
        elif op == CALL:
            if arg < 0:
                raise VMError("undefined label", pc - 1)
            if len(calls) >= CALL_LIMIT:
                raise VMError("call stack overflow", pc - 1)
            calls.append(pc)
            pc = arg
            base += LOCALS
            frames[base : base + LOCALS] = [0] * LOCALS
        elif op == RET:
            if not calls:
                raise VMError("return without call", pc - 1)
            pc = calls.pop()
            base -= LOCALS
        elif op == SWAP:
            if len(stack) < 2:
                raise VMError("stack underflow", pc - 1)
            stack[-1], stack[-2] = stack[-2], stack[-1]
        elif op == OVER:
            if len(stack) < 2:
                raise VMError("stack underflow", pc - 1)
            if len(stack) >= STACK_LIMIT:
                raise VMError("stack overflow", pc - 1)
            push(stack[-2])
        elif op == DROP:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            pop()
        elif op == PRINT:
            if not stack:
                raise VMError("stack underflow", pc - 1)
            out.append(str(pop()))
        elif op == READ:
            if cursor == len(data):
                raise VMError("end of input", pc - 1)
            if len(stack) >= STACK_LIMIT:
                raise VMError("stack overflow", pc - 1)
            push(data[cursor])
            cursor += 1
        else:
            return pc - 1


def main() -> None:
    text = sys.stdin.buffer.read().decode("ascii")
    lines = text.split("\n")
    if lines[-1] == "":
        lines.pop()
    source, data = lines, []
    if ".input" in lines:
        at = lines.index(".input")
        source, data = lines[:at], [int(t) for t in "\n".join(lines[at + 1 :]).split()]
    out: list[str] = []
    try:
        ops, args, numbers = assemble(source)
        try:
            execute(ops, args, data, out)
        except VMError as error:
            raise VMError(error.message, numbers[error.line]) from None
    except VMError as error:
        out.append(f"error: {error.message} at line {error.line}")
    if out:
        sys.stdout.write("\n".join(out) + "\n")


if __name__ == "__main__":
    main()
