const SCALAR = 0;
const ARRAY = 1;
const OBJECT = 2;
const HEX = "0123456789abcdef";
const MAX_I64 = "9223372036854775807";
const MIN_I64 = "9223372036854775808";
const SHORT = new Map<number, string>([
  [0x22, '\\"'],
  [0x5c, "\\\\"],
  [0x08, "\\b"],
  [0x0c, "\\f"],
  [0x0a, "\\n"],
  [0x0d, "\\r"],
  [0x09, "\\t"],
]);
const ESCAPES = new Map<number, number>([
  [0x22, 0x22],
  [0x5c, 0x5c],
  [0x2f, 0x2f],
  [0x62, 0x08],
  [0x66, 0x0c],
  [0x6e, 0x0a],
  [0x72, 0x0d],
  [0x74, 0x09],
]);

// A name is the member name printed as a string. Printing is one-to-one on code unit
// sequences, so two names are equal exactly when their printed forms are.
type Selector =
  | { type: "name"; name: string }
  | { type: "wild" }
  | { type: "index"; index: number }
  | { type: "slice"; start: number | null; end: number | null; step: number };

interface Segment {
  descendant: boolean;
  selector: Selector;
}

function unitText(unit: number): string {
  const short = SHORT.get(unit);
  if (short !== undefined) return short;
  if (unit >= 0x20 && unit <= 0x7e) return String.fromCharCode(unit);
  return "\\u" + HEX[(unit >> 12) & 15] + HEX[(unit >> 8) & 15] + HEX[(unit >> 4) & 15] + HEX[unit & 15];
}

function isPlain(c: number): boolean {
  return c >= 0x20 && c < 0x7f && c !== 0x22 && c !== 0x5c;
}

// Parses the string literal at src[pos], which must be '"'. src holds one character per input
// byte. Returns [the literal as it prints, the position after the closing quote], or null if
// the literal is invalid.
function parseString(src: string, pos: number): [string, number] | null {
  const open = pos++;
  let plainEnd = pos;
  while (plainEnd < src.length && isPlain(src.charCodeAt(plainEnd))) plainEnd++;
  if (src.charCodeAt(plainEnd) === 0x22) return [src.slice(open, plainEnd + 1), plainEnd + 1];
  const parts = ['"'];
  for (;;) {
    const run = pos;
    while (pos < src.length && isPlain(src.charCodeAt(pos))) pos++;
    if (pos > run) parts.push(src.slice(run, pos));
    if (pos >= src.length) return null;
    const c = src.charCodeAt(pos++);
    if (c === 0x22) break;
    if (c === 0x5c) {
      if (pos >= src.length) return null;
      const e = src.charCodeAt(pos);
      let unit = ESCAPES.get(e);
      if (e === 0x75) {
        const digits = src.slice(pos + 1, pos + 5);
        if (!/^[0-9a-fA-F]{4}$/.test(digits)) return null;
        unit = parseInt(digits, 16);
        pos += 4;
      }
      if (unit === undefined) return null;
      pos++;
      parts.push(unitText(unit));
    } else if (c === 0x7f) {
      parts.push(unitText(c));
    } else if (c < 0x20) {
      return null;
    } else {
      let count: number;
      let low = 0x80;
      let high = 0xbf;
      if (c >= 0xc2 && c <= 0xdf) count = 1;
      else if (c === 0xe0) [count, low] = [2, 0xa0];
      else if ((c >= 0xe1 && c <= 0xec) || c === 0xee || c === 0xef) count = 2;
      else if (c === 0xed) [count, high] = [2, 0x9f];
      else if (c === 0xf0) [count, low] = [3, 0x90];
      else if (c >= 0xf1 && c <= 0xf3) count = 3;
      else if (c === 0xf4) [count, high] = [3, 0x8f];
      else return null;
      if (pos + count > src.length) return null;
      const second = src.charCodeAt(pos);
      if (second < low || second > high) return null;
      let point = c & (0x3f >> count);
      for (let i = pos; i < pos + count; i++) {
        const b = src.charCodeAt(i);
        if (b < 0x80 || b > 0xbf) return null;
        point = (point << 6) | (b & 0x3f);
      }
      pos += count;
      if (point >= 0x10000) {
        parts.push(unitText(0xd800 + ((point - 0x10000) >> 10)), unitText(0xdc00 + ((point - 0x10000) & 0x3ff)));
      } else {
        parts.push(unitText(point));
      }
    }
  }
  parts.push('"');
  return [parts.join(""), pos];
}

function isDigit(c: number): boolean {
  return c >= 0x30 && c <= 0x39;
}

// Parses -?(0|[1-9][0-9]*) at src[pos]; returns [end, whether it fits in 64 bits] or null.
function parseInteger(src: string, pos: number): [number, boolean] | null {
  const negative = src.charCodeAt(pos) === 0x2d;
  if (negative) pos++;
  const first = pos;
  if (src.charCodeAt(pos) === 0x30) pos++;
  else if (isDigit(src.charCodeAt(pos))) while (isDigit(src.charCodeAt(pos))) pos++;
  else return null;
  const digits = src.slice(first, pos);
  const limit = negative ? MIN_I64 : MAX_I64;
  return [pos, digits.length < limit.length || (digits.length === limit.length && digits <= limit)];
}

function grow(field: Int32Array<ArrayBuffer>): Int32Array<ArrayBuffer> {
  const grown = new Int32Array(field.length * 2);
  grown.set(field);
  return grown;
}

// The document's values, stored in document order as parallel arrays indexed by node id, so an
// id orders its node and a node's descendants are exactly the ids from id + 1 up to end[id].
// A scalar keeps its printed text, a container the span of its children in kids, and a member
// of an object its name printed as a string.
class Document {
  size = 0;
  kind = new Int32Array(1024);
  start = new Int32Array(1024);
  length = new Int32Array(1024);
  end = new Int32Array(1024);
  text: string[] = [];
  name: string[] = [];
  kids: number[] = [];

  add(name: string): number {
    if (this.size === this.kind.length) {
      this.kind = grow(this.kind);
      this.start = grow(this.start);
      this.length = grow(this.length);
      this.end = grow(this.end);
    }
    this.text.push("");
    this.name.push(name);
    return this.size++;
  }
}

class Parser {
  pos = 0;
  doc = new Document();
  pending: number[] = [];

  constructor(private readonly src: string) {}

  skipWhitespace(): void {
    const src = this.src;
    let pos = this.pos;
    for (;;) {
      const c = src.charCodeAt(pos);
      if (c !== 0x20 && c !== 0x09 && c !== 0x0a && c !== 0x0d) break;
      pos++;
    }
    this.pos = pos;
  }

  eat(c: number): boolean {
    this.skipWhitespace();
    if (this.src.charCodeAt(this.pos) !== c) return false;
    this.pos++;
    return true;
  }

  string(): string | null {
    const parsed = parseString(this.src, this.pos);
    if (parsed === null) return null;
    this.pos = parsed[1];
    return parsed[0];
  }

  value(name: string): boolean {
    this.skipWhitespace();
    const { doc, src } = this;
    const id = doc.add(name);
    const c = src.charCodeAt(this.pos);
    if (c === 0x5b || c === 0x7b) {
      this.pos++;
      const close = c === 0x5b ? 0x5d : 0x7d;
      const kind = c === 0x5b ? ARRAY : OBJECT;
      const mark = this.pending.length;
      if (!this.eat(close)) {
        for (;;) {
          let member: string | null = "";
          if (kind === OBJECT) {
            this.skipWhitespace();
            if (src.charCodeAt(this.pos) !== 0x22) return false;
            member = this.string();
            if (member === null || !this.eat(0x3a)) return false;
          }
          this.pending.push(doc.size);
          if (!this.value(member)) return false;
          if (this.eat(close)) break;
          if (!this.eat(0x2c)) return false;
        }
      }
      doc.kind[id] = kind;
      doc.start[id] = doc.kids.length;
      doc.length[id] = this.pending.length - mark;
      for (let i = mark; i < this.pending.length; i++) doc.kids.push(this.pending[i]!);
      this.pending.length = mark;
    } else if (c === 0x22) {
      const text = this.string();
      if (text === null) return false;
      doc.text[id] = text;
    } else if (c === 0x2d || isDigit(c)) {
      const integer = parseInteger(src, this.pos);
      if (integer === null || !integer[1]) return false;
      const digits = src.slice(this.pos, integer[0]);
      doc.text[id] = digits === "-0" ? "0" : digits;
      this.pos = integer[0];
    } else {
      const word = ["true", "false", "null"].find((w) => src.startsWith(w, this.pos));
      if (word === undefined) return false;
      doc.text[id] = word;
      this.pos += word.length;
    }
    doc.end[id] = doc.size;
    return true;
  }
}

function parseDocument(src: string): Document | null {
  const parser = new Parser(src);
  if (!parser.value("")) return null;
  parser.skipWhitespace();
  return parser.pos === src.length ? parser.doc : null;
}

// Parses an optional slice bound or index at q[state.pos]; returns null if there is none, or
// undefined if one is malformed. Bounds beyond 2^53 lose precision as numbers, but every such
// bound lies far outside any array, where the rounding cannot change what it selects.
function parseBound(q: string, state: { pos: number }): number | null | undefined {
  const c = q.charCodeAt(state.pos);
  if (c !== 0x2d && !isDigit(c)) return null;
  const integer = parseInteger(q, state.pos);
  if (integer === null || !integer[1]) return undefined;
  const value = Number(q.slice(state.pos, integer[0]));
  state.pos = integer[0];
  return value;
}

// Parses the selector of the bracket that opens at q[pos]; returns [selector, the position
// after ']'], or null if the bracket is invalid.
function parseBracket(q: string, pos: number): [Selector, number] | null {
  pos++;
  let selector: Selector;
  if (q[pos] === "*") {
    selector = { type: "wild" };
    pos++;
  } else if (q[pos] === '"') {
    const parsed = parseString(q, pos);
    if (parsed === null) return null;
    selector = { type: "name", name: parsed[0] };
    pos = parsed[1];
  } else {
    const state = { pos };
    const start = parseBound(q, state);
    if (start === undefined) return null;
    if (q[state.pos] === ":") {
      state.pos++;
      const end = parseBound(q, state);
      if (end === undefined) return null;
      let step = 1;
      if (q[state.pos] === ":") {
        state.pos++;
        const s = parseBound(q, state);
        if (s === undefined || (s !== null && s < 1)) return null;
        if (s !== null) step = s;
      }
      selector = { type: "slice", start, end, step };
    } else {
      if (start === null) return null;
      selector = { type: "index", index: start };
    }
    pos = state.pos;
  }
  return q[pos] === "]" ? [selector, pos + 1] : null;
}

function parseQuery(q: string): Segment[] | null {
  if (q[0] !== "$") return null;
  const segments: Segment[] = [];
  let pos = 1;
  while (pos < q.length) {
    let descendant = false;
    let dotted = false;
    if (q.startsWith("..", pos)) {
      descendant = dotted = true;
      pos += 2;
    } else if (q[pos] === ".") {
      dotted = true;
      pos++;
    }
    let selector: Selector;
    if (q[pos] === "[" && (descendant || !dotted)) {
      const parsed = parseBracket(q, pos);
      if (parsed === null) return null;
      [selector, pos] = parsed;
    } else if (dotted && q[pos] === "*") {
      selector = { type: "wild" };
      pos++;
    } else {
      const name = dotted ? /^[A-Za-z_][A-Za-z0-9_]*/.exec(q.slice(pos)) : null;
      if (name === null) return null;
      selector = { type: "name", name: `"${name[0]}"` };
      pos += name[0].length;
    }
    segments.push({ descendant, selector });
  }
  return segments;
}

// Normalizes a slice to its first position, the bound it stays below, and its step.
function sliceRange(selector: { start: number | null; end: number | null; step: number }, length: number): [number, number, number] {
  const clamp = (bound: number) => Math.min(Math.max(bound < 0 ? bound + length : bound, 0), length);
  return [
    selector.start === null ? 0 : clamp(selector.start),
    selector.end === null ? length : clamp(selector.end),
    selector.step,
  ];
}

function indexPosition(index: number, length: number): number {
  const position = index < 0 ? index + length : index;
  return position >= 0 && position < length ? position : -1;
}

function children(doc: Document, id: number, selector: Selector, out: number[]): void {
  const kind = doc.kind[id];
  if (kind === SCALAR) return;
  const first = doc.start[id]!;
  const count = doc.length[id]!;
  const { kids } = doc;
  switch (selector.type) {
    case "name":
      if (kind === OBJECT) {
        for (let j = first; j < first + count; j++) if (doc.name[kids[j]!] === selector.name) out.push(kids[j]!);
      }
      break;
    case "wild":
      for (let j = first; j < first + count; j++) out.push(kids[j]!);
      break;
    case "index":
      if (kind === ARRAY) {
        const j = indexPosition(selector.index, count);
        if (j >= 0) out.push(kids[first + j]!);
      }
      break;
    case "slice":
      if (kind === ARRAY) {
        const [start, end, step] = sliceRange(selector, count);
        for (let j = start; j < end; j += step) out.push(kids[first + j]!);
      }
      break;
  }
}

// Appends what the selector picks from id and each of its descendants, in document order.
function descend(doc: Document, id: number, selector: Selector, out: number[]): void {
  const kind = doc.kind[id];
  if (kind === SCALAR) return;
  const first = doc.start[id]!;
  const count = doc.length[id]!;
  const { kids } = doc;
  let start = 0;
  let end = 0;
  let step = 1;
  if (kind === ARRAY && selector.type === "index") {
    const j = indexPosition(selector.index, count);
    if (j >= 0) [start, end] = [j, j + 1];
  } else if (kind === ARRAY && selector.type === "slice") {
    [start, end, step] = sliceRange(selector, count);
  }
  for (let j = 0; j < count; j++) {
    const kid = kids[first + j]!;
    let picked: boolean;
    if (selector.type === "name") picked = kind === OBJECT && doc.name[kid] === selector.name;
    else if (selector.type === "wild") picked = true;
    else picked = j >= start && j < end && (j - start) % step === 0;
    if (picked) out.push(kid);
    if (doc.kind[kid] !== SCALAR) descend(doc, kid, selector, out);
  }
}

// The nodes a query selects, each once, in document order. A set holds no node nested in
// another until a descendant segment, and only a nested set can come out unsorted.
function evaluate(doc: Document, segments: Segment[]): number[] {
  let nodes = [0];
  let nested = false;
  for (const { descendant, selector } of segments) {
    const out: number[] = [];
    if (descendant) {
      let covered = 0;
      for (const id of nodes) {
        if (id >= covered) {
          descend(doc, id, selector, out);
          covered = doc.end[id]!;
        }
      }
      nested = true;
    } else {
      for (const id of nodes) children(doc, id, selector, out);
      if (nested) out.sort((a, b) => a - b);
    }
    nodes = out;
  }
  return nodes;
}

function print(doc: Document, id: number, out: string[]): void {
  const kind = doc.kind[id];
  if (kind === SCALAR) {
    out.push(doc.text[id]!);
    return;
  }
  out.push(kind === ARRAY ? "[" : "{");
  const first = doc.start[id]!;
  for (let j = first; j < first + doc.length[id]!; j++) {
    if (j > first) out.push(",");
    const kid = doc.kids[j]!;
    if (kind === OBJECT) out.push(doc.name[kid]!, ":");
    print(doc, kid, out);
  }
  out.push(kind === ARRAY ? "]" : "}");
}

async function main(): Promise<void> {
  // One character per byte, so offsets are byte offsets and a byte's value is its char code.
  const input = Buffer.from(await Bun.stdin.arrayBuffer()).toString("latin1");
  let newline = input.indexOf("\n");
  const count = Number(input.slice(0, newline));
  let pos = newline + 1;
  const queries: string[] = [];
  for (let i = 0; i < count; i++) {
    newline = input.indexOf("\n", pos);
    queries.push(input.slice(pos, newline));
    pos = newline + 1;
  }

  const out: string[] = [];
  const doc = parseDocument(input.slice(pos));
  if (doc === null) {
    out.push("invalid document\n");
  } else {
    for (const query of queries) {
      const segments = parseQuery(query);
      if (segments === null) {
        out.push("invalid query\n");
        continue;
      }
      const nodes = evaluate(doc, segments);
      out.push(`${nodes.length}\n`);
      for (const id of nodes) {
        print(doc, id, out);
        out.push("\n");
      }
    }
  }
  await Bun.write(Bun.stdout, out.join(""));
}

await main();
