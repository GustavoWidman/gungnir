use std::collections::HashMap;
use std::io::{self, Read, Write};

fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let newline = input.iter().position(|&b| b == b'\n').unwrap();
    let (first, text) = input.split_at_mut(newline + 1);
    let k: usize = std::str::from_utf8(&first[..newline]).unwrap().parse().unwrap();
    text.make_ascii_lowercase();

    let mut counts: HashMap<&[u8], u32> = HashMap::new();
    for word in text.split(|b| !b.is_ascii_alphabetic()).filter(|w| !w.is_empty()) {
        *counts.entry(word).or_insert(0) += 1;
    }

    let mut ranked: Vec<(&[u8], u32)> = counts.into_iter().collect();
    ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

    let mut out = io::BufWriter::with_capacity(1 << 16, io::stdout().lock());
    for (word, count) in ranked.into_iter().take(k) {
        out.write_all(word).unwrap();
        writeln!(out, " {count}").unwrap();
    }
    out.flush().unwrap();
}
