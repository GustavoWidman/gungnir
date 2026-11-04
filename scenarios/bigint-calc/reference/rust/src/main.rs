use std::cmp::Ordering;
use std::io::{self, Read, Write};

/// Limbs are base 10^9, so reading and printing decimal take linear time.
const BASE: u32 = 1_000_000_000;
const BASE64: u64 = BASE as u64;
const BASE_DIGITS: usize = 9;
/// Below this many limbs in the shorter operand, schoolbook multiplication beats Karatsuba.
const KARATSUBA_CUTOFF: usize = 40;

/// A signed integer: little-endian limbs with no high zero limb, so zero has no limbs and is
/// never negative.
struct Int {
    limbs: Vec<u32>,
    negative: bool,
}

fn trimmed(a: &[u32]) -> &[u32] {
    let n = a.iter().rposition(|&limb| limb != 0).map_or(0, |i| i + 1);
    &a[..n]
}

fn compare(a: &[u32], b: &[u32]) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.iter().rev().cmp(b.iter().rev()))
}

/// r += a, for a no longer than r and a sum that fits in r.
fn add_into(r: &mut [u32], a: &[u32]) {
    let mut carry = false;
    for (x, &y) in r.iter_mut().zip(a) {
        let s = *x + y + carry as u32;
        carry = s >= BASE;
        *x = if carry { s - BASE } else { s };
    }
    for x in &mut r[a.len()..] {
        if !carry {
            break;
        }
        carry = *x == BASE - 1;
        *x = if carry { 0 } else { *x + 1 };
    }
}

/// r -= a, for a no longer than r and r >= a.
fn sub_into(r: &mut [u32], a: &[u32]) {
    let mut borrow = false;
    for (x, &y) in r.iter_mut().zip(a) {
        let s = y + borrow as u32;
        borrow = *x < s;
        *x = if borrow { *x + BASE - s } else { *x - s };
    }
    for x in &mut r[a.len()..] {
        if !borrow {
            break;
        }
        borrow = *x == 0;
        *x = if borrow { BASE - 1 } else { *x - 1 };
    }
}

fn mul_schoolbook(r: &mut [u32], a: &[u32], b: &[u32]) {
    r.fill(0);
    for (i, &x) in a.iter().enumerate() {
        let mut carry = 0;
        for (j, &y) in b.iter().enumerate() {
            let t = r[i + j] as u64 + x as u64 * y as u64 + carry;
            r[i + j] = (t % BASE64) as u32;
            carry = t / BASE64;
        }
        r[i + b.len()] = carry as u32;
    }
}

/// r = a * b, where r has a.len() + b.len() limbs. The operands may have high zero limbs.
fn mul_into(r: &mut [u32], a: &[u32], b: &[u32]) {
    let (a, b) = if a.len() < b.len() { (b, a) } else { (a, b) };
    let (an, bn) = (a.len(), b.len());
    if bn < KARATSUBA_CUTOFF {
        mul_schoolbook(r, a, b);
        return;
    }
    if an >= 2 * bn {
        // Unbalanced: multiply b by each bn-limb slice of a and add the products.
        r.fill(0);
        let mut t = vec![0; 2 * bn];
        for (i, slice) in a.chunks(bn).enumerate() {
            let t = &mut t[..slice.len() + bn];
            mul_into(t, slice, b);
            add_into(&mut r[i * bn..], t);
        }
        return;
    }
    // Karatsuba with a = a1 B^m + a0 and b = b1 B^m + b0, where m < bn <= an < 2 bn.
    let m = an / 2;
    let (a0, a1) = a.split_at(m);
    let (b0, b1) = b.split_at(m);
    let mut sa = vec![0; a1.len() + 1];
    sa[..a1.len()].copy_from_slice(a1);
    add_into(&mut sa, a0);
    let mut sb = vec![0; m.max(b1.len()) + 1];
    sb[..m].copy_from_slice(b0);
    add_into(&mut sb, b1);
    let (low, high) = r.split_at_mut(2 * m);
    mul_into(low, a0, b0);
    mul_into(high, a1, b1);
    let mut z = vec![0; sa.len() + sb.len()];
    mul_into(&mut z, &sa, &sb);
    sub_into(&mut z, &r[..2 * m]);
    sub_into(&mut z, &r[2 * m..]);
    add_into(&mut r[m..], trimmed(&z));
}

/// a * k for a single limb k, with one more limb than a.
fn mul_limb(a: &[u32], k: u32) -> Vec<u32> {
    let mut r = Vec::with_capacity(a.len() + 1);
    let mut carry = 0;
    for &x in a {
        let t = x as u64 * k as u64 + carry;
        r.push((t % BASE64) as u32);
        carry = t / BASE64;
    }
    r.push(carry as u32);
    r
}

/// (u / v, u % v) for u no shorter than v and v without high zero limbs: Knuth's Algorithm D
/// (TAOCP 4.3.1).
fn divmod(u: &[u32], v: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let (un, vn) = (u.len(), v.len());
    let mut q = vec![0; un - vn + 1];
    if vn == 1 {
        let d = v[0] as u64;
        let mut rem = 0;
        for i in (0..un).rev() {
            let cur = rem * BASE64 + u[i] as u64;
            q[i] = (cur / d) as u32;
            rem = cur % d;
        }
        return (q, vec![rem as u32]);
    }
    // Scale both so the divisor's top limb is at least BASE / 2, which keeps each estimated
    // quotient limb at most one too large after the two-limb test.
    let d = (BASE64 / (v[vn - 1] as u64 + 1)) as u32;
    let mut w = mul_limb(u, d);
    let y = mul_limb(v, d);
    let (top, next) = (y[vn - 1] as u64, y[vn - 2] as u64);
    for j in (0..=un - vn).rev() {
        let num = w[j + vn] as u64 * BASE64 + w[j + vn - 1] as u64;
        let (mut qhat, mut rhat) = (num / top, num % top);
        while qhat >= BASE64 || qhat * next > rhat * BASE64 + w[j + vn - 2] as u64 {
            qhat -= 1;
            rhat += top;
            if rhat >= BASE64 {
                break;
            }
        }
        let (mut carry, mut borrow) = (0, 0);
        for i in 0..vn {
            let p = qhat * y[i] as u64 + carry;
            carry = p / BASE64;
            let t = w[i + j] as i64 - (p % BASE64) as i64 - borrow;
            borrow = (t < 0) as i64;
            w[i + j] = (if t < 0 { t + BASE64 as i64 } else { t }) as u32;
        }
        let mut t = w[j + vn] as i64 - carry as i64 - borrow;
        if t < 0 {
            // The estimate was one too large, which is rare: add the divisor back.
            qhat -= 1;
            let mut c = false;
            for i in 0..vn {
                let s = w[i + j] + y[i] + c as u32;
                c = s >= BASE;
                w[i + j] = if c { s - BASE } else { s };
            }
            t += c as i64;
        }
        w[j + vn] = t as u32;
        q[j] = qhat as u32;
    }
    let mut r = vec![0; vn];
    let mut rem = 0;
    for i in (0..vn).rev() {
        let cur = rem * BASE64 + w[i] as u64;
        r[i] = (cur / d as u64) as u32;
        rem = cur % d as u64;
    }
    (q, r)
}

impl Int {
    fn new(mut limbs: Vec<u32>, negative: bool) -> Int {
        limbs.truncate(trimmed(&limbs).len());
        let negative = negative && !limbs.is_empty();
        Int { limbs, negative }
    }

    fn small(value: u32, negative: bool) -> Int {
        Int::new(vec![value], negative)
    }

    fn parse(digits: &[u8]) -> Int {
        let start = digits.iter().position(|&c| c != b'0').unwrap_or(digits.len());
        let limbs = digits[start..]
            .rchunks(BASE_DIGITS)
            .map(|chunk| chunk.iter().fold(0, |limb, &c| limb * 10 + (c - b'0') as u32))
            .collect();
        Int::new(limbs, false)
    }

    /// self + b, or self - b when subtract is set.
    fn add(&self, b: &Int, subtract: bool) -> Int {
        let bneg = b.negative != subtract;
        let (mut a, mut b, mut negative) = (self, b, self.negative);
        if a.negative == bneg {
            if a.limbs.len() < b.limbs.len() {
                (a, b) = (b, a);
            }
            let mut r = a.limbs.clone();
            r.push(0);
            add_into(&mut r, &b.limbs);
            return Int::new(r, negative);
        }
        // Opposite signs: subtract the smaller magnitude from the larger.
        if compare(&a.limbs, &b.limbs) == Ordering::Less {
            (a, b, negative) = (b, a, bneg);
        }
        let mut r = a.limbs.clone();
        sub_into(&mut r, &b.limbs);
        Int::new(r, negative)
    }

    fn mul(&self, b: &Int) -> Int {
        if self.limbs.is_empty() || b.limbs.is_empty() {
            return Int::small(0, false);
        }
        let mut r = vec![0; self.limbs.len() + b.limbs.len()];
        mul_into(&mut r, &self.limbs, &b.limbs);
        Int::new(r, self.negative != b.negative)
    }

    /// The quotient rounded toward negative infinity, or the remainder that goes with it. b is
    /// nonzero.
    fn divide(&self, b: &Int, remainder: bool) -> Int {
        let (mut q, mut r) = if self.limbs.len() < b.limbs.len() {
            (vec![], self.limbs.clone())
        } else {
            divmod(&self.limbs, &b.limbs)
        };
        if !trimmed(&r).is_empty() && self.negative != b.negative {
            // Truncation rounded toward zero, so step the quotient down by one and move the
            // remainder to the divisor's sign: |r| becomes |b| - |r|.
            q.push(0);
            add_into(&mut q, &[1]);
            let mut flipped = b.limbs.clone();
            sub_into(&mut flipped, trimmed(&r));
            r = flipped;
        }
        if remainder {
            Int::new(r, b.negative)
        } else {
            Int::new(q, self.negative != b.negative)
        }
    }

    /// self ^ e for e >= 0.
    fn pow(&self, e: &Int) -> Int {
        if self.limbs.is_empty() {
            return Int::small(e.limbs.is_empty() as u32, false);
        }
        if self.limbs == [1] {
            return Int::small(1, self.negative && e.limbs.first().is_some_and(|&x| x & 1 == 1));
        }
        // Any other base keeps the exponent below 664386, so it fits in one limb.
        let n = e.limbs.first().copied().unwrap_or(0);
        let mut result = Int::small(1, false);
        for bit in (0..u32::BITS - n.leading_zeros()).rev() {
            result = result.mul(&result);
            if (n >> bit) & 1 == 1 {
                result = result.mul(self);
            }
        }
        result
    }

    fn write(&self, out: &mut Vec<u8>) {
        let Some((&top, rest)) = self.limbs.split_last() else {
            out.push(b'0');
            return;
        };
        if self.negative {
            out.push(b'-');
        }
        out.extend_from_slice(top.to_string().as_bytes());
        for &limb in rest.iter().rev() {
            let mut digits = [b'0'; BASE_DIGITS];
            let mut v = limb;
            for d in digits.iter_mut().rev() {
                *d = b'0' + (v % 10) as u8;
                v /= 10;
            }
            out.extend_from_slice(&digits);
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Number,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Neg,
    Pow,
    Open,
}

impl Kind {
    /// Binding strength. `^` and unary minus group from the right, the others from the left.
    fn rank(self) -> u8 {
        match self {
            Kind::Add | Kind::Sub => 1,
            Kind::Mul | Kind::Div | Kind::Mod => 2,
            Kind::Neg => 3,
            Kind::Pow => 4,
            Kind::Number | Kind::Open => 0,
        }
    }
}

#[derive(Clone, Copy)]
struct Item<'a> {
    kind: Kind,
    digits: &'a [u8],
}

impl Item<'_> {
    fn op(kind: Kind) -> Self {
        Item { kind, digits: &[] }
    }
}

/// Converts a line to postfix order with the shunting-yard algorithm, or returns None when it
/// does not match the grammar.
fn postfix(line: &[u8]) -> Option<Vec<Item<'_>>> {
    let mut items = Vec::new();
    let mut ops: Vec<Item> = Vec::new();
    let mut want_operand = true;
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c == b' ' || c == b'\t' {
            i += 1;
            continue;
        }
        if c.is_ascii_digit() {
            if !want_operand {
                return None;
            }
            let start = i;
            while i < line.len() && line[i].is_ascii_digit() {
                i += 1;
            }
            items.push(Item { kind: Kind::Number, digits: &line[start..i] });
            want_operand = false;
            continue;
        }
        i += 1;
        if want_operand {
            match c {
                b'(' => ops.push(Item::op(Kind::Open)),
                b'-' => ops.push(Item::op(Kind::Neg)),
                _ => return None,
            }
            continue;
        }
        if c == b')' {
            loop {
                let top = ops.pop()?;
                if top.kind == Kind::Open {
                    break;
                }
                items.push(top);
            }
            continue;
        }
        let kind = match c {
            b'+' => Kind::Add,
            b'-' => Kind::Sub,
            b'*' => Kind::Mul,
            b'/' => Kind::Div,
            b'%' => Kind::Mod,
            b'^' => Kind::Pow,
            _ => return None,
        };
        while let Some(&top) = ops.last() {
            if top.kind == Kind::Open
                || top.kind.rank() < kind.rank()
                || (top.kind.rank() == kind.rank() && kind == Kind::Pow)
            {
                break;
            }
            items.push(top);
            ops.pop();
        }
        ops.push(Item::op(kind));
        want_operand = true;
    }
    if want_operand {
        return None;
    }
    while let Some(top) = ops.pop() {
        if top.kind == Kind::Open {
            return None;
        }
        items.push(top);
    }
    Some(items)
}

/// Appends the line's value or error message to out.
fn evaluate(line: &[u8], out: &mut Vec<u8>) {
    let Some(items) = postfix(line) else {
        out.extend_from_slice(b"syntax error");
        return;
    };
    let mut values: Vec<Int> = Vec::new();
    for item in items {
        match item.kind {
            Kind::Number => values.push(Int::parse(item.digits)),
            Kind::Neg => {
                let x = values.last_mut().expect("an operand");
                x.negative = !x.negative && !x.limbs.is_empty();
            }
            kind => {
                let b = values.pop().expect("a right operand");
                let a = values.pop().expect("a left operand");
                let r = match kind {
                    Kind::Add => a.add(&b, false),
                    Kind::Sub => a.add(&b, true),
                    Kind::Mul => a.mul(&b),
                    Kind::Pow if b.negative => {
                        out.extend_from_slice(b"negative exponent");
                        return;
                    }
                    Kind::Pow => a.pow(&b),
                    _ if b.limbs.is_empty() => {
                        out.extend_from_slice(b"division by zero");
                        return;
                    }
                    _ => a.divide(&b, kind == Kind::Mod),
                };
                values.push(r);
            }
        }
    }
    values[0].write(out);
}

fn main() {
    let mut input = Vec::new();
    io::stdin().read_to_end(&mut input).unwrap();
    let mut out = Vec::new();
    let mut lines = input.split(|&c| c == b'\n');
    // Every line ends with a line feed, so the piece after the last one is empty.
    lines.next_back();
    for line in lines {
        evaluate(line, &mut out);
        out.push(b'\n');
    }
    io::stdout().lock().write_all(&out).unwrap();
}
