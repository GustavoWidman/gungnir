const std = @import("std");

const stack_limit = 1_000_000;
const call_limit = 100_000;
const locals = 8;
const memory_size = 1 << 20;

const Op = enum(u8) {
    push, drop, dup, swap, over, add, sub, mul, div, mod, eq, ne, lt, le, gt, ge,
    jmp, jz, jnz, call, ret, get, set, load, store, read, print, halt,
};

const Operand = enum { none, integer, local, label };

const Mnemonic = struct { name: []const u8, op: Op, operand: Operand };

const mnemonics = [_]Mnemonic{
    .{ .name = "push", .op = .push, .operand = .integer }, .{ .name = "drop", .op = .drop, .operand = .none },
    .{ .name = "dup", .op = .dup, .operand = .none },      .{ .name = "swap", .op = .swap, .operand = .none },
    .{ .name = "over", .op = .over, .operand = .none },    .{ .name = "add", .op = .add, .operand = .none },
    .{ .name = "sub", .op = .sub, .operand = .none },      .{ .name = "mul", .op = .mul, .operand = .none },
    .{ .name = "div", .op = .div, .operand = .none },      .{ .name = "mod", .op = .mod, .operand = .none },
    .{ .name = "eq", .op = .eq, .operand = .none },        .{ .name = "ne", .op = .ne, .operand = .none },
    .{ .name = "lt", .op = .lt, .operand = .none },        .{ .name = "le", .op = .le, .operand = .none },
    .{ .name = "gt", .op = .gt, .operand = .none },        .{ .name = "ge", .op = .ge, .operand = .none },
    .{ .name = "jmp", .op = .jmp, .operand = .label },     .{ .name = "jz", .op = .jz, .operand = .label },
    .{ .name = "jnz", .op = .jnz, .operand = .label },     .{ .name = "call", .op = .call, .operand = .label },
    .{ .name = "ret", .op = .ret, .operand = .none },      .{ .name = "get", .op = .get, .operand = .local },
    .{ .name = "set", .op = .set, .operand = .local },     .{ .name = "load", .op = .load, .operand = .none },
    .{ .name = "store", .op = .store, .operand = .none },  .{ .name = "read", .op = .read, .operand = .none },
    .{ .name = "print", .op = .print, .operand = .none },  .{ .name = "halt", .op = .halt, .operand = .none },
};

/// arg is the literal of push, the local of get and set, or the target index of a jump or
/// call, which is -1 when the program never defines the label.
const Instruction = struct { op: Op, line: usize, arg: i64 };

const Failure = struct { message: []const u8, line: usize };

fn isLabel(name: []const u8) bool {
    if (name.len == 0) return false;
    for (name, 0..) |c, i| {
        const letter = std.ascii.isAlphabetic(c) or c == '_';
        if (!letter and !(i > 0 and std.ascii.isDigit(c))) return false;
    }
    return true;
}

/// Parses -?[0-9]+ into a signed 64-bit value. The form is checked first because parseInt
/// also accepts a leading + and underscores.
fn parseInteger(token: []const u8) ?i64 {
    const digits = if (std.mem.startsWith(u8, token, "-")) token[1..] else token;
    if (digits.len == 0) return null;
    for (digits) |c| if (!std.ascii.isDigit(c)) return null;
    return std.fmt.parseInt(i64, token, 10) catch null;
}

const Assembled = union(enum) { code: []Instruction, failure: Failure };

/// Assembles the source lines. Label operands resolve after the last line, so a jump may name
/// a label defined later.
fn assemble(arena: std.mem.Allocator, source: []const []const u8) !Assembled {
    var code: std.ArrayList(Instruction) = .empty;
    const Target = struct { index: usize, name: []const u8 };
    var targets: std.ArrayList(Target) = .empty;
    var labels: std.StringHashMapUnmanaged(i64) = .empty;
    for (source, 1..) |raw, line| {
        const fail = struct {
            fn at(message: []const u8, number: usize) Assembled {
                return .{ .failure = .{ .message = message, .line = number } };
            }
        }.at;
        const text = raw[0 .. std.mem.indexOfScalar(u8, raw, ';') orelse raw.len];
        var tokens = std.mem.tokenizeAny(u8, text, " \t");
        var token = tokens.next();
        while (token) |t| : (token = tokens.next()) {
            if (!std.mem.endsWith(u8, t, ":")) break;
            const name = t[0 .. t.len - 1];
            if (!isLabel(name)) return fail("bad label", line);
            const slot = try labels.getOrPut(arena, name);
            if (slot.found_existing) return fail("duplicate label", line);
            slot.value_ptr.* = @intCast(code.items.len);
        }
        const word = token orelse continue;
        const mnemonic = for (mnemonics) |m| {
            if (std.ascii.eqlIgnoreCase(m.name, word)) break m;
        } else return fail("unknown instruction", line);
        const operand = tokens.next();
        const extra = tokens.next() != null;
        var in: Instruction = .{ .op = mnemonic.op, .line = line, .arg = 0 };
        var valid = !extra and (operand != null) == (mnemonic.operand != .none);
        if (valid) switch (mnemonic.operand) {
            .none => {},
            .integer => if (parseInteger(operand.?)) |v| {
                in.arg = v;
            } else {
                valid = false;
            },
            .local => if (parseInteger(operand.?)) |v| {
                in.arg = v;
                valid = v >= 0 and v < locals;
            } else {
                valid = false;
            },
            .label => {
                valid = isLabel(operand.?);
                try targets.append(arena, .{ .index = code.items.len, .name = operand.? });
            },
        };
        if (!valid) return fail("bad operand", line);
        try code.append(arena, in);
    }
    for (targets.items) |t| code.items[t.index].arg = labels.get(t.name) orelse -1;
    // Running off the end halts, and a label after the last instruction points here.
    try code.append(arena, .{ .op = .halt, .line = 0, .arg = 0 });
    return .{ .code = code.items };
}

/// Runs the program, printing to out, until it halts or an instruction fails.
fn execute(arena: std.mem.Allocator, code: []const Instruction, data: []const i64, out: *std.Io.Writer) !?Failure {
    const stack = try arena.alloc(i64, stack_limit);
    const frames = try arena.alloc(i64, locals * (call_limit + 1));
    const memory = try arena.alloc(i64, memory_size);
    @memset(frames[0..locals], 0);
    @memset(memory, 0);
    const calls = try arena.alloc(usize, call_limit);
    var pc: usize = 0;
    var sp: usize = 0;
    var depth: usize = 0;
    var base: usize = 0;
    var cursor: usize = 0;
    run: while (true) {
        const in = code[pc];
        pc += 1;
        const failure: []const u8 = fail: switch (in.op) {
            .push => {
                if (sp == stack_limit) break :fail "stack overflow";
                stack[sp] = in.arg;
                sp += 1;
                continue :run;
            },
            .drop => {
                if (sp < 1) break :fail "stack underflow";
                sp -= 1;
                continue :run;
            },
            .dup => {
                if (sp < 1) break :fail "stack underflow";
                if (sp == stack_limit) break :fail "stack overflow";
                stack[sp] = stack[sp - 1];
                sp += 1;
                continue :run;
            },
            .swap => {
                if (sp < 2) break :fail "stack underflow";
                std.mem.swap(i64, &stack[sp - 1], &stack[sp - 2]);
                continue :run;
            },
            .over => {
                if (sp < 2) break :fail "stack underflow";
                if (sp == stack_limit) break :fail "stack overflow";
                stack[sp] = stack[sp - 2];
                sp += 1;
                continue :run;
            },
            .add, .sub, .mul, .div, .mod, .eq, .ne, .lt, .le, .gt, .ge => {
                if (sp < 2) break :fail "stack underflow";
                const a = stack[sp - 2];
                const b = stack[sp - 1];
                stack[sp - 2] = switch (in.op) {
                    .add => a +% b,
                    .sub => a -% b,
                    .mul => a *% b,
                    .div, .mod => blk: {
                        if (b == 0) break :fail "division by zero";
                        // minInt(i64) / -1 overflows, and the wrapped results are -a and 0.
                        if (b == -1) break :blk if (in.op == .div) 0 -% a else 0;
                        break :blk if (in.op == .div) @divTrunc(a, b) else @rem(a, b);
                    },
                    .eq => @intFromBool(a == b),
                    .ne => @intFromBool(a != b),
                    .lt => @intFromBool(a < b),
                    .le => @intFromBool(a <= b),
                    .gt => @intFromBool(a > b),
                    else => @intFromBool(a >= b),
                };
                sp -= 1;
                continue :run;
            },
            .jmp => {
                if (in.arg < 0) break :fail "undefined label";
                pc = @intCast(in.arg);
                continue :run;
            },
            .jz, .jnz => {
                if (sp < 1) break :fail "stack underflow";
                sp -= 1;
                if ((stack[sp] == 0) == (in.op == .jz)) {
                    if (in.arg < 0) break :fail "undefined label";
                    pc = @intCast(in.arg);
                }
                continue :run;
            },
            .call => {
                if (in.arg < 0) break :fail "undefined label";
                if (depth == call_limit) break :fail "call stack overflow";
                calls[depth] = pc;
                depth += 1;
                base += locals;
                @memset(frames[base .. base + locals], 0);
                pc = @intCast(in.arg);
                continue :run;
            },
            .ret => {
                if (depth == 0) break :fail "return without call";
                depth -= 1;
                pc = calls[depth];
                base -= locals;
                continue :run;
            },
            .get => {
                if (sp == stack_limit) break :fail "stack overflow";
                stack[sp] = frames[base + @as(usize, @intCast(in.arg))];
                sp += 1;
                continue :run;
            },
            .set => {
                if (sp < 1) break :fail "stack underflow";
                sp -= 1;
                frames[base + @as(usize, @intCast(in.arg))] = stack[sp];
                continue :run;
            },
            .load => {
                if (sp < 1) break :fail "stack underflow";
                const address = stack[sp - 1];
                if (address < 0 or address >= memory_size) break :fail "bad address";
                stack[sp - 1] = memory[@intCast(address)];
                continue :run;
            },
            .store => {
                if (sp < 2) break :fail "stack underflow";
                const address = stack[sp - 1];
                if (address < 0 or address >= memory_size) break :fail "bad address";
                memory[@intCast(address)] = stack[sp - 2];
                sp -= 2;
                continue :run;
            },
            .read => {
                if (cursor == data.len) break :fail "end of input";
                if (sp == stack_limit) break :fail "stack overflow";
                stack[sp] = data[cursor];
                cursor += 1;
                sp += 1;
                continue :run;
            },
            .print => {
                if (sp < 1) break :fail "stack underflow";
                sp -= 1;
                try out.print("{d}\n", .{stack[sp]});
                continue :run;
            },
            .halt => return null,
        };
        return .{ .message = failure, .line = in.line };
    }
}

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    var lines: std.ArrayList([]const u8) = .empty;
    var it = std.mem.splitScalar(u8, input, '\n');
    while (it.next()) |line| try lines.append(arena, line);
    if (lines.items.len > 0 and lines.items[lines.items.len - 1].len == 0) _ = lines.pop();

    // The source ends at the first line that is exactly ".input", and the data follows it.
    var source = lines.items;
    var data: std.ArrayList(i64) = .empty;
    for (lines.items, 0..) |line, i| {
        if (!std.mem.eql(u8, line, ".input")) continue;
        source = lines.items[0..i];
        for (lines.items[i + 1 ..]) |rest| {
            var tokens = std.mem.tokenizeAny(u8, rest, " \t");
            while (tokens.next()) |token| try data.append(arena, try std.fmt.parseInt(i64, token, 10));
        }
        break;
    }

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;
    const failure = switch (try assemble(arena, source)) {
        .failure => |f| f,
        .code => |code| try execute(arena, code, data.items, out),
    };
    if (failure) |f| try out.print("error: {s} at line {d}\n", .{ f.message, f.line });
    try out.flush();
}
