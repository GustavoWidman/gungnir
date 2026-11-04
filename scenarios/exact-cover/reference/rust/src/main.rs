use std::io::{self, Read};

type Shape = Vec<(i32, i32)>;

/// Shifts the shape to row and column 0 and sorts its cells.
fn normalize(cells: &[(i32, i32)]) -> Shape {
    let top = cells.iter().map(|&(r, _)| r).min().unwrap();
    let left = cells.iter().map(|&(_, c)| c).min().unwrap();
    let mut out: Shape = cells.iter().map(|&(r, c)| (r - top, c - left)).collect();
    out.sort_unstable();
    out
}

/// The distinct shapes the 4 rotations and their mirror images give, the smallest first.
fn orientations(cells: &[(i32, i32)]) -> Vec<Shape> {
    let mut cells = cells.to_vec();
    let mut found = Vec::new();
    for _ in 0..2 {
        for _ in 0..4 {
            cells = cells.iter().map(|&(r, c)| (c, -r)).collect();
            found.push(normalize(&cells));
        }
        cells = cells.iter().map(|&(r, c)| (r, -c)).collect();
    }
    found.sort_unstable();
    found.dedup();
    found
}

/// Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
/// covered once all its copies are placed, so copies of a shape are never told apart.
struct Dlx {
    cells: usize,
    root: usize,
    copies: Vec<u32>,
    left: Vec<usize>,
    right: Vec<usize>,
    up: Vec<usize>,
    down: Vec<usize>,
    column: Vec<usize>,
    size: Vec<u32>,
}

impl Dlx {
    fn new(cells: usize, copies: Vec<u32>) -> Self {
        let root = cells + copies.len();
        let ids: Vec<usize> = (0..=root).collect();
        let mut dlx = Dlx {
            cells,
            root,
            copies,
            left: ids.clone(),
            right: ids.clone(),
            up: ids.clone(),
            down: ids.clone(),
            column: ids,
            size: vec![0; root + 1],
        };
        // Only the cell columns and the root form the header ring.
        let mut previous = root;
        for c in 0..cells {
            dlx.right[previous] = c;
            dlx.left[c] = previous;
            previous = c;
        }
        dlx.right[previous] = root;
        dlx.left[root] = previous;
        dlx
    }

    fn add_row(&mut self, columns: &[usize]) {
        let first = self.left.len();
        let count = columns.len();
        for (k, &col) in columns.iter().enumerate() {
            let node = first + k;
            self.column.push(col);
            self.up.push(self.up[col]);
            self.down.push(col);
            let above = self.up[col];
            self.down[above] = node;
            self.up[col] = node;
            self.size[col] += 1;
            self.left.push(if k > 0 { node - 1 } else { first + count - 1 });
            self.right.push(if k + 1 < count { node + 1 } else { first });
        }
    }

    fn cover(&mut self, c: usize) {
        let (l, r) = (self.left[c], self.right[c]);
        self.left[r] = l;
        self.right[l] = r;
        let mut i = self.down[c];
        while i != c {
            let mut j = self.right[i];
            while j != i {
                let (u, d) = (self.up[j], self.down[j]);
                self.up[d] = u;
                self.down[u] = d;
                self.size[self.column[j]] -= 1;
                j = self.right[j];
            }
            i = self.down[i];
        }
    }

    fn uncover(&mut self, c: usize) {
        let mut i = self.up[c];
        while i != c {
            let mut j = self.left[i];
            while j != i {
                self.size[self.column[j]] += 1;
                let (u, d) = (self.up[j], self.down[j]);
                self.up[d] = j;
                self.down[u] = j;
                j = self.left[j];
            }
            i = self.up[i];
        }
        let (l, r) = (self.left[c], self.right[c]);
        self.left[r] = c;
        self.right[l] = c;
    }

    fn place(&mut self, row: usize) {
        let mut j = self.right[row];
        while j != row {
            let col = self.column[j];
            if col < self.cells {
                self.cover(col);
            } else {
                self.copies[col - self.cells] -= 1;
                if self.copies[col - self.cells] == 0 {
                    self.cover(col);
                }
            }
            j = self.right[j];
        }
    }

    fn unplace(&mut self, row: usize) {
        let mut j = self.left[row];
        while j != row {
            let col = self.column[j];
            if col < self.cells {
                self.uncover(col);
            } else {
                if self.copies[col - self.cells] == 0 {
                    self.uncover(col);
                }
                self.copies[col - self.cells] += 1;
            }
            j = self.left[j];
        }
    }

    /// The number of exact covers, choosing at each step the cell with the fewest options.
    fn count(&mut self) -> u64 {
        let mut best = self.right[self.root];
        if best == self.root {
            return 1;
        }
        let mut c = self.right[best];
        while c != self.root && self.size[best] > 0 {
            if self.size[c] < self.size[best] {
                best = c;
            }
            c = self.right[c];
        }
        if self.size[best] == 0 {
            return 0;
        }
        self.cover(best);
        let mut total = 0;
        let mut row = self.down[best];
        while row != best {
            self.place(row);
            total += self.count();
            self.unplace(row);
            row = self.down[row];
        }
        self.uncover(best);
        total
    }
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut tokens = input.split_ascii_whitespace();
    let mut number = || tokens.next().unwrap().parse::<usize>().unwrap();
    let (height, width) = (number(), number());
    drop(number);

    let mut index = vec![None; height * width];
    let mut cells = 0;
    for r in 0..height {
        let line = tokens.next().unwrap().as_bytes();
        for c in 0..width {
            if line[c] == b'.' {
                index[r * width + c] = Some(cells);
                cells += 1;
            }
        }
    }

    let pieces: usize = tokens.next().unwrap().parse().unwrap();
    let mut shapes: Vec<Shape> = Vec::new();
    let mut copies: Vec<u32> = Vec::new();
    let mut area = 0;
    for _ in 0..pieces {
        let rows: usize = tokens.next().unwrap().parse().unwrap();
        tokens.next();
        let mut piece = Vec::new();
        for r in 0..rows {
            for (c, &ch) in tokens.next().unwrap().as_bytes().iter().enumerate() {
                if ch == b'#' {
                    piece.push((r as i32, c as i32));
                }
            }
        }
        area += piece.len();
        let canonical = orientations(&piece).swap_remove(0);
        match shapes.iter().position(|s| *s == canonical) {
            Some(kind) => copies[kind] += 1,
            None => {
                shapes.push(canonical);
                copies.push(1);
            }
        }
    }

    let mut found: u64 = 0;
    if area == cells {
        let mut dlx = Dlx::new(cells, copies.clone());
        let mut row = Vec::new();
        for (kind, shape) in shapes.iter().enumerate() {
            for oriented in orientations(shape) {
                for top in 0..height {
                    for left in 0..width {
                        row.clear();
                        for &(r, c) in &oriented {
                            let (r, c) = (top + r as usize, left + c as usize);
                            match (r < height && c < width).then(|| index[r * width + c]).flatten() {
                                Some(id) => row.push(id),
                                None => break,
                            }
                        }
                        if row.len() == oriented.len() {
                            row.push(cells + kind);
                            dlx.add_row(&row);
                        }
                    }
                }
            }
        }
        found = dlx.count();
        if found > 0 {
            for &k in &copies {
                found *= (2..=u64::from(k)).product::<u64>();
            }
        }
    }
    println!("{found}");
}
