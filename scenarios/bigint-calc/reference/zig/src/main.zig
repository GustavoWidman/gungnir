const std = @import("std");
const Allocator = std.mem.Allocator;
const Managed = std.math.big.int.Managed;

/// The standard library's setString and toString take quadratic time in the number of digits,
/// so long numbers are split in halves at powers of ten: 10^chunk, 10^(2 chunk), 10^(4 chunk)
/// and so on.
const chunk = 512;
/// Numbers of at most this many limbs are converted by the standard library directly.
const small_limbs = 64;

const Kind = enum { number, add, sub, mul, div, mod, neg, pow, open };

/// Binding strength. `^` and unary minus group from the right, the others from the left.
fn rank(kind: Kind) u8 {
    return switch (kind) {
        .add, .sub => 1,
        .mul, .div, .mod => 2,
        .neg => 3,
        .pow => 4,
        .number, .open => 0,
    };
}

const Item = struct {
    kind: Kind,
    digits: []const u8 = "",
};

/// Converts a line to postfix order with the shunting-yard algorithm, or returns null when it
/// does not match the grammar.
fn postfix(arena: Allocator, line: []const u8) !?[]Item {
    var items: std.ArrayList(Item) = .empty;
    var ops: std.ArrayList(Item) = .empty;
    var want_operand = true;
    var i: usize = 0;
    while (i < line.len) {
        const c = line[i];
        if (c == ' ' or c == '\t') {
            i += 1;
            continue;
        }
        if (std.ascii.isDigit(c)) {
            if (!want_operand) return null;
            const start = i;
            while (i < line.len and std.ascii.isDigit(line[i])) i += 1;
            try items.append(arena, .{ .kind = .number, .digits = line[start..i] });
            want_operand = false;
            continue;
        }
        i += 1;
        if (want_operand) {
            switch (c) {
                '(' => try ops.append(arena, .{ .kind = .open }),
                '-' => try ops.append(arena, .{ .kind = .neg }),
                else => return null,
            }
            continue;
        }
        if (c == ')') {
            while (true) {
                const top = ops.pop() orelse return null;
                if (top.kind == .open) break;
                try items.append(arena, top);
            }
            continue;
        }
        const kind: Kind = switch (c) {
            '+' => .add,
            '-' => .sub,
            '*' => .mul,
            '/' => .div,
            '%' => .mod,
            '^' => .pow,
            else => return null,
        };
        while (ops.items.len > 0) {
            const top = ops.items[ops.items.len - 1];
            if (top.kind == .open or rank(top.kind) < rank(kind) or (rank(top.kind) == rank(kind) and kind == .pow)) break;
            try items.append(arena, top);
            _ = ops.pop();
        }
        try ops.append(arena, .{ .kind = kind });
        want_operand = true;
    }
    if (want_operand) return null;
    while (ops.pop()) |top| {
        if (top.kind == .open) return null;
        try items.append(arena, top);
    }
    return items.items;
}

fn width(k: usize) usize {
    return @as(usize, chunk) << @intCast(k);
}

fn isNegative(x: *const Managed) bool {
    return !x.isPositive() and !x.eqlZero();
}

fn power(r: *Managed, a: *const Managed, b: *const Managed) !void {
    if (isNegative(b)) return error.NegativeExponent;
    // The exponent is unbounded only for these bases, whose powers need no multiplication.
    if (a.eqlZero()) return r.set(@intFromBool(b.eqlZero()));
    if (a.toConst().abs().orderAgainstScalar(1) == .eq) {
        return r.set(@as(i8, if (isNegative(a) and b.isOdd()) -1 else 1));
    }
    // Any other base keeps the exponent below 664386.
    try r.pow(a, try b.toConst().toInt(u32));
}

const Calculator = struct {
    /// Owns the cached powers of ten, which every line shares.
    gpa: Allocator,
    /// Holds one line's values; it is reset after each line.
    arena: Allocator,
    tens: std.ArrayList(Managed) = .empty,

    /// 10^width(k). The pointer is valid until the next call.
    fn ten(self: *Calculator, k: usize) !*const Managed {
        while (self.tens.items.len <= k) {
            var next = try Managed.init(self.gpa);
            if (self.tens.items.len == 0) {
                var base = try Managed.initSet(self.gpa, 10);
                defer base.deinit();
                try next.pow(&base, chunk);
            } else {
                try next.sqr(&self.tens.items[self.tens.items.len - 1]);
            }
            try self.tens.append(self.gpa, next);
        }
        return &self.tens.items[k];
    }

    fn parse(self: *Calculator, digits: []const u8) !Managed {
        var z = try Managed.init(self.arena);
        if (digits.len <= chunk) {
            if (digits.len > 0) try z.setString(10, digits);
            return z;
        }
        var k: usize = 0;
        while (width(k + 1) < digits.len) k += 1;
        const split = digits.len - width(k);
        const high = try self.parse(digits[0..split]);
        const low = try self.parse(digits[split..]);
        try z.mul(&high, try self.ten(k));
        try z.add(&z, &low);
        return z;
    }

    /// Writes x >= 0 in decimal, zero-padded to exactly `pad` digits when `pad` is not zero.
    fn emit(self: *Calculator, out: *std.Io.Writer, x: *const Managed, pad: usize) !void {
        if (x.len() <= small_limbs) {
            const digits = try x.toString(self.arena, 10, .lower);
            if (pad > digits.len) try out.splatByteAll('0', pad - digits.len);
            return out.writeAll(digits);
        }
        // The divisor has at most half of x's limbs, so the quotient is nonzero.
        var k: usize = 0;
        while ((try self.ten(k + 1)).len() * 2 <= x.len()) k += 1;
        var q = try Managed.init(self.arena);
        var r = try Managed.init(self.arena);
        try q.divTrunc(&r, x, try self.ten(k));
        try self.emit(out, &q, if (pad > 0) pad - width(k) else 0);
        try self.emit(out, &r, width(k));
    }

    fn apply(self: *Calculator, kind: Kind, a: *const Managed, b: *const Managed) !Managed {
        var r = try Managed.init(self.arena);
        switch (kind) {
            .add => try r.add(a, b),
            .sub => try r.sub(a, b),
            .mul => try r.mul(a, b),
            .pow => try power(&r, a, b),
            .div, .mod => {
                if (b.eqlZero()) return error.DivisionByZero;
                var rem = try Managed.init(self.arena);
                // divFloor rounds the quotient toward negative infinity, as the spec does.
                try r.divFloor(&rem, a, b);
                if (kind == .mod) return rem;
            },
            .number, .neg, .open => unreachable,
        }
        return r;
    }

    /// Writes the line's value or error message.
    fn evaluate(self: *Calculator, out: *std.Io.Writer, items: []const Item) !void {
        var values: std.ArrayList(Managed) = .empty;
        for (items) |item| switch (item.kind) {
            .number => try values.append(self.arena, try self.parse(std.mem.trimStart(u8, item.digits, "0"))),
            .neg => values.items[values.items.len - 1].negate(),
            else => {
                const b = values.pop().?;
                const a = &values.items[values.items.len - 1];
                a.* = self.apply(item.kind, a, &b) catch |err| switch (err) {
                    error.DivisionByZero => return out.writeAll("division by zero"),
                    error.NegativeExponent => return out.writeAll("negative exponent"),
                    else => |e| return e,
                };
            },
        };
        var x = values.items[0];
        if (isNegative(&x)) try out.writeByte('-');
        x.abs();
        try self.emit(out, &x, 0);
    }
};

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(init.gpa, .unlimited);

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;

    var line_arena = std.heap.ArenaAllocator.init(init.gpa);
    var calculator: Calculator = .{ .gpa = init.gpa, .arena = line_arena.allocator() };
    var lines = std.mem.splitScalar(u8, input, '\n');
    while (lines.next()) |line| {
        // Every line ends with a line feed, so the piece after the last one is empty.
        if (lines.peek() == null) break;
        if (try postfix(calculator.arena, line)) |items| {
            try calculator.evaluate(out, items);
        } else {
            try out.writeAll("syntax error");
        }
        try out.writeByte('\n');
        _ = line_arena.reset(.retain_capacity);
    }
    try out.flush();
}
