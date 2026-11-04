use std::collections::HashMap;
use std::io::{self, Read, Write};

const STACK_LIMIT: usize = 1_000_000;
const CALL_LIMIT: usize = 100_000;
const LOCALS: usize = 8;
const MEMORY: usize = 1 << 20;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Push,
    Drop,
    Dup,
    Swap,
    Over,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Jmp,
    Jz,
    Jnz,
    Call,
    Ret,
    Get,
    Set,
    Load,
    Store,
    Read,
    Print,
    Halt,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operand {
    None,
    Integer,
    Local,
    Label,
}

const MNEMONICS: [(&str, Op, Operand); 28] = [
    ("push", Op::Push, Operand::Integer),
    ("drop", Op::Drop, Operand::None),
    ("dup", Op::Dup, Operand::None),
    ("swap", Op::Swap, Operand::None),
    ("over", Op::Over, Operand::None),
    ("add", Op::Add, Operand::None),
    ("sub", Op::Sub, Operand::None),
    ("mul", Op::Mul, Operand::None),
    ("div", Op::Div, Operand::None),
    ("mod", Op::Mod, Operand::None),
    ("eq", Op::Eq, Operand::None),
    ("ne", Op::Ne, Operand::None),
    ("lt", Op::Lt, Operand::None),
    ("le", Op::Le, Operand::None),
    ("gt", Op::Gt, Operand::None),
    ("ge", Op::Ge, Operand::None),
    ("jmp", Op::Jmp, Operand::Label),
    ("jz", Op::Jz, Operand::Label),
    ("jnz", Op::Jnz, Operand::Label),
    ("call", Op::Call, Operand::Label),
    ("ret", Op::Ret, Operand::None),
    ("get", Op::Get, Operand::Local),
    ("set", Op::Set, Operand::Local),
    ("load", Op::Load, Operand::None),
    ("store", Op::Store, Operand::None),
    ("read", Op::Read, Operand::None),
    ("print", Op::Print, Operand::None),
    ("halt", Op::Halt, Operand::None),
];

/// `arg` is the literal of push, the local of get and set, or the target index of a jump or
/// call, which is -1 when the program never defines the label.
#[derive(Clone, Copy)]
struct Instruction {
    op: Op,
    arg: i64,
    line: usize,
}

struct VmError {
    message: &'static str,
    line: usize,
}

fn is_label(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Parses `-?[0-9]+` into a signed 64-bit value. The form is checked first because `parse`
/// also accepts a leading `+`.
fn parse_integer(token: &str) -> Option<i64> {
    let digits = token.strip_prefix('-').unwrap_or(token);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    token.parse().ok()
}

/// Turns source lines into instructions. Label operands resolve after the last line, so a
/// jump may name a label defined later.
fn assemble(source: &[&str]) -> Result<Vec<Instruction>, VmError> {
    let mut code = Vec::with_capacity(source.len() + 1);
    let mut targets = Vec::new();
    let mut labels: HashMap<&str, i64> = HashMap::new();
    for (index, text) in source.iter().enumerate() {
        let line = index + 1;
        let fail = |message| Err(VmError { message, line });
        let text = text.split(';').next().unwrap_or_default();
        let mut tokens = text.split([' ', '\t']).filter(|t| !t.is_empty()).peekable();
        while let Some(token) = tokens.next_if(|t| t.ends_with(':')) {
            let name = &token[..token.len() - 1];
            if !is_label(name) {
                return fail("bad label");
            }
            if labels.insert(name, code.len() as i64).is_some() {
                return fail("duplicate label");
            }
        }
        let Some(mnemonic) = tokens.next() else {
            continue;
        };
        let Some(&(_, op, kind)) = MNEMONICS
            .iter()
            .find(|(name, _, _)| name.eq_ignore_ascii_case(mnemonic))
        else {
            return fail("unknown instruction");
        };
        let operands: Vec<&str> = tokens.collect();
        let arg = match (kind, operands.as_slice()) {
            (Operand::None, []) => Some(0),
            (Operand::Integer, [token]) => parse_integer(token),
            (Operand::Local, [token]) => parse_integer(token).filter(|k| (0..LOCALS as i64).contains(k)),
            (Operand::Label, [token]) if is_label(token) => {
                targets.push((code.len(), *token));
                Some(0)
            }
            _ => None,
        };
        let Some(arg) = arg else {
            return fail("bad operand");
        };
        code.push(Instruction { op, arg, line });
    }
    for (index, name) in targets {
        code[index].arg = labels.get(name).copied().unwrap_or(-1);
    }
    // Running off the end halts, and a label after the last instruction points here.
    code.push(Instruction {
        op: Op::Halt,
        arg: 0,
        line: 0,
    });
    Ok(code)
}

/// Runs the program, printing to `out`, until it halts or an instruction fails.
fn execute(code: &[Instruction], data: &[i64], out: &mut Vec<u8>) -> Result<(), VmError> {
    let mut stack = vec![0i64; STACK_LIMIT];
    let mut frames = vec![0i64; LOCALS * (CALL_LIMIT + 1)];
    let mut memory = vec![0i64; MEMORY];
    let mut calls = Vec::with_capacity(CALL_LIMIT);
    let (mut pc, mut sp, mut base, mut cursor) = (0usize, 0usize, 0usize, 0usize);
    loop {
        let Instruction { op, arg, line } = code[pc];
        pc += 1;
        let fail = |message| Err(VmError { message, line });
        match op {
            Op::Push => {
                if sp == STACK_LIMIT {
                    return fail("stack overflow");
                }
                stack[sp] = arg;
                sp += 1;
            }
            Op::Drop => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                sp -= 1;
            }
            Op::Dup => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                if sp == STACK_LIMIT {
                    return fail("stack overflow");
                }
                stack[sp] = stack[sp - 1];
                sp += 1;
            }
            Op::Swap => {
                if sp < 2 {
                    return fail("stack underflow");
                }
                stack.swap(sp - 1, sp - 2);
            }
            Op::Over => {
                if sp < 2 {
                    return fail("stack underflow");
                }
                if sp == STACK_LIMIT {
                    return fail("stack overflow");
                }
                stack[sp] = stack[sp - 2];
                sp += 1;
            }
            Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Mod | Op::Eq | Op::Ne | Op::Lt
            | Op::Le | Op::Gt | Op::Ge => {
                if sp < 2 {
                    return fail("stack underflow");
                }
                let (a, b) = (stack[sp - 2], stack[sp - 1]);
                if b == 0 && matches!(op, Op::Div | Op::Mod) {
                    return fail("division by zero");
                }
                stack[sp - 2] = match op {
                    Op::Add => a.wrapping_add(b),
                    Op::Sub => a.wrapping_sub(b),
                    Op::Mul => a.wrapping_mul(b),
                    Op::Div => a.wrapping_div(b),
                    Op::Mod => a.wrapping_rem(b),
                    Op::Eq => (a == b) as i64,
                    Op::Ne => (a != b) as i64,
                    Op::Lt => (a < b) as i64,
                    Op::Le => (a <= b) as i64,
                    Op::Gt => (a > b) as i64,
                    _ => (a >= b) as i64,
                };
                sp -= 1;
            }
            Op::Jmp => {
                if arg < 0 {
                    return fail("undefined label");
                }
                pc = arg as usize;
            }
            Op::Jz | Op::Jnz => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                sp -= 1;
                if (stack[sp] == 0) == (op == Op::Jz) {
                    if arg < 0 {
                        return fail("undefined label");
                    }
                    pc = arg as usize;
                }
            }
            Op::Call => {
                if arg < 0 {
                    return fail("undefined label");
                }
                if calls.len() == CALL_LIMIT {
                    return fail("call stack overflow");
                }
                calls.push(pc);
                base += LOCALS;
                frames[base..base + LOCALS].fill(0);
                pc = arg as usize;
            }
            Op::Ret => {
                let Some(back) = calls.pop() else {
                    return fail("return without call");
                };
                pc = back;
                base -= LOCALS;
            }
            Op::Get => {
                if sp == STACK_LIMIT {
                    return fail("stack overflow");
                }
                stack[sp] = frames[base + arg as usize];
                sp += 1;
            }
            Op::Set => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                sp -= 1;
                frames[base + arg as usize] = stack[sp];
            }
            Op::Load => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                let Some(&value) = usize::try_from(stack[sp - 1]).ok().and_then(|a| memory.get(a))
                else {
                    return fail("bad address");
                };
                stack[sp - 1] = value;
            }
            Op::Store => {
                if sp < 2 {
                    return fail("stack underflow");
                }
                let Some(cell) = usize::try_from(stack[sp - 1]).ok().and_then(|a| memory.get_mut(a))
                else {
                    return fail("bad address");
                };
                *cell = stack[sp - 2];
                sp -= 2;
            }
            Op::Read => {
                let Some(&value) = data.get(cursor) else {
                    return fail("end of input");
                };
                if sp == STACK_LIMIT {
                    return fail("stack overflow");
                }
                cursor += 1;
                stack[sp] = value;
                sp += 1;
            }
            Op::Print => {
                if sp < 1 {
                    return fail("stack underflow");
                }
                sp -= 1;
                writeln!(out, "{}", stack[sp]).unwrap();
            }
            Op::Halt => return Ok(()),
        }
    }
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut lines: Vec<&str> = input.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    // The source ends at the first line that is exactly ".input", and the data follows it.
    let (source, data) = match lines.iter().position(|&l| l == ".input") {
        Some(at) => (&lines[..at], &lines[at + 1..]),
        None => (&lines[..], &[][..]),
    };
    let data: Vec<i64> = data
        .iter()
        .flat_map(|l| l.split([' ', '\t']))
        .filter(|t| !t.is_empty())
        .map(|t| t.parse().unwrap())
        .collect();

    let mut out = Vec::new();
    let result = assemble(source).and_then(|code| execute(&code, &data, &mut out));
    if let Err(VmError { message, line }) = result {
        writeln!(out, "error: {message} at line {line}").unwrap();
    }
    io::stdout().lock().write_all(&out).unwrap();
}
