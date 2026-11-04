const NONE = 0xffffffff;

/**
 * Numbers the lines of `doc`, giving equal lines equal numbers. A line keeps its line feed, so
 * a last line without one differs from the same text with one.
 */
function intern(doc, ids) {
  const lines = [];
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
function offset(e) {
  return (e * (e + 1)) / 2;
}

/**
 * Myers' search run from the end of both documents. Entry i of round e is the smallest x on
 * diagonal k = n - m - e + 2i such that (x, x - k) is at most e edits from the end, or NONE when
 * no point of that diagonal is. `cost` is the first round that reaches (0, 0), which is the
 * shortest script's cost.
 */
function searchBack(a, b) {
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
      if (i < next && rounds[prev + i] !== NONE) {
        // Back from diagonal k + 1 over a deletion.
        best = Math.max(rounds[prev + i] - 1, lo);
      }
      if (i > 0 && rounds[prev + i - 1] !== NONE) {
        // Back from diagonal k - 1 over an insertion.
        best = Math.min(best, Math.max(rounds[prev + i - 1], lo));
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

/** The hunks [x, a, y, b] of the first shortest edit script in keep, delete, insert order. */
function diff(a, b) {
  const n = a.length;
  const m = b.length;
  const kend = n - m;
  let { rounds, cost: e } = searchBack(a, b);
  const keeps = (x, y) => x < n && y < m && a[x] === b[y];
  const hunks = [];
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
      if (x < n && i < e && rounds[offset(e - 1) + i] <= x + 1) {
        x++;
      } else {
        y++;
      }
      e--;
    } while (!keeps(x, y) && !(x === n && y === m));
    hunks.push([x0, x - x0, y0, y - y0]);
  }
}

async function main() {
  const input = await Bun.stdin.text();
  let pos = input.indexOf("\n") + 1;
  const pairs = Number(input.slice(0, pos - 1));
  const out = [];
  const ids = new Map();
  for (let pair = 0; pair < pairs; pair++) {
    const end = input.indexOf("\n", pos);
    const [n, m] = input.slice(pos, end).split(" ").map(Number);
    const first = input.slice(end + 1, end + 1 + n);
    const second = input.slice(end + 1 + n, end + 1 + n + m);
    pos = end + 1 + n + m;

    ids.clear();
    const hunks = diff(intern(first, ids), intern(second, ids));
    let deleted = 0;
    let inserted = 0;
    for (const hunk of hunks) {
      deleted += hunk[1];
      inserted += hunk[3];
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
