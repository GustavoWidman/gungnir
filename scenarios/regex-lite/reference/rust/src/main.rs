use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};
use std::io::{self, Read, Write};

const ESCAPABLE: &[u8] = b"\\.*+?|()[]^$-";
/// A text line has at most 100000 characters, so one line adds at most about 300000 DFA states.
const STATE_LIMIT: usize = 10_000;
const UNKNOWN: u32 = u32::MAX;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Char,
    Eps,
    Begin,
    End,
    Match,
}

struct Invalid;

/// Builds a Thompson NFA while parsing. Every fragment is an (entry, exit) pair whose exit is an
/// `Eps` node that later gets the edges to whatever follows.
struct Parser<'a> {
    pattern: &'a [u8],
    pos: usize,
    kind: Vec<Kind>,
    chars: Vec<u128>,
    out: Vec<Vec<usize>>,
}

impl<'a> Parser<'a> {
    fn new(pattern: &'a [u8]) -> Self {
        Parser { pattern, pos: 0, kind: Vec::new(), chars: Vec::new(), out: Vec::new() }
    }

    fn node(&mut self, kind: Kind, chars: u128) -> usize {
        self.kind.push(kind);
        self.chars.push(chars);
        self.out.push(Vec::new());
        self.kind.len() - 1
    }

    fn peek(&self) -> Option<u8> {
        self.pattern.get(self.pos).copied()
    }

    fn take(&mut self) -> Result<u8, Invalid> {
        let c = self.peek().ok_or(Invalid)?;
        self.pos += 1;
        Ok(c)
    }

    /// Parses the whole pattern and returns the start node. The accepting node comes last.
    fn parse(&mut self) -> Result<usize, Invalid> {
        let (start, exit) = self.alternation()?;
        if self.pos != self.pattern.len() {
            return Err(Invalid);
        }
        let accept = self.node(Kind::Match, 0);
        self.out[exit].push(accept);
        Ok(start)
    }

    fn alternation(&mut self) -> Result<(usize, usize), Invalid> {
        let (entry, exit) = (self.node(Kind::Eps, 0), self.node(Kind::Eps, 0));
        loop {
            let (first, last) = self.sequence()?;
            self.out[entry].push(first);
            self.out[last].push(exit);
            if self.peek() != Some(b'|') {
                return Ok((entry, exit));
            }
            self.pos += 1;
        }
    }

    fn sequence(&mut self) -> Result<(usize, usize), Invalid> {
        let entry = self.node(Kind::Eps, 0);
        let mut last = entry;
        while let Some(c) = self.peek() {
            if c == b'|' || c == b')' {
                break;
            }
            let (first, end) = self.item()?;
            self.out[last].push(first);
            last = end;
        }
        Ok((entry, last))
    }

    fn item(&mut self) -> Result<(usize, usize), Invalid> {
        let (first, last) = self.atom()?;
        match self.peek() {
            Some(b'*') => {
                self.pos += 1;
                let looped = self.node(Kind::Eps, 0);
                self.out[looped].push(first);
                self.out[last].push(looped);
                Ok((looped, looped))
            }
            Some(b'+') => {
                self.pos += 1;
                self.out[last].push(first);
                Ok((first, last))
            }
            Some(b'?') => {
                self.pos += 1;
                let skip = self.node(Kind::Eps, 0);
                self.out[skip].extend([first, last]);
                Ok((skip, last))
            }
            _ => Ok((first, last)),
        }
    }

    fn atom(&mut self) -> Result<(usize, usize), Invalid> {
        let chars = match self.take()? {
            b'(' => {
                let fragment = self.alternation()?;
                return if self.take()? == b')' { Ok(fragment) } else { Err(Invalid) };
            }
            b'*' | b'+' | b'?' => return Err(Invalid),
            b'^' => return Ok(self.wrap(Kind::Begin, 0)),
            b'$' => return Ok(self.wrap(Kind::End, 0)),
            b'.' => u128::MAX,
            b'[' => self.bracket()?,
            b'\\' => 1 << self.escaped()?,
            c => 1 << c,
        };
        Ok(self.wrap(Kind::Char, chars))
    }

    fn wrap(&mut self, kind: Kind, chars: u128) -> (usize, usize) {
        let node = self.node(kind, chars);
        let exit = self.node(Kind::Eps, 0);
        self.out[node].push(exit);
        (node, exit)
    }

    fn escaped(&mut self) -> Result<u8, Invalid> {
        let c = self.take()?;
        if ESCAPABLE.contains(&c) { Ok(c) } else { Err(Invalid) }
    }

    fn bracket(&mut self) -> Result<u128, Invalid> {
        let negated = self.peek() == Some(b'^');
        if negated {
            self.pos += 1;
        }
        // Each body character as (byte, is an unescaped dash).
        let mut body: Vec<(u8, bool)> = Vec::new();
        loop {
            match self.take()? {
                b']' => break,
                b'\\' => body.push((self.escaped()?, false)),
                c => body.push((c, c == b'-')),
            }
        }
        if body.is_empty() {
            return Err(Invalid);
        }
        let (mut chars, mut start, mut stop) = (0u128, 0, body.len());
        if body[0].1 {
            chars |= 1 << b'-';
            start = 1;
        }
        if stop > start && body[stop - 1].1 {
            chars |= 1 << b'-';
            stop -= 1;
        }
        let mut i = start;
        while i < stop {
            let (low, dash) = body[i];
            if dash {
                return Err(Invalid);
            }
            if i + 1 < stop && body[i + 1].1 {
                if i + 2 >= stop || body[i + 2].1 {
                    return Err(Invalid);
                }
                let high = body[i + 2].0;
                if low > high {
                    return Err(Invalid);
                }
                chars |= (u128::MAX >> (127 - high)) & (u128::MAX << low);
                i += 3;
            } else {
                chars |= 1 << low;
                i += 1;
            }
        }
        Ok(if negated { !chars } else { chars })
    }
}

/// A multiply-rotate hasher for the DFA's own keys, which need no protection from collisions.
#[derive(Default)]
struct Fx(u64);

impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }

    fn write_u64(&mut self, word: u64) {
        self.0 = (self.0.rotate_left(5) ^ word).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

type FastMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;
type Bits = Box<[u64]>;

fn set(bits: &mut [u64], k: usize) {
    bits[k / 64] |= 1 << (k % 64);
}

fn intersects(a: &[u64], b: &[u64]) -> bool {
    a.iter().zip(b).any(|(x, y)| x & y != 0)
}

/// One interned DFA state: a bitset over the kernel and what is cached about it.
struct State {
    bits: Bits,
    /// Reverse transitions by character, `UNKNOWN` until computed.
    back: [u32; 128],
    /// The state after every member consumes a character, `UNKNOWN` until computed.
    union: u32,
    /// This state intersected with the start closure at an inner position.
    entry: u32,
    accepts: bool,
    consumes: bool,
    starts: bool,
}

/// Counts leftmost-longest matches in linear time per line.
///
/// The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
/// node that consumes a character. A reverse pass records at each position which kernel states
/// can still reach acceptance from there. Forward scans start only where a match starts and keep
/// only states that can still accept, so each scan stops right after its longest match. The
/// forward DFA reads the reverse pass's state at each position as its input symbol.
struct Matcher {
    follow: Vec<Bits>,
    consumers: Vec<Vec<usize>>,
    /// The closure of the start node, indexed by [at line start][at line end].
    start: [[Bits; 2]; 2],
    /// For each character, the states that can accept from the last position when it is there.
    last: Vec<Bits>,
    ids: FastMap<Bits, u32>,
    states: Vec<State>,
    forward: FastMap<u64, u32>,
    last_ids: Vec<u32>,
    accept_only: u32,
    scratch: Vec<u64>,
}

impl Matcher {
    fn new(parser: &Parser, start_node: usize) -> Self {
        let (kind, out) = (&parser.kind, &parser.out);
        let mut kernel = vec![usize::MAX; kind.len()];
        kernel[kind.len() - 1] = 0;
        let mut size = 1;
        for (node, &k) in kind.iter().enumerate() {
            if k == Kind::Char {
                kernel[node] = size;
                size += 1;
            }
        }
        let words = size.div_ceil(64);
        let empty = || vec![0u64; words].into_boxed_slice();
        let mut mark = vec![0u32; kind.len()];
        let mut stamp = 0;
        let mut closure = |roots: &[usize], begin: bool, end: bool| -> Bits {
            stamp += 1;
            let mut bits = empty();
            let mut stack: Vec<usize> = Vec::new();
            for &root in roots {
                if mark[root] != stamp {
                    mark[root] = stamp;
                    stack.push(root);
                }
            }
            while let Some(node) = stack.pop() {
                match kind[node] {
                    Kind::Char | Kind::Match => {
                        set(&mut bits, kernel[node]);
                        continue;
                    }
                    Kind::Begin if !begin => continue,
                    Kind::End if !end => continue,
                    _ => {}
                }
                for &next in &out[node] {
                    if mark[next] != stamp {
                        mark[next] = stamp;
                        stack.push(next);
                    }
                }
            }
            bits
        };

        let mut follow = vec![empty(); size];
        let mut follow_end = vec![empty(); size];
        let mut consumers = vec![Vec::new(); 128];
        for (node, &k) in kernel.iter().enumerate() {
            if k == usize::MAX || k == 0 {
                continue;
            }
            follow[k] = closure(&out[node], false, false);
            follow_end[k] = closure(&out[node], false, true);
            for (c, list) in consumers.iter_mut().enumerate().take(127).skip(32) {
                if parser.chars[node] >> c & 1 == 1 {
                    list.push(k);
                }
            }
        }
        let start = [
            [closure(&[start_node], false, false), closure(&[start_node], false, true)],
            [closure(&[start_node], true, false), closure(&[start_node], true, true)],
        ];
        let last = consumers
            .iter()
            .map(|list| {
                let mut bits = empty();
                set(&mut bits, 0);
                for &k in list.iter().filter(|&&k| follow_end[k][0] & 1 == 1) {
                    set(&mut bits, k);
                }
                bits
            })
            .collect();
        let mut matcher = Matcher {
            follow,
            consumers,
            start,
            last,
            ids: FastMap::default(),
            states: Vec::new(),
            forward: FastMap::default(),
            last_ids: Vec::new(),
            accept_only: 0,
            scratch: vec![0; words],
        };
        matcher.reset();
        matcher
    }

    fn reset(&mut self) {
        self.ids.clear();
        self.states.clear();
        self.forward.clear();
        self.scratch.fill(0);
        self.scratch[0] = 1;
        self.accept_only = self.intern_scratch();
        self.last_ids = (0..128)
            .map(|c| {
                self.scratch.copy_from_slice(&self.last[c]);
                self.intern_scratch()
            })
            .collect();
    }

    fn intern_scratch(&mut self) -> u32 {
        if let Some(&id) = self.ids.get(&self.scratch[..]) {
            return id;
        }
        let bits: Bits = self.scratch.as_slice().into();
        let id = self.states.len() as u32;
        self.states.push(State {
            back: [UNKNOWN; 128],
            union: UNKNOWN,
            entry: UNKNOWN,
            accepts: bits[0] & 1 == 1,
            consumes: bits[0] > 1 || bits[1..].iter().any(|&w| w != 0),
            starts: intersects(&self.start[0][0], &bits),
            bits: bits.clone(),
        });
        self.ids.insert(bits, id);
        id
    }

    /// The state holding the members of both `a` and `b`.
    fn meet(&mut self, a: &[u64], b: u32) -> u32 {
        for (s, (x, y)) in self.scratch.iter_mut().zip(a.iter().zip(self.states[b as usize].bits.iter())) {
            *s = x & y;
        }
        self.intern_scratch()
    }

    /// The kernel states that can accept from a position holding `c`, given those that can
    /// accept from the next position.
    fn step_back(&mut self, state: u32, c: u8) -> u32 {
        self.scratch.fill(0);
        self.scratch[0] = 1;
        let ahead = &self.states[state as usize].bits;
        for &k in &self.consumers[c as usize] {
            if intersects(&self.follow[k], ahead) {
                set(&mut self.scratch, k);
            }
        }
        let next = self.intern_scratch();
        self.states[state as usize].back[c as usize] = next;
        next
    }

    /// The state after every member of `state` consumes a character. The character is implied:
    /// every state in a pruned set accepts the character at its position.
    fn union(&mut self, state: u32) -> u32 {
        let cached = self.states[state as usize].union;
        if cached != UNKNOWN {
            return cached;
        }
        self.scratch.fill(0);
        for (w, &word) in self.states[state as usize].bits.iter().enumerate() {
            let mut rest = if w == 0 { word & !1 } else { word };
            while rest != 0 {
                let k = w * 64 + rest.trailing_zeros() as usize;
                for (s, f) in self.scratch.iter_mut().zip(self.follow[k].iter()) {
                    *s |= f;
                }
                rest &= rest - 1;
            }
        }
        let union = self.intern_scratch();
        self.states[state as usize].union = union;
        union
    }

    /// The pruned forward state after `state`, at a position whose reverse state is `alive`.
    fn step(&mut self, state: u32, alive: u32) -> u32 {
        let key = (state as u64) << 32 | alive as u64;
        if let Some(&next) = self.forward.get(&key) {
            return next;
        }
        let union = self.union(state);
        let bits = self.states[union as usize].bits.clone();
        let next = self.meet(&bits, alive);
        self.forward.insert(key, next);
        next
    }

    /// The forward state at a match candidate `s` of a line of `n` characters.
    fn entry(&mut self, s: usize, n: usize, alive: u32) -> u32 {
        if 0 < s && s < n {
            let cached = self.states[alive as usize].entry;
            if cached != UNKNOWN {
                return cached;
            }
            let start = self.start[0][0].clone();
            let entry = self.meet(&start, alive);
            self.states[alive as usize].entry = entry;
            return entry;
        }
        let start = self.start[(s == 0) as usize][(s == n) as usize].clone();
        self.meet(&start, alive)
    }

    fn count(&mut self, line: &[u8], alive: &mut Vec<u32>) -> u32 {
        if self.states.len() + self.forward.len() > STATE_LIMIT {
            self.reset();
        }
        let n = line.len();
        alive.clear();
        alive.resize(n + 1, self.accept_only);
        if n > 0 {
            let mut state = self.last_ids[line[n - 1] as usize];
            alive[n - 1] = state;
            for i in (0..n - 1).rev() {
                let c = line[i];
                let mut next = self.states[state as usize].back[c as usize];
                if next == UNKNOWN {
                    next = self.step_back(state, c);
                }
                alive[i] = next;
                state = next;
            }
        }

        let (mut count, mut p) = (0, 0);
        while p <= n {
            let mut s = p;
            while 0 < s && s < n && !self.states[alive[s] as usize].starts {
                s += 1;
            }
            let mut current = self.entry(s, n, alive[s]);
            let first = &self.states[current as usize];
            if !first.accepts && !first.consumes {
                if s >= n {
                    break;
                }
                p = s + 1;
                continue;
            }
            let mut end = if first.accepts { s } else { usize::MAX };
            let mut i = s;
            while self.states[current as usize].consumes {
                i += 1;
                current = if i == n { self.accept_only } else { self.step(current, alive[i]) };
                if self.states[current as usize].accepts {
                    end = i;
                }
            }
            count += 1;
            p = if end > s { end } else { s + 1 };
        }
        count
    }
}

fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let newline = input.iter().position(|&b| b == b'\n').unwrap_or(input.len());
    let pattern = &input[..newline];
    let text = &input[(newline + 1).min(input.len())..];
    let mut lines: Vec<&[u8]> = text.split(|&b| b == b'\n').collect();
    lines.pop();

    let mut out = io::BufWriter::with_capacity(1 << 16, io::stdout().lock());
    let mut parser = Parser::new(pattern);
    match parser.parse() {
        Err(Invalid) => out.write_all(b"invalid\n").unwrap(),
        Ok(start) => {
            let mut matcher = Matcher::new(&parser, start);
            let mut alive = Vec::new();
            for line in lines {
                writeln!(out, "{}", matcher.count(line, &mut alive)).unwrap();
            }
        }
    }
    out.flush().unwrap();
}
