// A number is a run of digits. Any other character but a space or tab is a token of its own, so
// a character outside the grammar becomes a token the parser rejects.
const TOKEN = /[0-9]+|[^ \t]/g;
const NEG = "neg";
// Binding strength. `^` and unary minus group from the right, the others from the left.
const PRECEDENCE = new Map([["+", 1], ["-", 1], ["*", 2], ["/", 2], ["%", 2], [NEG, 3], ["^", 4]]);

class Failure extends Error {}

function isDigit(token) {
  return token >= "0" && token <= "9";
}

// Converts a line to postfix order with the shunting-yard algorithm, or returns null when it
// does not match the grammar.
function postfix(line) {
  const out = [];
  const stack = [];
  let wantOperand = true;
  for (const [token] of line.matchAll(TOKEN)) {
    if (isDigit(token[0])) {
      if (!wantOperand) return null;
      out.push(token);
      wantOperand = false;
    } else if (wantOperand) {
      if (token === "(") stack.push(token);
      else if (token === "-") stack.push(NEG);
      else return null;
    } else if (token === ")") {
      while (stack.length > 0 && stack.at(-1) !== "(") out.push(stack.pop());
      if (stack.length === 0) return null;
      stack.pop();
    } else if (PRECEDENCE.has(token)) {
      const rank = PRECEDENCE.get(token);
      while (stack.length > 0 && stack.at(-1) !== "(") {
        const top = PRECEDENCE.get(stack.at(-1));
        if (top < rank || (top === rank && token === "^")) break;
        out.push(stack.pop());
      }
      stack.push(token);
      wantOperand = true;
    } else {
      return null;
    }
  }
  if (wantOperand || stack.includes("(")) return null;
  while (stack.length > 0) out.push(stack.pop());
  return out;
}

function power(base, exponent) {
  if (exponent < 0n) throw new Failure("negative exponent");
  // The exponent is unbounded only for these bases, whose powers need no multiplication.
  if (base === 0n) return exponent === 0n ? 1n : 0n;
  if (base === 1n || base === -1n) return base === -1n && exponent % 2n === 1n ? -1n : 1n;
  return base ** exponent;
}

// BigInt division truncates toward zero; the spec rounds the quotient toward negative infinity.
function divide(a, b) {
  if (b === 0n) throw new Failure("division by zero");
  let q = a / b;
  let r = a % b;
  if (r !== 0n && r < 0n !== b < 0n) {
    q -= 1n;
    r += b;
  }
  return [q, r];
}

function apply(op, a, b) {
  switch (op) {
    case "+":
      return a + b;
    case "-":
      return a - b;
    case "*":
      return a * b;
    case "/":
      return divide(a, b)[0];
    case "%":
      return divide(a, b)[1];
    default:
      return power(a, b);
  }
}

function evaluate(line) {
  const items = postfix(line);
  if (items === null) return "syntax error";
  const values = [];
  try {
    for (const item of items) {
      if (isDigit(item[0])) {
        values.push(BigInt(item));
      } else if (item === NEG) {
        values.push(-values.pop());
      } else {
        const b = values.pop();
        values.push(apply(item, values.pop(), b));
      }
    }
  } catch (error) {
    if (error instanceof Failure) return error.message;
    throw error;
  }
  return values[0].toString();
}

async function main() {
  const lines = (await Bun.stdin.text()).split("\n");
  lines.pop();
  const out = lines.map((line) => evaluate(line) + "\n").join("");
  if (out) {
    await Bun.write(Bun.stdout, out);
  }
}

await main();
