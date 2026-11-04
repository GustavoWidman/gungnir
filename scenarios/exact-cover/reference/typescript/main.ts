type Cell = [number, number];

/** Shifts the cells to row and column 0 and sorts them. */
function normalize(cells: Cell[]): Cell[] {
  const top = Math.min(...cells.map(([r]) => r));
  const left = Math.min(...cells.map(([, c]) => c));
  return cells.map(([r, c]): Cell => [r - top, c - left]).sort((a, b) => a[0] - b[0] || a[1] - b[1]);
}

/** The distinct shapes, as strings and cells, that the 4 rotations and their mirror images give, the smallest first. */
function orientations(cells: Cell[]): [string, Cell[]][] {
  const found = new Map<string, Cell[]>();
  for (let mirror = 0; mirror < 2; mirror++) {
    for (let turn = 0; turn < 4; turn++) {
      cells = cells.map(([r, c]): Cell => [c, -r]);
      const shape = normalize(cells);
      found.set(shape.join(";"), shape);
    }
    cells = cells.map(([r, c]): Cell => [r, -c]);
  }
  return [...found].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
}

/**
 * Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
 * covered once all its copies are placed, so copies of a shape are never told apart.
 */
class Dlx {
  readonly cells: number;
  readonly root: number;
  readonly copies: Int32Array;
  readonly left: Int32Array;
  readonly right: Int32Array;
  readonly up: Int32Array;
  readonly down: Int32Array;
  readonly column: Int32Array;
  readonly size: Int32Array;

  constructor(cells: number, copies: number[], rows: number[][]) {
    const root = cells + copies.length;
    const nodes = root + 1 + rows.reduce((sum, row) => sum + row.length, 0);
    this.cells = cells;
    this.root = root;
    this.copies = Int32Array.from(copies);
    this.left = new Int32Array(nodes);
    this.right = new Int32Array(nodes);
    this.up = new Int32Array(nodes);
    this.down = new Int32Array(nodes);
    this.column = new Int32Array(nodes);
    this.size = new Int32Array(root + 1);
    for (let c = 0; c <= root; c++) {
      this.left[c] = this.right[c] = this.up[c] = this.down[c] = this.column[c] = c;
    }
    // Only the cell columns and the root form the header ring.
    let previous = root;
    for (let c = 0; c < cells; c++) {
      this.right[previous] = c;
      this.left[c] = previous;
      previous = c;
    }
    this.right[previous] = root;
    this.left[root] = previous;

    let node = root + 1;
    for (const row of rows) {
      const first = node;
      for (let k = 0; k < row.length; k++, node++) {
        const col = row[k]!;
        this.column[node] = col;
        this.up[node] = this.up[col]!;
        this.down[node] = col;
        this.down[this.up[col]!] = node;
        this.up[col] = node;
        this.size[col]!++;
        this.left[node] = k > 0 ? node - 1 : first + row.length - 1;
        this.right[node] = k + 1 < row.length ? node + 1 : first;
      }
    }
  }

  cover(c: number): void {
    const { left, right, up, down, column, size } = this;
    left[right[c]!] = left[c]!;
    right[left[c]!] = right[c]!;
    for (let i = down[c]!; i !== c; i = down[i]!) {
      for (let j = right[i]!; j !== i; j = right[j]!) {
        up[down[j]!] = up[j]!;
        down[up[j]!] = down[j]!;
        size[column[j]!]!--;
      }
    }
  }

  uncover(c: number): void {
    const { left, right, up, down, column, size } = this;
    for (let i = up[c]!; i !== c; i = up[i]!) {
      for (let j = left[i]!; j !== i; j = left[j]!) {
        size[column[j]!]!++;
        up[down[j]!] = j;
        down[up[j]!] = j;
      }
    }
    left[right[c]!] = c;
    right[left[c]!] = c;
  }

  place(row: number): void {
    for (let j = this.right[row]!; j !== row; j = this.right[j]!) {
      const col = this.column[j]!;
      if (col < this.cells || --this.copies[col - this.cells]! === 0) this.cover(col);
    }
  }

  unplace(row: number): void {
    for (let j = this.left[row]!; j !== row; j = this.left[j]!) {
      const col = this.column[j]!;
      if (col < this.cells || this.copies[col - this.cells]!++ === 0) this.uncover(col);
    }
  }

  /** The number of exact covers, choosing at each step the cell with the fewest options. */
  count(): number {
    const { right, size, down, root } = this;
    let best = right[root]!;
    if (best === root) return 1;
    for (let c = right[best]!; c !== root && size[best]! > 0; c = right[c]!) {
      if (size[c]! < size[best]!) best = c;
    }
    if (size[best]! === 0) return 0;
    this.cover(best);
    let total = 0;
    for (let row = down[best]!; row !== best; row = down[row]!) {
      this.place(row);
      total += this.count();
      this.unplace(row);
    }
    this.uncover(best);
    return total;
  }
}

async function main(): Promise<void> {
  const tokens = (await Bun.stdin.text()).split(/\s+/);
  let at = 0;
  const height = Number(tokens[at++]);
  const width = Number(tokens[at++]);
  const index = new Int32Array(height * width).fill(-1);
  let cells = 0;
  for (let r = 0; r < height; r++) {
    const line = tokens[at++]!;
    for (let c = 0; c < width; c++) {
      if (line[c] === ".") index[r * width + c] = cells++;
    }
  }

  const pieces = Number(tokens[at++]);
  const copies = new Map<string, { shape: Cell[]; count: number }>();
  let area = 0;
  for (let p = 0; p < pieces; p++) {
    const rows = Number(tokens[at]);
    at += 2;
    const piece: Cell[] = [];
    for (let r = 0; r < rows; r++) {
      const line = tokens[at++]!;
      for (let c = 0; c < line.length; c++) {
        if (line[c] === "#") piece.push([r, c]);
      }
    }
    area += piece.length;
    const [key, shape] = orientations(piece)[0]!;
    const entry = copies.get(key) ?? { shape, count: 0 };
    entry.count++;
    copies.set(key, entry);
  }

  let found = 0n;
  if (area === cells) {
    const kinds = [...copies.values()];
    const rows: number[][] = [];
    kinds.forEach(({ shape }, kind) => {
      for (const [, oriented] of orientations(shape)) {
        for (let top = 0; top < height; top++) {
          for (let left = 0; left < width; left++) {
            const row: number[] = [];
            for (const [r, c] of oriented) {
              const id = top + r < height && left + c < width ? index[(top + r) * width + left + c]! : -1;
              if (id < 0) break;
              row.push(id);
            }
            if (row.length === oriented.length) rows.push([...row, cells + kind]);
          }
        }
      }
    });
    found = BigInt(new Dlx(cells, kinds.map(({ count }) => count), rows).count());
    // The answer can pass 2**53, beyond what a number holds exactly.
    for (const { count } of kinds) {
      for (let k = 2n; k <= BigInt(count) && found > 0n; k++) found *= k;
    }
  }
  await Bun.write(Bun.stdout, `${found}\n`);
}

await main();
