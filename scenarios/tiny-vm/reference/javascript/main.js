const STACK_LIMIT = 1_000_000;
const CALL_LIMIT = 100_000;
const LOCALS = 8;
const MEMORY = 1 << 20;
const MIN = -(1n << 63n);
const MAX = (1n << 63n) - 1n;

const PUSH = 0, DROP = 1, DUP = 2, SWAP = 3, OVER = 4, ADD = 5, SUB = 6, MUL = 7, DIV = 8,
  MOD = 9, EQ = 10, NE = 11, LT = 12, LE = 13, GT = 14, GE = 15, JMP = 16, JZ = 17, JNZ = 18,
  CALL = 19, RET = 20, GET = 21, SET = 22, LOAD = 23, STORE = 24, READ = 25, PRINT = 26,
  HALT = 27;

// Each mnemonic's opcode and the kind of operand it takes, if any.
const INSTRUCTIONS = new Map([
  ["push", [PUSH, "integer"]], ["drop", [DROP, null]], ["dup", [DUP, null]],
  ["swap", [SWAP, null]], ["over", [OVER, null]], ["add", [ADD, null]], ["sub", [SUB, null]],
  ["mul", [MUL, null]], ["div", [DIV, null]], ["mod", [MOD, null]], ["eq", [EQ, null]],
  ["ne", [NE, null]], ["lt", [LT, null]], ["le", [LE, null]], ["gt", [GT, null]],
  ["ge", [GE, null]], ["jmp", [JMP, "label"]], ["jz", [JZ, "label"]], ["jnz", [JNZ, "label"]],
  ["call", [CALL, "label"]], ["ret", [RET, null]], ["get", [GET, "local"]],
  ["set", [SET, "local"]], ["load", [LOAD, null]], ["store", [STORE, null]],
  ["read", [READ, null]], ["print", [PRINT, null]], ["halt", [HALT, null]],
]);
const LABEL = /^[A-Za-z_][A-Za-z0-9_]*$/;
const INTEGER = /^-?[0-9]+$/;

class VMError extends Error {
  constructor(message, line) {
    super(message);
    this.line = line;
  }
}

function integer(token) {
  if (!INTEGER.test(token)) return null;
  const value = BigInt(token);
  return value >= MIN && value <= MAX ? value : null;
}

/**
 * Turns source lines into an instruction array: opcodes, integer literals for push, and for
 * every other operand an index, a local or a jump target, -1 for a label never defined.
 */
function assemble(source) {
  const ops = [], literals = [], indices = [], lines = [], targets = [];
  const labels = new Map();
  source.forEach((text, i) => {
    const number = i + 1;
    const semicolon = text.indexOf(";");
    const tokens = (semicolon < 0 ? text : text.slice(0, semicolon))
      .split(/[ \t]+/)
      .filter((t) => t.length > 0);
    let at = 0;
    while (at < tokens.length && tokens[at].endsWith(":")) {
      const name = tokens[at++].slice(0, -1);
      if (!LABEL.test(name)) throw new VMError("bad label", number);
      if (labels.has(name)) throw new VMError("duplicate label", number);
      labels.set(name, ops.length);
    }
    if (at === tokens.length) return;
    const found = INSTRUCTIONS.get(tokens[at].toLowerCase());
    if (found === undefined) throw new VMError("unknown instruction", number);
    const [op, kind] = found;
    const operands = tokens.slice(at + 1);
    if (operands.length !== (kind === null ? 0 : 1)) throw new VMError("bad operand", number);
    let literal = 0n, index = 0, target = null;
    if (kind === "integer") {
      literal = integer(operands[0]);
      if (literal === null) throw new VMError("bad operand", number);
    } else if (kind === "local") {
      const value = integer(operands[0]);
      if (value === null || value < 0n || value >= BigInt(LOCALS)) {
        throw new VMError("bad operand", number);
      }
      index = Number(value);
    } else if (kind === "label") {
      if (!LABEL.test(operands[0])) throw new VMError("bad operand", number);
      target = operands[0];
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
  ops.push(HALT);
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
function execute({ ops, literals, indices }, data, out) {
  const stack = new BigInt64Array(STACK_LIMIT);
  const frames = new BigInt64Array(LOCALS * (CALL_LIMIT + 1));
  const memory = new BigInt64Array(MEMORY);
  const calls = new Int32Array(CALL_LIMIT);
  let pc = 0, sp = 0, depth = 0, base = 0, cursor = 0;
  const fail = (message) => {
    throw new VMError(message, pc - 1);
  };
  for (;;) {
    const op = ops[pc];
    const index = indices[pc];
    pc++;
    switch (op) {
      case PUSH:
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = literals[pc - 1];
        break;
      case DROP:
        if (sp < 1) fail("stack underflow");
        sp--;
        break;
      case DUP:
        if (sp < 1) fail("stack underflow");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp] = stack[sp - 1];
        sp++;
        break;
      case SWAP: {
        if (sp < 2) fail("stack underflow");
        const top = stack[sp - 1];
        stack[sp - 1] = stack[sp - 2];
        stack[sp - 2] = top;
        break;
      }
      case OVER:
        if (sp < 2) fail("stack underflow");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp] = stack[sp - 2];
        sp++;
        break;
      case ADD:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] += stack[sp];
        break;
      case SUB:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] -= stack[sp];
        break;
      case MUL:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] *= stack[sp];
        break;
      case DIV:
      case MOD: {
        if (sp < 2) fail("stack underflow");
        const b = stack[sp - 1];
        if (b === 0n) fail("division by zero");
        sp--;
        // BigInt division truncates toward zero, and MIN / -1 wraps back to MIN on the store.
        stack[sp - 1] = op === DIV ? stack[sp - 1] / b : stack[sp - 1] % b;
        break;
      }
      case EQ:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] === stack[sp] ? 1n : 0n;
        break;
      case NE:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] !== stack[sp] ? 1n : 0n;
        break;
      case LT:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] < stack[sp] ? 1n : 0n;
        break;
      case LE:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] <= stack[sp] ? 1n : 0n;
        break;
      case GT:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] > stack[sp] ? 1n : 0n;
        break;
      case GE:
        if (sp < 2) fail("stack underflow");
        sp--;
        stack[sp - 1] = stack[sp - 1] >= stack[sp] ? 1n : 0n;
        break;
      case JMP:
        if (index < 0) fail("undefined label");
        pc = index;
        break;
      case JZ:
        if (sp < 1) fail("stack underflow");
        if (stack[--sp] === 0n) {
          if (index < 0) fail("undefined label");
          pc = index;
        }
        break;
      case JNZ:
        if (sp < 1) fail("stack underflow");
        if (stack[--sp] !== 0n) {
          if (index < 0) fail("undefined label");
          pc = index;
        }
        break;
      case CALL:
        if (index < 0) fail("undefined label");
        if (depth === CALL_LIMIT) fail("call stack overflow");
        calls[depth++] = pc;
        base += LOCALS;
        frames.fill(0n, base, base + LOCALS);
        pc = index;
        break;
      case RET:
        if (depth === 0) fail("return without call");
        pc = calls[--depth];
        base -= LOCALS;
        break;
      case GET:
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = frames[base + index];
        break;
      case SET:
        if (sp < 1) fail("stack underflow");
        frames[base + index] = stack[--sp];
        break;
      case LOAD: {
        if (sp < 1) fail("stack underflow");
        const address = stack[sp - 1];
        if (address < 0n || address >= MEMORY) fail("bad address");
        stack[sp - 1] = memory[Number(address)];
        break;
      }
      case STORE: {
        if (sp < 2) fail("stack underflow");
        const address = stack[sp - 1];
        if (address < 0n || address >= MEMORY) fail("bad address");
        memory[Number(address)] = stack[sp - 2];
        sp -= 2;
        break;
      }
      case READ:
        if (cursor === data.length) fail("end of input");
        if (sp === STACK_LIMIT) fail("stack overflow");
        stack[sp++] = data[cursor++];
        break;
      case PRINT:
        if (sp < 1) fail("stack underflow");
        out.push(String(stack[--sp]));
        break;
      default:
        return;
    }
  }
}

async function main() {
  const lines = (await Bun.stdin.text()).split("\n");
  if (lines.at(-1) === "") lines.pop();
  const at = lines.indexOf(".input");
  const source = at < 0 ? lines : lines.slice(0, at);
  const data = BigInt64Array.from(
    at < 0 ? [] : lines.slice(at + 1).join("\n").split(/[ \t\n]+/).filter((t) => t).map(BigInt),
  );
  const out = [];
  try {
    const program = assemble(source);
    try {
      execute(program, data, out);
    } catch (error) {
      if (!(error instanceof VMError)) throw error;
      throw new VMError(error.message, program.lines[error.line]);
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
