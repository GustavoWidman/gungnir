const STACK_LIMIT = 1_000_000;
const CALL_LIMIT = 100_000;
const LOCALS = 8;
const MEMORY = 1 << 20;
const MIN = -(1n << 63n);
const MAX = (1n << 63n) - 1n;

const enum Op {
  Push, Drop, Dup, Swap, Over, Add, Sub, Mul, Div, Mod, Eq, Ne, Lt, Le, Gt, Ge,
  Jmp, Jz, Jnz, Call, Ret, Get, Set, Load, Store, Read, Print, Halt,
}

type Operand = "integer" | "local" | "label" | null;

// Each mnemonic's opcode and the kind of operand it takes, if any.
const INSTRUCTIONS = new Map<string, [Op, Operand]>([
  ["push", [Op.Push, "integer"]], ["drop", [Op.Drop, null]], ["dup", [Op.Dup, null]],
  ["swap", [Op.Swap, null]], ["over", [Op.Over, null]], ["add", [Op.Add, null]],
  ["sub", [Op.Sub, null]], ["mul", [Op.Mul, null]], ["div", [Op.Div, null]],
  ["mod", [Op.Mod, null]], ["eq", [Op.Eq, null]], ["ne", [Op.Ne, null]], ["lt", [Op.Lt, null]],
  ["le", [Op.Le, null]], ["gt", [Op.Gt, null]], ["ge", [Op.Ge, null]],
  ["jmp", [Op.Jmp, "label"]], ["jz", [Op.Jz, "label"]], ["jnz", [Op.Jnz, "label"]],
  ["call", [Op.Call, "label"]], ["ret", [Op.Ret, null]], ["get", [Op.Get, "local"]],
  ["set", [Op.Set, "local"]], ["load", [Op.Load, null]], ["store", [Op.Store, null]],
  ["read", [Op.Read, null]], ["print", [Op.Print, null]], ["halt", [Op.Halt, null]],
]);
const LABEL = /^[A-Za-z_][A-Za-z0-9_]*$/;
const INTEGER = /^-?[0-9]+$/;

class VMError extends Error {
  constructor(
    message: string,
    readonly line: number,
  ) {
    super(message);
  }
}

/**
 * The instruction array: opcodes, integer literals for push, and for every other operand an
 * index, a local or a jump target, -1 for a label never defined.
 */
interface Program {
  ops: Uint8Array;
  literals: BigInt64Array;
  indices: Int32Array;
  lines: number[];
}

function integer(token: string): bigint | null {
  if (!INTEGER.test(token)) return null;
  const value = BigInt(token);
  return value >= MIN && value <= MAX ? value : null;
}

function assemble(source: string[]): Program {
  const ops: Op[] = [], literals: bigint[] = [], indices: number[] = [], lines: number[] = [];
  const targets: (string | null)[] = [];
  const labels = new Map<string, number>();
  source.forEach((text, i) => {
    const number = i + 1;
    const semicolon = text.indexOf(";");
    const tokens = (semicolon < 0 ? text : text.slice(0, semicolon))
      .split(/[ \t]+/)
      .filter((t) => t.length > 0);
    let at = 0;
    for (; at < tokens.length && tokens[at]!.endsWith(":"); at++) {
      const name = tokens[at]!.slice(0, -1);
      if (!LABEL.test(name)) throw new VMError("bad label", number);
      if (labels.has(name)) throw new VMError("duplicate label", number);
      labels.set(name, ops.length);
    }
    if (at === tokens.length) return;
    const found = INSTRUCTIONS.get(tokens[at]!.toLowerCase());
    if (found === undefined) throw new VMError("unknown instruction", number);
    const [op, kind] = found;
    const operands = tokens.slice(at + 1);
    if (operands.length !== (kind === null ? 0 : 1)) throw new VMError("bad operand", number);
    let literal = 0n, index = 0, target: string | null = null;
    const operand = operands[0] ?? "";
    if (kind === "integer") {
      const value = integer(operand);
      if (value === null) throw new VMError("bad operand", number);
      literal = value;
    } else if (kind === "local") {
      const value = integer(operand);
      if (value === null || value < 0n || value >= BigInt(LOCALS)) {
        throw new VMError("bad operand", number);
      }
      index = Number(value);
    } else if (kind === "label") {
      if (!LABEL.test(operand)) throw new VMError("bad operand", number);
      target = operand;
    }
    ops.push(op);
    literals.push(literal);
    indices.push(index);
    lines.push(number);
    targets.push(target);
  });
  targets.forEach((target, i) => {
    if (target !== null) indices[i] = labels.get(target) ?? -1;
  });
  // Running off the end halts, and a label after the last instruction points here.
  ops.push(Op.Halt);
  literals.push(0n);
  indices.push(0);
  lines.push(0);
  return {
    ops: Uint8Array.from(ops),
    literals: BigInt64Array.from(literals),
    indices: Int32Array.from(indices),
    lines,
  };
}

/**
 * Runs the program, appending printed lines to out. Throws VMError(message, index) on a
 * runtime error. Values live in BigInt64Arrays, whose stores wrap to signed 64 bits.
 */
function execute({ ops, literals, indices }: Program, data: BigInt64Array, out: string[]): void {
  const stack = new BigInt64Array(STACK_LIMIT);
  const frames = new BigInt64Array(LOCALS * (CALL_LIMIT + 1));
  const memory = new BigInt64Array(MEMORY);
  const calls = new Int32Array(CALL_LIMIT);
  let pc = 0, sp = 0, depth = 0, base = 0, cursor = 0;
  const fail = (message: string): never => {
    throw new VMError(message, pc - 1);
  };
  for (;;) {
    const op = ops[pc]!;
    const index = indices[pc]!;
    pc++;
    switch (op) {
      case Op.Push:
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = literals[pc - 1]!;
        break;
      case Op.Drop:
        if (sp < 1) fail("stack underflow");
        sp--;
        break;
      case Op.Dup:
        if (sp < 1) fail("stack underflow");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp] = stack[sp - 1]!;
        sp++;
        break;
      case Op.Swap: {
        if (sp < 2) fail("stack underflow");
        const top = stack[sp - 1]!;
        stack[sp - 1] = stack[sp - 2]!;
        stack[sp - 2] = top;
        break;
      }
      case Op.Over:
        if (sp < 2) fail("stack underflow");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp] = stack[sp - 2]!;
        sp++;
        break;
      case Op.Add:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! + stack[sp]!;
        break;
      case Op.Sub:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! - stack[sp]!;
        break;
      case Op.Mul:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! * stack[sp]!;
        break;
      case Op.Div:
      case Op.Mod: {
        if (sp < 2) fail("stack underflow");
        const b = stack[sp - 1]!;
        if (b === 0n) fail("division by zero");
        sp--;
        // BigInt division truncates toward zero, and MIN / -1 wraps back to MIN on the store.
        stack[sp - 1] = op === Op.Div ? stack[sp - 1]! / b : stack[sp - 1]! % b;
        break;
      }
      case Op.Eq:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] === stack[sp] ? 1n : 0n;
        break;
      case Op.Ne:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] !== stack[sp] ? 1n : 0n;
        break;
      case Op.Lt:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! < stack[sp]! ? 1n : 0n;
        break;
      case Op.Le:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! <= stack[sp]! ? 1n : 0n;
        break;
      case Op.Gt:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! > stack[sp]! ? 1n : 0n;
        break;
      case Op.Ge:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1]! >= stack[sp]! ? 1n : 0n;
        break;
      case Op.Jmp:
        if (index < 0) fail("undefined label");
        pc = index;
        break;
      case Op.Jz:
        if (sp < 1) fail("stack underflow");
        if (stack[--sp] === 0n) {
          if (index < 0) fail("undefined label");
          pc = index;
        }
        break;
      case Op.Jnz:
        if (sp < 1) fail("stack underflow");
        if (stack[--sp] !== 0n) {
          if (index < 0) fail("undefined label");
          pc = index;
        }
        break;
      case Op.Call:
        if (index < 0) fail("undefined label");
        if (depth === CALL_LIMIT) fail("call stack overflow");
        calls[depth++] = pc;
        base += LOCALS;
        frames.fill(0n, base, base + LOCALS);
        pc = index;
        break;
      case Op.Ret:
        if (depth === 0) fail("return without call");
        pc = calls[--depth]!;
        base -= LOCALS;
        break;
      case Op.Get:
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = frames[base + index]!;
        break;
      case Op.Set:
        if (sp < 1) fail("stack underflow");
        frames[base + index] = stack[--sp]!;
        break;
      case Op.Load: {
        if (sp < 1) fail("stack underflow");
        const address = stack[sp - 1]!;
        if (address < 0n || address >= MEMORY) fail("bad address");
        stack[sp - 1] = memory[Number(address)]!;
        break;
      }
      case Op.Store: {
        if (sp < 2) fail("stack underflow");
        const address = stack[sp - 1]!;
        if (address < 0n || address >= MEMORY) fail("bad address");
        memory[Number(address)] = stack[sp - 2]!;
        sp -= 2;
        break;
      }
      case Op.Read:
        if (cursor === data.length) fail("end of input");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = data[cursor++]!;
        break;
      case Op.Print:
        if (sp < 1) fail("stack underflow");
        out.push(String(stack[--sp]));
        break;
      default:
        return;
    }
  }
}

async function main(): Promise<void> {
  const lines = (await Bun.stdin.text()).split("\n");
  if (lines.at(-1) === "") lines.pop();
  const at = lines.indexOf(".input");
  const source = at < 0 ? lines : lines.slice(0, at);
  const tokens = at < 0 ? [] : lines.slice(at + 1).join("\n").split(/[ \t\n]+/).filter((t) => t);
  const data = BigInt64Array.from(tokens.map((t) => BigInt(t)));
  const out: string[] = [];
  try {
    const program = assemble(source);
    try {
      execute(program, data, out);
    } catch (error) {
      if (!(error instanceof VMError)) throw error;
      throw new VMError(error.message, program.lines[error.line]!);
    }
  } catch (error) {
    if (!(error instanceof VMError)) throw error;
    out.push(`error: ${error.message} at line ${error.line}`);
  }
  if (out.length > 0) {
    await Bun.write(Bun.stdout, out.join("\n") + "\n");
  }
}

await main();
