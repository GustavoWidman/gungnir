const NONE = 0xffffffff;

type Hunk = [x: number, a: number, y: number, b: number];

/**
 * Numbers the lines of `doc`, giving equal lines equal numbers. A line keeps its line feed, so
 * a last line without one differs from the same text with one.
 */
function intern(doc: string, ids: Map<string, number>): Int32Array {
  const lines: number[] = [];
  for (let start = 0; start < doc.length; ) {
    const newline = doc.indexOf("\n", start);
    const end = newline < 0 ? doc.length : newline + 1;
    const line = doc.slice(start, end);
    let id = ids.get(line);
    if (id === undefined) {
      id = ids.size;
      ids.set(line, id);
    }
    lines.push(id);
    start = end;
  }
  return Int32Array.from(lines);
}

/** Where round `e` starts in the flat array of rounds: rounds 0 to e - 1 hold 1 to e entries. */
function offset(e: number): number {
  return (e * (e + 1)) / 2;
}

/**
 * Myers' search run from the end of both documents. Entry i of round e is the smallest x on
 * diagonal k = n - m - e + 2i such that (x, x - k) is at most e edits from the end, or NONE when
 * no point of that diagonal is. `cost` is the first round that reaches (0, 0), which is the
 * shortest script's cost.
 */
function searchBack(a: Int32Array, b: Int32Array): { rounds: Uint32Array; cost: number } {
  const n = a.length;
  const m = b.length;
  const kend = n - m;
  let x = n;
  let y = m;
  while (x > 0 && y > 0 && a[x - 1] === b[y - 1]) {
    x--;
    y--;
  }
  let rounds = new Uint32Array(1024);
  rounds[0] = x;
  for (let e = 0; ; e++) {
    const at = e - kend;
    if (at % 2 === 0 && at >= 0 && at <= 2 * e && rounds[offset(e) + at / 2] === 0) {
      return { rounds, cost: e };
    }
    const next = e + 1;
    const prev = offset(e);
    const cur = offset(next);
    if (cur + next + 1 > rounds.length) {
      const grown = new Uint32Array(2 * (cur + next + 1));
      grown.set(rounds);
      rounds = grown;
    }
    rounds.fill(NONE, cur, cur + next + 1);
    // Only the diagonals -m <= k <= n cross the grid.
    const first = Math.max(0, (next - n + 1) >> 1);
    const last = Math.min(next, (m + next) >> 1);
    for (let i = first; i <= last; i++) {
      const k = kend - next + 2 * i;
      const lo = k > 0 ? k : 0;
      let best = Infinity;
      const back = i < next ? rounds[prev + i]! : NONE;
      if (back !== NONE) {
        // Back from diagonal k + 1 over a deletion.
        best = Math.max(back - 1, lo);
      }
      const up = i > 0 ? rounds[prev + i - 1]! : NONE;
      if (up !== NONE) {
        // Back from diagonal k - 1 over an insertion.
        best = Math.min(best, Math.max(up, lo));
      }
      if (best !== Infinity) {
        let sx = best;
        let sy = best - k;
        while (sx > 0 && sy > 0 && a[sx - 1] === b[sy - 1]) {
          sx--;
          sy--;
        }
        rounds[cur + i] = sx;
      }
    }
  }
}

/** The hunks of the first shortest edit script in keep, delete, insert order. */
function diff(a: Int32Array, b: Int32Array): Hunk[] {
  const n = a.length;
  const m = b.length;
  const kend = n - m;
  const { rounds, cost } = searchBack(a, b);
  const keeps = (x: number, y: number): boolean => x < n && y < m && a[x] === b[y];
  const hunks: Hunk[] = [];
  let e = cost;
  let x = 0;
  let y = 0;
  for (;;) {
    while (keeps(x, y)) {
      x++;
      y++;
    }
    if (x === n && y === m) {
      return hunks;
    }
    const x0 = x;
    const y0 = y;
    do {
      // Delete when (x + 1, y) is still within e - 1 edits of the end.
      const i = (x - y + e - kend) / 2;
      if (x < n && i < e && rounds[offset(e - 1) + i]! <= x + 1) {
        x++;
      } else {
        y++;
      }
      e--;
    } while (!keeps(x, y) && !(x === n && y === m));
    hunks.push([x0, x - x0, y0, y - y0]);
  }
}

async function main(): Promise<void> {
  const input = await Bun.stdin.text();
  let pos = input.indexOf("\n") + 1;
  const pairs = Number(input.slice(0, pos - 1));
  const out: string[] = [];
  const ids = new Map<string, number>();
  for (let pair = 0; pair < pairs; pair++) {
    const end = input.indexOf("\n", pos);
    const space = input.indexOf(" ", pos);
    const n = Number(input.slice(pos, space));
    const m = Number(input.slice(space + 1, end));
    const first = input.slice(end + 1, end + 1 + n);
    const second = input.slice(end + 1 + n, end + 1 + n + m);
    pos = end + 1 + n + m;

    ids.clear();
    const hunks = diff(intern(first, ids), intern(second, ids));
    let deleted = 0;
    let inserted = 0;
    for (const [, a, , b] of hunks) {
      deleted += a;
      inserted += b;
    }
    out.push(`${deleted} ${inserted} ${hunks.length}\n`);
    for (const hunk of hunks) {
      out.push(`${hunk.join(" ")}\n`);
    }
  }
  if (out.length > 0) {
    await Bun.write(Bun.stdout, out.join(""));
  }
}

await main();
