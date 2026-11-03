use std::io::{self, Read, Write};

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut tokens = input
        .split_ascii_whitespace()
        .map(|t| t.parse::<i64>().unwrap());
    let n = tokens.next().unwrap() as usize;
    let q = tokens.next().unwrap() as usize;

    let mut starts = Vec::with_capacity(n);
    let mut ends = Vec::with_capacity(n);
    for _ in 0..n {
        starts.push(tokens.next().unwrap());
        ends.push(tokens.next().unwrap());
    }
    starts.sort_unstable();
    ends.sort_unstable();

    let mut out = io::BufWriter::with_capacity(1 << 16, io::stdout().lock());
    for x in tokens.take(q) {
        // Intervals containing x: those starting at or before x, minus those ending before x.
        let count = starts.partition_point(|&s| s <= x) - ends.partition_point(|&e| e < x);
        writeln!(out, "{count}").unwrap();
    }
    out.flush().unwrap();
}
