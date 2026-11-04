const std = @import("std");
const Allocator = std.mem.Allocator;

const Kind = enum(u8) { scalar, array, object };

/// A value of the document. Nodes are stored in document order, so a node's index orders it,
/// and its descendants are exactly the nodes from its index + 1 up to `end`.
const Node = struct {
    kind: Kind,
    /// A scalar's printed text in `Document.text`, or a container's children in `Document.kids`.
    start: u32,
    len: u32,
    /// The member name printed as a string, in `Document.text`, when the parent is an object.
    name_start: u32,
    name_len: u32,
    end: u32,
};

const Document = struct {
    nodes: std.ArrayList(Node) = .empty,
    kids: std.ArrayList(u32) = .empty,
    text: std.ArrayList(u8) = .empty,

    fn kidsOf(doc: *const Document, node: Node) []const u32 {
        if (node.kind == .scalar) return &.{};
        return doc.kids.items[node.start..][0..node.len];
    }

    fn name(doc: *const Document, id: u32) []const u8 {
        const node = doc.nodes.items[id];
        return doc.text.items[node.name_start..][0..node.name_len];
    }
};

const Slice = struct { start: ?i64, end: ?i64, step: i64 };

/// A name is the member name printed as a string. Printing is one-to-one on code unit
/// sequences, so two names are equal exactly when their printed forms are.
const Selector = union(enum) { name: []const u8, wild, index: i64, slice: Slice };

const Segment = struct { descendant: bool, selector: Selector };

const Error = error{Invalid} || Allocator.Error;

fn putUnit(gpa: Allocator, out: *std.ArrayList(u8), unit: u32) Allocator.Error!void {
    const hex = "0123456789abcdef";
    const short: ?[]const u8 = switch (unit) {
        0x22 => "\\\"",
        0x5C => "\\\\",
        0x08 => "\\b",
        0x0C => "\\f",
        0x0A => "\\n",
        0x0D => "\\r",
        0x09 => "\\t",
        else => null,
    };
    if (short) |s| {
        try out.appendSlice(gpa, s);
    } else if (unit >= 0x20 and unit <= 0x7E) {
        try out.append(gpa, @intCast(unit));
    } else {
        try out.appendSlice(gpa, &.{ '\\', 'u', hex[unit >> 12 & 15], hex[unit >> 8 & 15], hex[unit >> 4 & 15], hex[unit & 15] });
    }
}

/// Parses the string literal at `src[pos]`, which must be `"`, and appends it to `out` as it
/// prints. Returns the position after the closing quote.
fn parseString(gpa: Allocator, src: []const u8, start: usize, out: *std.ArrayList(u8)) Error!usize {
    var pos = start + 1;
    try out.append(gpa, '"');
    while (true) {
        var run = pos;
        while (run < src.len and src[run] >= 0x20 and src[run] < 0x7F and src[run] != '"' and src[run] != '\\') run += 1;
        try out.appendSlice(gpa, src[pos..run]);
        pos = run;
        if (pos >= src.len) return error.Invalid;
        const c = src[pos];
        pos += 1;
        if (c == '"') break;
        if (c == '\\') {
            if (pos >= src.len) return error.Invalid;
            const unit: u32 = switch (src[pos]) {
                '"' => 0x22,
                '\\' => 0x5C,
                '/' => 0x2F,
                'b' => 0x08,
                'f' => 0x0C,
                'n' => 0x0A,
                'r' => 0x0D,
                't' => 0x09,
                'u' => blk: {
                    if (pos + 5 > src.len) return error.Invalid;
                    var unit: u32 = 0;
                    for (src[pos + 1 .. pos + 5]) |d| {
                        unit = unit << 4 | (std.fmt.charToDigit(d, 16) catch return error.Invalid);
                    }
                    pos += 4;
                    break :blk unit;
                },
                else => return error.Invalid,
            };
            pos += 1;
            try putUnit(gpa, out, unit);
        } else if (c == 0x7F) {
            try putUnit(gpa, out, c);
        } else if (c < 0x20) {
            return error.Invalid;
        } else {
            var low: u8 = 0x80;
            var high: u8 = 0xBF;
            const count: u5 = switch (c) {
                0xC2...0xDF => 1,
                0xE0 => blk: {
                    low = 0xA0;
                    break :blk 2;
                },
                0xE1...0xEC, 0xEE...0xEF => 2,
                0xED => blk: {
                    high = 0x9F;
                    break :blk 2;
                },
                0xF0 => blk: {
                    low = 0x90;
                    break :blk 3;
                },
                0xF1...0xF3 => 3,
                0xF4 => blk: {
                    high = 0x8F;
                    break :blk 3;
                },
                else => return error.Invalid,
            };
            if (pos + count > src.len or src[pos] < low or src[pos] > high) return error.Invalid;
            var point: u32 = c & (@as(u32, 0x3F) >> count);
            for (src[pos .. pos + count]) |b| {
                if (b < 0x80 or b > 0xBF) return error.Invalid;
                point = point << 6 | (b & 0x3F);
            }
            pos += count;
            if (point >= 0x10000) {
                try putUnit(gpa, out, 0xD800 + ((point - 0x10000) >> 10));
                try putUnit(gpa, out, 0xDC00 + ((point - 0x10000) & 0x3FF));
            } else {
                try putUnit(gpa, out, point);
            }
        }
    }
    try out.append(gpa, '"');
    return pos;
}

const Integer = struct { end: usize, fits: bool };

/// Parses `-?(0|[1-9][0-9]*)` at `src[start]`.
fn parseInteger(src: []const u8, start: usize) ?Integer {
    var pos = start;
    const negative = pos < src.len and src[pos] == '-';
    if (negative) pos += 1;
    const first = pos;
    if (pos < src.len and src[pos] == '0') {
        pos += 1;
    } else if (pos < src.len and src[pos] >= '1' and src[pos] <= '9') {
        while (pos < src.len and std.ascii.isDigit(src[pos])) pos += 1;
    } else {
        return null;
    }
    const digits = src[first..pos];
    const limit = if (negative) "9223372036854775808" else "9223372036854775807";
    const fits = digits.len < limit.len or (digits.len == limit.len and std.mem.order(u8, digits, limit) != .gt);
    return .{ .end = pos, .fits = fits };
}

const Parser = struct {
    gpa: Allocator,
    src: []const u8,
    pos: usize = 0,
    doc: Document = .{},
    pending: std.ArrayList(u32) = .empty,

    fn skipWhitespace(p: *Parser) void {
        while (p.pos < p.src.len) : (p.pos += 1) {
            switch (p.src[p.pos]) {
                ' ', '\t', '\n', '\r' => {},
                else => return,
            }
        }
    }

    fn eat(p: *Parser, c: u8) bool {
        p.skipWhitespace();
        if (p.pos < p.src.len and p.src[p.pos] == c) {
            p.pos += 1;
            return true;
        }
        return false;
    }

    fn string(p: *Parser, start: *u32, len: *u32) Error!void {
        const before = p.doc.text.items.len;
        p.pos = try parseString(p.gpa, p.src, p.pos, &p.doc.text);
        start.* = @intCast(before);
        len.* = @intCast(p.doc.text.items.len - before);
    }

    fn value(p: *Parser, name_start: u32, name_len: u32) Error!void {
        p.skipWhitespace();
        if (p.pos >= p.src.len) return error.Invalid;
        const id = p.doc.nodes.items.len;
        var node: Node = .{ .kind = .scalar, .start = 0, .len = 0, .name_start = name_start, .name_len = name_len, .end = 0 };
        try p.doc.nodes.append(p.gpa, node);
        const c = p.src[p.pos];
        if (c == '[' or c == '{') {
            p.pos += 1;
            const close: u8 = if (c == '[') ']' else '}';
            node.kind = if (c == '[') .array else .object;
            const mark = p.pending.items.len;
            if (!p.eat(close)) {
                while (true) {
                    var ns: u32 = 0;
                    var nl: u32 = 0;
                    if (node.kind == .object) {
                        p.skipWhitespace();
                        if (p.pos >= p.src.len or p.src[p.pos] != '"') return error.Invalid;
                        try p.string(&ns, &nl);
                        if (!p.eat(':')) return error.Invalid;
                    }
                    try p.pending.append(p.gpa, @intCast(p.doc.nodes.items.len));
                    try p.value(ns, nl);
                    if (p.eat(close)) break;
                    if (!p.eat(',')) return error.Invalid;
                }
            }
            node.start = @intCast(p.doc.kids.items.len);
            node.len = @intCast(p.pending.items.len - mark);
            try p.doc.kids.appendSlice(p.gpa, p.pending.items[mark..]);
            p.pending.shrinkRetainingCapacity(mark);
        } else if (c == '"') {
            try p.string(&node.start, &node.len);
        } else if (c == '-' or std.ascii.isDigit(c)) {
            const integer = parseInteger(p.src, p.pos) orelse return error.Invalid;
            if (!integer.fits) return error.Invalid;
            const digits = p.src[p.pos..integer.end];
            node.start = @intCast(p.doc.text.items.len);
            try p.doc.text.appendSlice(p.gpa, if (std.mem.eql(u8, digits, "-0")) "0" else digits);
            node.len = @intCast(p.doc.text.items.len - node.start);
            p.pos = integer.end;
        } else {
            var word: ?[]const u8 = null;
            for ([_][]const u8{ "true", "false", "null" }) |w| {
                if (std.mem.startsWith(u8, p.src[p.pos..], w)) word = w;
            }
            const w = word orelse return error.Invalid;
            node.start = @intCast(p.doc.text.items.len);
            node.len = @intCast(w.len);
            try p.doc.text.appendSlice(p.gpa, w);
            p.pos += w.len;
        }
        node.end = @intCast(p.doc.nodes.items.len);
        p.doc.nodes.items[id] = node;
    }
};

fn parseDocument(gpa: Allocator, src: []const u8) Error!Document {
    var p: Parser = .{ .gpa = gpa, .src = src };
    try p.doc.text.ensureUnusedCapacity(gpa, src.len);
    try p.value(0, 0);
    p.skipWhitespace();
    if (p.pos != src.len) return error.Invalid;
    return p.doc;
}

/// Parses an optional slice bound or index at `q[pos.*]`.
fn parseBound(q: []const u8, pos: *usize) error{Invalid}!?i64 {
    if (pos.* >= q.len or (q[pos.*] != '-' and !std.ascii.isDigit(q[pos.*]))) return null;
    const integer = parseInteger(q, pos.*) orelse return error.Invalid;
    if (!integer.fits) return error.Invalid;
    const value = std.fmt.parseInt(i64, q[pos.*..integer.end], 10) catch return error.Invalid;
    pos.* = integer.end;
    return value;
}

/// Parses the selector of the bracket that opens at `q[start]`; returns it and moves `end`
/// past the `]`.
fn parseBracket(gpa: Allocator, q: []const u8, start: usize, end: *usize) Error!Selector {
    var pos = start + 1;
    var selector: Selector = undefined;
    if (pos < q.len and q[pos] == '*') {
        selector = .wild;
        pos += 1;
    } else if (pos < q.len and q[pos] == '"') {
        var name: std.ArrayList(u8) = .empty;
        pos = try parseString(gpa, q, pos, &name);
        selector = .{ .name = name.items };
    } else {
        const first = try parseBound(q, &pos);
        if (pos < q.len and q[pos] == ':') {
            pos += 1;
            const last = try parseBound(q, &pos);
            var step: i64 = 1;
            if (pos < q.len and q[pos] == ':') {
                pos += 1;
                if (try parseBound(q, &pos)) |s| {
                    if (s < 1) return error.Invalid;
                    step = s;
                }
            }
            selector = .{ .slice = .{ .start = first, .end = last, .step = step } };
        } else {
            selector = .{ .index = first orelse return error.Invalid };
        }
    }
    if (pos >= q.len or q[pos] != ']') return error.Invalid;
    end.* = pos + 1;
    return selector;
}

fn isNameStart(c: u8) bool {
    return std.ascii.isAlphabetic(c) or c == '_';
}

fn parseQuery(gpa: Allocator, q: []const u8) Error![]Segment {
    if (q.len == 0 or q[0] != '$') return error.Invalid;
    var segments: std.ArrayList(Segment) = .empty;
    var pos: usize = 1;
    while (pos < q.len) {
        var descendant = false;
        var dotted = false;
        if (std.mem.startsWith(u8, q[pos..], "..")) {
            descendant = true;
            dotted = true;
            pos += 2;
        } else if (q[pos] == '.') {
            dotted = true;
            pos += 1;
        }
        var selector: Selector = undefined;
        if (pos < q.len and q[pos] == '[' and (descendant or !dotted)) {
            selector = try parseBracket(gpa, q, pos, &pos);
        } else if (dotted and pos < q.len and q[pos] == '*') {
            selector = .wild;
            pos += 1;
        } else if (dotted and pos < q.len and isNameStart(q[pos])) {
            const start = pos;
            while (pos < q.len and (isNameStart(q[pos]) or std.ascii.isDigit(q[pos]))) pos += 1;
            selector = .{ .name = try std.mem.concat(gpa, u8, &.{ "\"", q[start..pos], "\"" }) };
        } else {
            return error.Invalid;
        }
        try segments.append(gpa, .{ .descendant = descendant, .selector = selector });
    }
    return segments.items;
}

const Range = struct { start: usize = 0, end: usize = 0, step: usize = 1 };

/// Normalizes a slice to its first position, the bound it stays below, and its step. The step
/// is capped at the length + 1, which picks the same positions and cannot overflow.
fn sliceRange(s: Slice, len: usize) Range {
    const n: i64 = @intCast(len);
    const clamp = struct {
        fn f(bound: i64, length: i64) usize {
            const b = if (bound < 0) bound + length else bound;
            return @intCast(std.math.clamp(b, 0, length));
        }
    }.f;
    return .{
        .start = if (s.start) |b| clamp(b, n) else 0,
        .end = if (s.end) |b| clamp(b, n) else len,
        .step = @intCast(@min(s.step, n + 1)),
    };
}

fn indexPosition(index: i64, len: usize) ?usize {
    const n: i64 = @intCast(len);
    const i = if (index < 0) index + n else index;
    return if (i >= 0 and i < n) @intCast(i) else null;
}

fn children(doc: *const Document, gpa: Allocator, id: u32, selector: Selector, out: *std.ArrayList(u32)) Allocator.Error!void {
    const node = doc.nodes.items[id];
    const kids = doc.kidsOf(node);
    switch (selector) {
        .name => |name| if (node.kind == .object) {
            for (kids) |kid| {
                if (std.mem.eql(u8, doc.name(kid), name)) try out.append(gpa, kid);
            }
        },
        .wild => try out.appendSlice(gpa, kids),
        .index => |index| if (node.kind == .array) {
            if (indexPosition(index, kids.len)) |j| try out.append(gpa, kids[j]);
        },
        .slice => |s| if (node.kind == .array) {
            const r = sliceRange(s, kids.len);
            var j = r.start;
            while (j < r.end) : (j += r.step) try out.append(gpa, kids[j]);
        },
    }
}

/// Appends what `selector` picks from `id` and each of its descendants, in document order.
fn descend(doc: *const Document, gpa: Allocator, id: u32, selector: Selector, out: *std.ArrayList(u32)) Allocator.Error!void {
    const node = doc.nodes.items[id];
    if (node.kind == .scalar) return;
    const kids = doc.kidsOf(node);
    var r: Range = .{};
    if (node.kind == .array) {
        switch (selector) {
            .index => |index| if (indexPosition(index, kids.len)) |j| {
                r = .{ .start = j, .end = j + 1 };
            },
            .slice => |s| r = sliceRange(s, kids.len),
            else => {},
        }
    }
    for (kids, 0..) |kid, j| {
        const picked = switch (selector) {
            .name => |name| node.kind == .object and std.mem.eql(u8, doc.name(kid), name),
            .wild => true,
            else => j >= r.start and j < r.end and (j - r.start) % r.step == 0,
        };
        if (picked) try out.append(gpa, kid);
        try descend(doc, gpa, kid, selector, out);
    }
}

/// The nodes a query selects, each once, in document order. A set holds no node nested in
/// another until a descendant segment, and only a nested set can come out unsorted.
fn evaluate(doc: *const Document, gpa: Allocator, segments: []const Segment) Allocator.Error![]u32 {
    var nodes: std.ArrayList(u32) = .empty;
    try nodes.append(gpa, 0);
    var nested = false;
    for (segments) |segment| {
        var out: std.ArrayList(u32) = .empty;
        if (segment.descendant) {
            var covered: u32 = 0;
            for (nodes.items) |id| {
                if (id >= covered) {
                    try descend(doc, gpa, id, segment.selector, &out);
                    covered = doc.nodes.items[id].end;
                }
            }
            nested = true;
        } else {
            for (nodes.items) |id| try children(doc, gpa, id, segment.selector, &out);
            if (nested) std.mem.sortUnstable(u32, out.items, {}, std.sort.asc(u32));
        }
        nodes = out;
    }
    return nodes.items;
}

fn print(doc: *const Document, gpa: Allocator, id: u32, out: *std.ArrayList(u8)) Allocator.Error!void {
    const node = doc.nodes.items[id];
    if (node.kind == .scalar) {
        try out.appendSlice(gpa, doc.text.items[node.start..][0..node.len]);
        return;
    }
    try out.append(gpa, if (node.kind == .array) '[' else '{');
    for (doc.kidsOf(node), 0..) |kid, j| {
        if (j > 0) try out.append(gpa, ',');
        if (node.kind == .object) {
            try out.appendSlice(gpa, doc.name(kid));
            try out.append(gpa, ':');
        }
        try print(doc, gpa, kid, out);
    }
    try out.append(gpa, if (node.kind == .array) ']' else '}');
}

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const gpa = init.gpa;

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(gpa, .unlimited);
    var rest = input;
    var newline = std.mem.indexOfScalar(u8, rest, '\n') orelse rest.len;
    const count = try std.fmt.parseInt(usize, rest[0..newline], 10);
    rest = rest[newline + 1 ..];
    const queries = try gpa.alloc([]const u8, count);
    for (queries) |*query| {
        newline = std.mem.indexOfScalar(u8, rest, '\n') orelse rest.len;
        query.* = rest[0..newline];
        rest = rest[newline + 1 ..];
    }

    var out: std.ArrayList(u8) = .empty;
    if (parseDocument(gpa, rest)) |doc| {
        var scratch = std.heap.ArenaAllocator.init(gpa);
        for (queries) |query| {
            _ = scratch.reset(.retain_capacity);
            const temp = scratch.allocator();
            const segments = parseQuery(temp, query) catch |err| switch (err) {
                error.Invalid => {
                    try out.appendSlice(gpa, "invalid query\n");
                    continue;
                },
                else => return err,
            };
            const nodes = try evaluate(&doc, temp, segments);
            try out.print(gpa, "{d}\n", .{nodes.len});
            for (nodes) |id| {
                try print(&doc, gpa, id, &out);
                try out.append(gpa, '\n');
            }
        }
    } else |err| switch (err) {
        error.Invalid => try out.appendSlice(gpa, "invalid document\n"),
        else => return err,
    }

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    try stdout.interface.writeAll(out.items);
    try stdout.interface.flush();
}
