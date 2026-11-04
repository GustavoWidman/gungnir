use std::io::{self, Read, Write};

const HEX: &[u8; 16] = b"0123456789abcdef";
const MAX_I64: &[u8] = b"9223372036854775807";
const MIN_I64: &[u8] = b"9223372036854775808";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Scalar,
    Array,
    Object,
}

/// A value of the document. Nodes are stored in document order, so a node's index orders it,
/// and its descendants are exactly the nodes from its index + 1 up to `end`.
struct Node {
    kind: Kind,
    /// A scalar's printed text in `Document::text`, or a container's children in `Document::kids`.
    start: u32,
    len: u32,
    /// The member name printed as a string, in `Document::text`, when the parent is an object.
    name_start: u32,
    name_len: u32,
    end: u32,
}

struct Document {
    nodes: Vec<Node>,
    kids: Vec<u32>,
    text: Vec<u8>,
}

enum Selector {
    /// A member name printed as a string. Printing is one-to-one on code unit sequences, so
    /// two names are equal exactly when their printed forms are.
    Name(Vec<u8>),
    Wild,
    Index(i64),
    Slice(Option<i64>, Option<i64>, i64),
}

struct Segment {
    descendant: bool,
    selector: Selector,
}

fn push_unit(out: &mut Vec<u8>, unit: u32) {
    match unit {
        0x22 => out.extend_from_slice(b"\\\""),
        0x5C => out.extend_from_slice(b"\\\\"),
        0x08 => out.extend_from_slice(b"\\b"),
        0x0C => out.extend_from_slice(b"\\f"),
        0x0A => out.extend_from_slice(b"\\n"),
        0x0D => out.extend_from_slice(b"\\r"),
        0x09 => out.extend_from_slice(b"\\t"),
        0x20..=0x7E => out.push(unit as u8),
        _ => out.extend_from_slice(&[
            b'\\',
            b'u',
            HEX[(unit >> 12) as usize & 15],
            HEX[(unit >> 8) as usize & 15],
            HEX[(unit >> 4) as usize & 15],
            HEX[unit as usize & 15],
        ]),
    }
}

fn hex_digit(byte: u8) -> Option<u32> {
    (byte as char).to_digit(16)
}

/// Parses the string literal at `src[pos]`, which must be `"`, and appends it to `out` as it
/// prints. Returns the position after the closing quote, or None if the literal is invalid.
fn string(src: &[u8], mut pos: usize, out: &mut Vec<u8>) -> Option<usize> {
    out.push(b'"');
    pos += 1;
    loop {
        let run = src[pos..]
            .iter()
            .position(|&b| !(0x20..0x7F).contains(&b) || b == b'"' || b == b'\\')
            .unwrap_or(src.len() - pos);
        out.extend_from_slice(&src[pos..pos + run]);
        pos += run;
        let byte = *src.get(pos)?;
        pos += 1;
        match byte {
            b'"' => break,
            b'\\' => {
                let unit = match *src.get(pos)? {
                    b'"' => 0x22,
                    b'\\' => 0x5C,
                    b'/' => 0x2F,
                    b'b' => 0x08,
                    b'f' => 0x0C,
                    b'n' => 0x0A,
                    b'r' => 0x0D,
                    b't' => 0x09,
                    b'u' => {
                        let digits = src.get(pos + 1..pos + 5)?;
                        pos += 4;
                        digits.iter().try_fold(0, |acc, &d| Some(acc << 4 | hex_digit(d)?))?
                    }
                    _ => return None,
                };
                pos += 1;
                push_unit(out, unit);
            }
            0x7F => push_unit(out, 0x7F),
            0x00..=0x1F => return None,
            _ => {
                let (count, low, high) = match byte {
                    0xC2..=0xDF => (1, 0x80, 0xBF),
                    0xE0 => (2, 0xA0, 0xBF),
                    0xE1..=0xEC | 0xEE..=0xEF => (2, 0x80, 0xBF),
                    0xED => (2, 0x80, 0x9F),
                    0xF0 => (3, 0x90, 0xBF),
                    0xF1..=0xF3 => (3, 0x80, 0xBF),
                    0xF4 => (3, 0x80, 0x8F),
                    _ => return None,
                };
                let tail = src.get(pos..pos + count)?;
                if !(low..=high).contains(&tail[0]) || tail[1..].iter().any(|&b| !(0x80..=0xBF).contains(&b)) {
                    return None;
                }
                let point = tail.iter().fold(u32::from(byte) & (0x3F >> count), |acc, &b| acc << 6 | u32::from(b & 0x3F));
                pos += count;
                if point >= 0x10000 {
                    push_unit(out, 0xD800 + ((point - 0x10000) >> 10));
                    push_unit(out, 0xDC00 + ((point - 0x10000) & 0x3FF));
                } else {
                    push_unit(out, point);
                }
            }
        }
    }
    out.push(b'"');
    Some(pos)
}

/// Parses `-?(0|[1-9][0-9]*)` at `src[pos]`; returns its end and whether it fits in 64 bits.
fn integer(src: &[u8], mut pos: usize) -> Option<(usize, bool)> {
    let negative = src.get(pos) == Some(&b'-');
    pos += usize::from(negative);
    let first = pos;
    match src.get(pos) {
        Some(b'0') => pos += 1,
        Some(b'1'..=b'9') => pos += src[pos..].iter().take_while(|b| b.is_ascii_digit()).count(),
        _ => return None,
    }
    let digits = &src[first..pos];
    let limit = if negative { MIN_I64 } else { MAX_I64 };
    Some((pos, digits.len() < limit.len() || (digits.len() == limit.len() && digits <= limit)))
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
    doc: Document,
    pending: Vec<u32>,
}

impl Parser<'_> {
    fn skip_whitespace(&mut self) {
        while matches!(self.src.get(self.pos), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip_whitespace();
        let found = self.src.get(self.pos) == Some(&byte);
        self.pos += usize::from(found);
        found
    }

    fn string(&mut self) -> Option<(u32, u32)> {
        let start = self.doc.text.len();
        self.pos = string(self.src, self.pos, &mut self.doc.text)?;
        Some((start as u32, (self.doc.text.len() - start) as u32))
    }

    fn value(&mut self, name: (u32, u32)) -> Option<()> {
        self.skip_whitespace();
        let id = self.doc.nodes.len();
        self.doc.nodes.push(Node { kind: Kind::Scalar, start: 0, len: 0, name_start: name.0, name_len: name.1, end: 0 });
        let (kind, start, len) = match *self.src.get(self.pos)? {
            open @ (b'[' | b'{') => {
                self.pos += 1;
                let close = if open == b'[' { b']' } else { b'}' };
                let mark = self.pending.len();
                if !self.eat(close) {
                    loop {
                        let mut name = (0, 0);
                        if open == b'{' {
                            self.skip_whitespace();
                            if self.src.get(self.pos) != Some(&b'"') {
                                return None;
                            }
                            name = self.string()?;
                            if !self.eat(b':') {
                                return None;
                            }
                        }
                        self.pending.push(self.doc.nodes.len() as u32);
                        self.value(name)?;
                        if self.eat(close) {
                            break;
                        }
                        if !self.eat(b',') {
                            return None;
                        }
                    }
                }
                let start = self.doc.kids.len();
                self.doc.kids.extend_from_slice(&self.pending[mark..]);
                self.pending.truncate(mark);
                let kind = if open == b'[' { Kind::Array } else { Kind::Object };
                (kind, start as u32, (self.doc.kids.len() - start) as u32)
            }
            b'"' => {
                let (start, len) = self.string()?;
                (Kind::Scalar, start, len)
            }
            b'-' | b'0'..=b'9' => {
                let (end, fits) = integer(self.src, self.pos)?;
                if !fits {
                    return None;
                }
                let text = &self.src[self.pos..end];
                let start = self.doc.text.len();
                self.doc.text.extend_from_slice(if text == b"-0" { b"0" } else { text });
                self.pos = end;
                (Kind::Scalar, start as u32, (self.doc.text.len() - start) as u32)
            }
            _ => {
                let word: &[u8] = [b"true".as_slice(), b"false", b"null"]
                    .into_iter()
                    .find(|w| self.src[self.pos..].starts_with(w))?;
                self.pos += word.len();
                let start = self.doc.text.len();
                self.doc.text.extend_from_slice(word);
                (Kind::Scalar, start as u32, word.len() as u32)
            }
        };
        let end = self.doc.nodes.len() as u32;
        let node = &mut self.doc.nodes[id];
        (node.kind, node.start, node.len, node.end) = (kind, start, len, end);
        Some(())
    }
}

fn parse_document(src: &[u8]) -> Option<Document> {
    let doc = Document { nodes: Vec::new(), kids: Vec::new(), text: Vec::with_capacity(src.len()) };
    let mut parser = Parser { src, pos: 0, doc, pending: Vec::new() };
    parser.value((0, 0))?;
    parser.skip_whitespace();
    (parser.pos == src.len()).then_some(parser.doc)
}

fn bound(q: &[u8], pos: &mut usize) -> Option<Option<i64>> {
    if !matches!(q.get(*pos), Some(b'-' | b'0'..=b'9')) {
        return Some(None);
    }
    let (end, fits) = integer(q, *pos)?;
    let value = std::str::from_utf8(&q[*pos..end]).ok()?.parse().ok().filter(|_| fits)?;
    *pos = end;
    Some(Some(value))
}

/// Parses the selector of the bracket that opens at `q[pos]`; returns it and the position after `]`.
fn bracket(q: &[u8], mut pos: usize) -> Option<(Selector, usize)> {
    pos += 1;
    let selector = match q.get(pos)? {
        b'*' => {
            pos += 1;
            Selector::Wild
        }
        b'"' => {
            let mut name = Vec::new();
            pos = string(q, pos, &mut name)?;
            Selector::Name(name)
        }
        _ => {
            let start = bound(q, &mut pos)?;
            if q.get(pos) == Some(&b':') {
                pos += 1;
                let end = bound(q, &mut pos)?;
                let mut step = None;
                if q.get(pos) == Some(&b':') {
                    pos += 1;
                    step = bound(q, &mut pos)?;
                }
                if step.is_some_and(|s| s < 1) {
                    return None;
                }
                Selector::Slice(start, end, step.unwrap_or(1))
            } else {
                Selector::Index(start?)
            }
        }
    };
    (q.get(pos) == Some(&b']')).then_some((selector, pos + 1))
}

fn parse_query(q: &[u8]) -> Option<Vec<Segment>> {
    if q.first() != Some(&b'$') {
        return None;
    }
    let mut segments = Vec::new();
    let mut pos = 1;
    while pos < q.len() {
        let (descendant, dotted) = if q[pos..].starts_with(b"..") {
            pos += 2;
            (true, true)
        } else if q[pos] == b'.' {
            pos += 1;
            (false, true)
        } else {
            (false, false)
        };
        let selector = match q.get(pos) {
            Some(b'[') if descendant || !dotted => {
                let (selector, end) = bracket(q, pos)?;
                pos = end;
                selector
            }
            Some(b'*') if dotted => {
                pos += 1;
                Selector::Wild
            }
            Some(&c) if dotted && (c.is_ascii_alphabetic() || c == b'_') => {
                let len = q[pos..].iter().take_while(|&&c| c.is_ascii_alphanumeric() || c == b'_').count();
                let name = [b"\"".as_slice(), &q[pos..pos + len], b"\""].concat();
                pos += len;
                Selector::Name(name)
            }
            _ => return None,
        };
        segments.push(Segment { descendant, selector });
    }
    Some(segments)
}

/// Normalizes a slice to its first position, the bound it stays below, and its step.
fn slice_range(start: Option<i64>, end: Option<i64>, step: i64, len: usize) -> (usize, usize, usize) {
    let clamp = |bound: i64| {
        let bound = if bound < 0 { bound + len as i64 } else { bound };
        bound.clamp(0, len as i64) as usize
    };
    (start.map_or(0, clamp), end.map_or(len, clamp), step as usize)
}

fn index_position(index: i64, len: usize) -> Option<usize> {
    let position = if index < 0 { index + len as i64 } else { index };
    (0..len as i64).contains(&position).then_some(position as usize)
}

impl Document {
    fn kids(&self, node: &Node) -> &[u32] {
        match node.kind {
            Kind::Scalar => &[],
            Kind::Array | Kind::Object => &self.kids[node.start as usize..(node.start + node.len) as usize],
        }
    }

    fn name(&self, id: u32) -> &[u8] {
        let node = &self.nodes[id as usize];
        &self.text[node.name_start as usize..(node.name_start + node.name_len) as usize]
    }

    fn children(&self, id: u32, selector: &Selector, out: &mut Vec<u32>) {
        let node = &self.nodes[id as usize];
        let kids = self.kids(node);
        match (node.kind, selector) {
            (Kind::Object, Selector::Name(name)) => out.extend(kids.iter().filter(|&&c| self.name(c) == name.as_slice())),
            (Kind::Array | Kind::Object, Selector::Wild) => out.extend_from_slice(kids),
            (Kind::Array, Selector::Index(index)) => out.extend(index_position(*index, kids.len()).map(|j| kids[j])),
            (Kind::Array, &Selector::Slice(start, end, step)) => {
                let (start, end, step) = slice_range(start, end, step, kids.len());
                let mut j = start;
                while j < end {
                    out.push(kids[j]);
                    if step >= end - j {
                        break;
                    }
                    j += step;
                }
            }
            _ => {}
        }
    }

    /// Appends what `selector` picks from `id` and each of its descendants, in document order.
    fn descend(&self, id: u32, selector: &Selector, out: &mut Vec<u32>) {
        let node = &self.nodes[id as usize];
        if node.kind == Kind::Scalar {
            return;
        }
        let kids = self.kids(node);
        let array = node.kind == Kind::Array;
        let (start, end, step) = match *selector {
            Selector::Index(index) if array => index_position(index, kids.len()).map_or((0, 0, 1), |j| (j, j + 1, 1)),
            Selector::Slice(start, end, step) if array => slice_range(start, end, step, kids.len()),
            _ => (0, 0, 1),
        };
        for (j, &child) in kids.iter().enumerate() {
            let picked = match selector {
                Selector::Name(name) => !array && self.name(child) == name.as_slice(),
                Selector::Wild => true,
                _ => j >= start && j < end && (j - start) % step == 0,
            };
            if picked {
                out.push(child);
            }
            self.descend(child, selector, out);
        }
    }

    /// The nodes a query selects, each once, in document order. A set holds no node nested in
    /// another until a descendant segment, and only a nested set can come out unsorted.
    fn evaluate(&self, segments: &[Segment]) -> Vec<u32> {
        let mut nodes = vec![0];
        let mut nested = false;
        for segment in segments {
            let mut out = Vec::new();
            if segment.descendant {
                let mut covered = 0;
                for &id in &nodes {
                    if id >= covered {
                        self.descend(id, &segment.selector, &mut out);
                        covered = self.nodes[id as usize].end;
                    }
                }
                nested = true;
            } else {
                for &id in &nodes {
                    self.children(id, &segment.selector, &mut out);
                }
                if nested {
                    out.sort_unstable();
                }
            }
            nodes = out;
        }
        nodes
    }

    fn print(&self, id: u32, out: &mut Vec<u8>) {
        let node = &self.nodes[id as usize];
        let (open, close) = match node.kind {
            Kind::Scalar => {
                out.extend_from_slice(&self.text[node.start as usize..(node.start + node.len) as usize]);
                return;
            }
            Kind::Array => (b'[', b']'),
            Kind::Object => (b'{', b'}'),
        };
        out.push(open);
        for (j, &child) in self.kids(node).iter().enumerate() {
            if j > 0 {
                out.push(b',');
            }
            if node.kind == Kind::Object {
                out.extend_from_slice(self.name(child));
                out.push(b':');
            }
            self.print(child, out);
        }
        out.push(close);
    }
}

fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let mut lines = input.splitn(2, |&b| b == b'\n');
    let count: usize = std::str::from_utf8(lines.next().unwrap()).unwrap().parse().unwrap();
    let mut rest = lines.next().unwrap_or_default();
    let mut queries = Vec::with_capacity(count);
    for _ in 0..count {
        let newline = rest.iter().position(|&b| b == b'\n').unwrap();
        queries.push(&rest[..newline]);
        rest = &rest[newline + 1..];
    }

    let mut out = Vec::new();
    match parse_document(rest) {
        None => out.extend_from_slice(b"invalid document\n"),
        Some(doc) => {
            for query in queries {
                let Some(segments) = parse_query(query) else {
                    out.extend_from_slice(b"invalid query\n");
                    continue;
                };
                let nodes = doc.evaluate(&segments);
                writeln!(out, "{}", nodes.len()).unwrap();
                for id in nodes {
                    doc.print(id, &mut out);
                    out.push(b'\n');
                }
            }
        }
    }
    io::stdout().lock().write_all(&out).unwrap();
}
