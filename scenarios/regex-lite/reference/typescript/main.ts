const ESCAPABLE = "\\.*+?|()[]^$-";
// A text line has at most 100000 characters, so one line adds at most about 300000 DFA states.
const STATE_LIMIT = 10000;
const PAIR = 2 ** 24;

enum Kind {
  Char,
  Epsilon,
  Begin,
  End,
  Match,
}

class Invalid extends Error {}

type Fragment = [entry: number, exit: number];

function code(ch: string): number {
  return ch.charCodeAt(0);
}

// Builds a Thompson NFA while parsing. Every fragment is an [entry, exit] pair whose exit is an
// epsilon node that later gets the edges to whatever follows.
class Parser {
  pos = 0;
  readonly kind: Kind[] = [];
  readonly chars: (Uint8Array | null)[] = [];
  readonly out: number[][] = [];

  constructor(private readonly pattern: Uint8Array) {}

  private node(kind: Kind, chars: Uint8Array | null = null): number {
    this.kind.push(kind);
    this.chars.push(chars);
    this.out.push([]);
    return this.kind.length - 1;
  }

  private link(from: number, ...to: number[]): void {
    this.out[from]!.push(...to);
  }

  private peek(): number {
    return this.pos < this.pattern.length ? this.pattern[this.pos]! : -1;
  }

  private take(): number {
    const c = this.peek();
    if (c < 0) throw new Invalid();
    this.pos++;
    return c;
  }

  private escaped(): number {
    const c = this.take();
    if (!ESCAPABLE.includes(String.fromCharCode(c))) throw new Invalid();
    return c;
  }

  // Parses the whole pattern and returns the start node. The accepting node comes last.
  parse(): number {
    const [entry, exit] = this.alternation();
    if (this.pos !== this.pattern.length) throw new Invalid();
    this.link(exit, this.node(Kind.Match));
    return entry;
  }

  private alternation(): Fragment {
    const entry = this.node(Kind.Epsilon);
    const exit = this.node(Kind.Epsilon);
    for (;;) {
      const [first, last] = this.sequence();
      this.link(entry, first);
      this.link(last, exit);
      if (this.peek() !== code("|")) return [entry, exit];
      this.pos++;
    }
  }

  private sequence(): Fragment {
    const entry = this.node(Kind.Epsilon);
    let last = entry;
    for (let c = this.peek(); c !== -1 && c !== code("|") && c !== code(")"); c = this.peek()) {
      const [first, end] = this.item();
      this.link(last, first);
      last = end;
    }
    return [entry, last];
  }

  private item(): Fragment {
    const [first, last] = this.atom();
    switch (this.peek()) {
      case code("*"): {
        this.pos++;
        const loop = this.node(Kind.Epsilon);
        this.link(loop, first);
        this.link(last, loop);
        return [loop, loop];
      }
      case code("+"):
        this.pos++;
        this.link(last, first);
        return [first, last];
      case code("?"): {
        this.pos++;
        const skip = this.node(Kind.Epsilon);
        this.link(skip, first, last);
        return [skip, last];
      }
      default:
        return [first, last];
    }
  }

  private atom(): Fragment {
    const c = this.take();
    const chars = new Uint8Array(128);
    switch (c) {
      case code("("): {
        const fragment = this.alternation();
        if (this.take() !== code(")")) throw new Invalid();
        return fragment;
      }
      case code("*"):
      case code("+"):
      case code("?"):
        throw new Invalid();
      case code("^"):
        return this.wrap(Kind.Begin);
      case code("$"):
        return this.wrap(Kind.End);
      case code("."):
        chars.fill(1);
        break;
      case code("["):
        this.bracket(chars);
        break;
      case code("\\"):
        chars[this.escaped()] = 1;
        break;
      default:
        chars[c] = 1;
    }
    return this.wrap(Kind.Char, chars);
  }

  private wrap(kind: Kind, chars: Uint8Array | null = null): Fragment {
    const node = this.node(kind, chars);
    const exit = this.node(Kind.Epsilon);
    this.link(node, exit);
    return [node, exit];
  }

  private bracket(chars: Uint8Array): void {
    const negated = this.peek() === code("^");
    if (negated) this.pos++;
    // Each body character as [byte, is an unescaped dash].
    const body: [number, boolean][] = [];
    for (let c = this.take(); c !== code("]"); c = this.take()) {
      body.push(c === code("\\") ? [this.escaped(), false] : [c, c === code("-")]);
    }
    if (body.length === 0) throw new Invalid();
    let start = 0;
    let stop = body.length;
    if (body[0]![1]) {
      chars[code("-")] = 1;
      start = 1;
    }
    if (stop > start && body[stop - 1]![1]) {
      chars[code("-")] = 1;
      stop--;
    }
    for (let i = start; i < stop; ) {
      const [low, dash] = body[i]!;
      if (dash) throw new Invalid();
      const next = body[i + 1];
      if (i + 1 < stop && next![1]) {
        const high = body[i + 2];
        if (i + 2 >= stop || high![1] || low > high![0]) throw new Invalid();
        chars.fill(1, low, high![0] + 1);
        i += 3;
      } else {
        chars[low] = 1;
        i++;
      }
    }
    if (negated) {
      for (let c = 0; c < 128; c++) chars[c] = chars[c]! ^ 1;
    }
  }
}

function setBit(bits: Uint32Array, k: number): void {
  bits[k >>> 5] = bits[k >>> 5]! | (1 << (k & 31));
}

function intersects(a: Uint32Array, b: Uint32Array): boolean {
  for (let w = 0; w < a.length; w++) {
    if (a[w]! & b[w]!) return true;
  }
  return false;
}

// One interned DFA state: a bitset over the kernel and what is cached about it.
interface State {
  readonly bits: Uint32Array;
  // Reverse transitions by character, -1 until computed.
  readonly back: Int32Array;
  // The state after every member consumes a character, or -1.
  next: number;
  // This state intersected with the start closure at an inner position, or -1.
  entry: number;
  readonly accepts: boolean;
  readonly consumes: boolean;
  readonly starts: boolean;
}

// Counts leftmost-longest matches in linear time per line.
//
// The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
// node that consumes a character. A reverse pass records at each position which kernel states
// can still reach acceptance from there. Forward scans start only where a match starts and keep
// only states that can still accept, so each scan stops right after its longest match. The
// forward DFA reads the reverse pass's state at each position as its input symbol.
class Matcher {
  private readonly words: number;
  private readonly follow: Uint32Array[];
  private readonly consumers: number[][];
  // The start closure, indexed by [at line start][at line end].
  private readonly start: Uint32Array[][];
  // Per character: what can accept from the last position when it holds that character.
  private readonly last: Uint32Array[];
  private readonly scratch: Uint32Array;
  private readonly scratch16: Uint16Array;
  private ids = new Map<string, number>();
  private states: State[] = [];
  private forward = new Map<number, number>();
  private lastIds: number[] = [];
  private acceptOnly = 0;
  private alive = new Int32Array(1024);

  constructor(parser: Parser, startNode: number) {
    const { kind, out } = parser;
    const kernel = new Int32Array(kind.length).fill(-1);
    let size = 1;
    kind.forEach((k, v) => {
      if (k === Kind.Char) kernel[v] = size++;
    });
    kernel[kind.length - 1] = 0;
    const words = (size + 31) >>> 5;
    this.words = words;

    const mark = new Uint32Array(kind.length);
    let stamp = 0;
    const closure = (roots: number[], begin: boolean, end: boolean): Uint32Array => {
      stamp++;
      const bits = new Uint32Array(words);
      const stack: number[] = [];
      const visit = (v: number): void => {
        if (mark[v] !== stamp) {
          mark[v] = stamp;
          stack.push(v);
        }
      };
      roots.forEach(visit);
      for (let v = stack.pop(); v !== undefined; v = stack.pop()) {
        const k = kind[v];
        if (k === Kind.Char || k === Kind.Match) {
          setBit(bits, kernel[v]!);
        } else if ((k !== Kind.Begin || begin) && (k !== Kind.End || end)) {
          out[v]!.forEach(visit);
        }
      }
      return bits;
    };

    this.follow = new Array<Uint32Array>(size);
    const followEnd = new Array<Uint32Array>(size);
    this.consumers = Array.from({ length: 128 }, () => []);
    kind.forEach((_, v) => {
      const k = kernel[v]!;
      if (k <= 0) return;
      this.follow[k] = closure(out[v]!, false, false);
      followEnd[k] = closure(out[v]!, false, true);
      const chars = parser.chars[v]!;
      for (let c = 32; c < 127; c++) {
        if (chars[c]) this.consumers[c]!.push(k);
      }
    });
    this.start = [0, 1].map((b) => [0, 1].map((e) => closure([startNode], b === 1, e === 1)));
    this.last = this.consumers.map((list) => {
      const bits = new Uint32Array(words);
      setBit(bits, 0);
      for (const k of list) {
        if (followEnd[k]![0]! & 1) setBit(bits, k);
      }
      return bits;
    });
    this.scratch = new Uint32Array(words);
    this.scratch16 = new Uint16Array(this.scratch.buffer);
    this.reset();
  }

  private reset(): void {
    this.ids = new Map();
    this.states = [];
    this.forward = new Map();
    this.scratch.fill(0);
    setBit(this.scratch, 0);
    this.acceptOnly = this.intern();
    this.lastIds = this.last.map((bits) => {
      this.scratch.set(bits);
      return this.intern();
    });
  }

  // The state holding the bitset in `scratch`, added when it is new.
  private intern(): number {
    const key = String.fromCharCode(...this.scratch16);
    const known = this.ids.get(key);
    if (known !== undefined) return known;
    const bits = this.scratch.slice();
    const id = this.states.length;
    this.states.push({
      bits,
      back: new Int32Array(128).fill(-1),
      next: -1,
      entry: -1,
      accepts: (bits[0]! & 1) !== 0,
      consumes: bits[0]! >>> 1 !== 0 || bits.some((word, w) => w > 0 && word !== 0),
      starts: intersects(this.start[0]![0]!, bits),
    });
    this.ids.set(key, id);
    return id;
  }

  // The state holding the members of both `a` and state `b`.
  private meet(a: Uint32Array, b: number): number {
    const bits = this.states[b]!.bits;
    for (let w = 0; w < this.words; w++) this.scratch[w] = a[w]! & bits[w]!;
    return this.intern();
  }

  // The kernel states that can accept from a position holding c, given those that can accept
  // from the next position.
  private stepBack(state: number, c: number): number {
    this.scratch.fill(0);
    setBit(this.scratch, 0);
    const ahead = this.states[state]!.bits;
    for (const k of this.consumers[c]!) {
      if (intersects(this.follow[k]!, ahead)) setBit(this.scratch, k);
    }
    const next = this.intern();
    this.states[state]!.back[c] = next;
    return next;
  }

  // The state after every member of `state` consumes a character. The character is implied:
  // every state in a pruned set accepts the character at its position.
  private consume(state: number): number {
    const cached = this.states[state]!.next;
    if (cached >= 0) return cached;
    this.scratch.fill(0);
    const bits = this.states[state]!.bits;
    for (let w = 0; w < this.words; w++) {
      for (let rest = w === 0 ? bits[w]! & ~1 : bits[w]!; rest !== 0; rest &= rest - 1) {
        const follow = this.follow[w * 32 + (31 - Math.clz32(rest & -rest))]!;
        for (let v = 0; v < this.words; v++) this.scratch[v] = this.scratch[v]! | follow[v]!;
      }
    }
    const next = this.intern();
    this.states[state]!.next = next;
    return next;
  }

  // The pruned forward state after `state`, at a position whose reverse state is `alive`.
  private step(state: number, alive: number): number {
    const key = state * PAIR + alive;
    const known = this.forward.get(key);
    if (known !== undefined) return known;
    const next = this.meet(this.states[this.consume(state)]!.bits, alive);
    this.forward.set(key, next);
    return next;
  }

  // The forward state at a match candidate s of a line of n characters.
  private entryAt(s: number, n: number, alive: number): number {
    if (s === 0 || s === n) return this.meet(this.start[s === 0 ? 1 : 0]![s === n ? 1 : 0]!, alive);
    const state = this.states[alive]!;
    if (state.entry < 0) state.entry = this.meet(this.start[0]![0]!, alive);
    return state.entry;
  }

  count(input: Uint8Array, from: number, n: number): number {
    if (this.states.length + this.forward.size > STATE_LIMIT) this.reset();
    if (this.alive.length < n + 1) this.alive = new Int32Array(2 * (n + 1));
    const { alive, states } = this;
    alive[n] = this.acceptOnly;
    if (n > 0) {
      let state = this.lastIds[input[from + n - 1]!]!;
      alive[n - 1] = state;
      for (let i = n - 2; i >= 0; i--) {
        const c = input[from + i]!;
        let next = states[state]!.back[c]!;
        if (next < 0) next = this.stepBack(state, c);
        alive[i] = state = next;
      }
    }

    let matches = 0;
    for (let p = 0; p <= n; ) {
      let s = p;
      while (0 < s && s < n && !states[alive[s]!]!.starts) s++;
      let current = this.entryAt(s, n, alive[s]!);
      if (!states[current]!.accepts && !states[current]!.consumes) {
        if (s >= n) break;
        p = s + 1;
        continue;
      }
      let end = states[current]!.accepts ? s : -1;
      let i = s;
      while (states[current]!.consumes) {
        i++;
        current = i === n ? this.acceptOnly : this.step(current, alive[i]!);
        if (states[current]!.accepts) end = i;
      }
      matches++;
      p = end > s ? end : s + 1;
    }
    return matches;
  }
}

async function main(): Promise<void> {
  const input = new Uint8Array(await Bun.stdin.arrayBuffer());
  let newline = input.indexOf(10);
  if (newline < 0) newline = input.length;
  const parser = new Parser(input.subarray(0, newline));
  let start: number;
  try {
    start = parser.parse();
  } catch (error) {
    if (!(error instanceof Invalid)) throw error;
    await Bun.write(Bun.stdout, "invalid\n");
    return;
  }
  const matcher = new Matcher(parser, start);
  const out: number[] = [];
  for (let from = newline + 1; from < input.length; ) {
    const stop = input.indexOf(10, from);
    if (stop < 0) break;
    out.push(matcher.count(input, from, stop - from));
    from = stop + 1;
  }
  if (out.length > 0) {
    await Bun.write(Bun.stdout, out.join("\n") + "\n");
  }
}

await main();
