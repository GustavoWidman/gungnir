use std::collections::HashMap;
use std::io::{self, Read, Write};

const NONE: u32 = u32::MAX;

/// Where round `e` starts in the flat array of rounds: rounds 0 to e - 1 hold 1 to e entries.
fn offset(e: usize) -> usize {
    e * (e + 1) / 2
}

/// Myers' search run from the end of both documents.
///
/// Entry `i` of round `e` is the smallest x on diagonal k = a.len() - b.len() - e + 2i such
/// that (x, x - k) is at most e edits from the end, or NONE when no point of that diagonal is.
/// The last round is the first that reaches (0, 0), so its index is the shortest script's cost.
fn search_back(a: &[u32], b: &[u32]) -> (Vec<u32>, usize) {
    let (n, m) = (a.len() as i64, b.len() as i64);
    let kend = n - m;
    let (mut x, mut y) = (a.len(), b.len());
    while x > 0 && y > 0 && a[x - 1] == b[y - 1] {
        x -= 1;
        y -= 1;
    }
    let mut rounds = vec![x as u32];
    let mut e: i64 = 0;
    loop {
        let reached = (e - kend) & 1 == 0 && kend.abs() <= e;
        if reached && rounds[offset(e as usize) + ((e - kend) / 2) as usize] == 0 {
            return (rounds, e as usize);
        }
        e += 1;
        let prev = offset(e as usize - 1);
        let cur = offset(e as usize);
        rounds.resize(cur + e as usize + 1, NONE);
        // Only the diagonals -m <= k <= n cross the grid.
        let first = (e - n + 1).div_euclid(2).max(0);
        let last = ((m + e) / 2).min(e);
        for i in first..=last {
            let k = kend - e + 2 * i;
            let lo = k.max(0);
            let mut best = i64::MAX;
            if i < e && rounds[prev + i as usize] != NONE {
                // Back from diagonal k + 1 over a deletion.
                best = (rounds[prev + i as usize] as i64 - 1).max(lo);
            }
            if i > 0 && rounds[prev + i as usize - 1] != NONE {
                // Back from diagonal k - 1 over an insertion.
                best = best.min((rounds[prev + i as usize - 1] as i64).max(lo));
            }
            if best != i64::MAX {
                let (mut x, mut y) = (best as usize, (best - k) as usize);
                while x > 0 && y > 0 && a[x - 1] == b[y - 1] {
                    x -= 1;
                    y -= 1;
                }
                rounds[cur + i as usize] = x as u32;
            }
        }
    }
}

/// The hunks (x, a, y, b) of the first shortest edit script in keep, delete, insert order.
fn diff(a: &[u32], b: &[u32], hunks: &mut Vec<[usize; 4]>) {
    let (n, m) = (a.len(), b.len());
    let kend = n as i64 - m as i64;
    let (rounds, mut e) = search_back(a, b);
    let (mut x, mut y) = (0, 0);
    loop {
        while x < n && y < m && a[x] == b[y] {
            x += 1;
            y += 1;
        }
        if x == n && y == m {
            return;
        }
        let (x0, y0) = (x, y);
        loop {
            // Delete when (x + 1, y) is still within e - 1 edits of the end.
            let i = ((x as i64 - y as i64 + e as i64 - kend) / 2) as usize;
            if x < n && i < e && rounds[offset(e - 1) + i] as usize <= x + 1 {
                x += 1;
            } else {
                y += 1;
            }
            e -= 1;
            if (x < n && y < m && a[x] == b[y]) || (x == n && y == m) {
                break;
            }
        }
        hunks.push([x0, x - x0, y0, y - y0]);
    }
}

/// Numbers the lines of `doc`, giving equal lines equal numbers. A line keeps its line feed,
/// so a last line without one differs from the same text with one.
fn intern<'a>(doc: &'a [u8], ids: &mut HashMap<&'a [u8], u32>) -> Vec<u32> {
    doc.split_inclusive(|&c| c == b'\n')
        .map(|line| {
            let next = ids.len() as u32;
            *ids.entry(line).or_insert(next)
        })
        .collect()
}

/// Parses the decimal integer that `text` holds.
fn number(text: &[u8]) -> usize {
    std::str::from_utf8(text).unwrap().parse().unwrap()
}

fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let mut out = io::BufWriter::with_capacity(1 << 16, io::stdout().lock());

    let newline = input.iter().position(|&c| c == b'\n').unwrap();
    let pairs = number(&input[..newline]);
    let mut pos = newline + 1;
    let mut ids = HashMap::new();
    let mut hunks = Vec::new();
    for _ in 0..pairs {
        let end = pos + input[pos..].iter().position(|&c| c == b'\n').unwrap();
        let space = pos + input[pos..end].iter().position(|&c| c == b' ').unwrap();
        let (n, m) = (number(&input[pos..space]), number(&input[space + 1..end]));
        let first = &input[end + 1..end + 1 + n];
        let second = &input[end + 1 + n..end + 1 + n + m];
        pos = end + 1 + n + m;

        ids.clear();
        let a = intern(first, &mut ids);
        let b = intern(second, &mut ids);
        hunks.clear();
        diff(&a, &b, &mut hunks);
        let deleted: usize = hunks.iter().map(|h| h[1]).sum();
        let inserted: usize = hunks.iter().map(|h| h[3]).sum();
        writeln!(out, "{deleted} {inserted} {}", hunks.len()).unwrap();
        for [x, a, y, b] in &hunks {
            writeln!(out, "{x} {a} {y} {b}").unwrap();
        }
    }
    out.flush().unwrap();
}
