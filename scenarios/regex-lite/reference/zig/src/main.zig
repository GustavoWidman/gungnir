const std = @import("std");
const Allocator = std.mem.Allocator;

const escapable = "\\.*+?|()[]^$-";
/// A text line has at most 100000 characters, so one line adds at most about 300000 DFA states.
const state_limit = 10000;

const Kind = enum { char, epsilon, begin, end, match };

const Node = struct {
    kind: Kind,
    chars: u128,
    out: std.ArrayList(u32) = .empty,
};

const Fragment = struct { entry: u32, exit: u32 };

const ParseError = error{ Invalid, OutOfMemory };

/// Builds a Thompson NFA while parsing. Every fragment is an (entry, exit) pair whose exit is an
/// epsilon node that later gets the edges to whatever follows.
const Parser = struct {
    gpa: Allocator,
    pattern: []const u8,
    pos: usize = 0,
    nodes: std.ArrayList(Node) = .empty,

    fn node(p: *Parser, kind: Kind, chars: u128) !u32 {
        try p.nodes.append(p.gpa, .{ .kind = kind, .chars = chars });
        return @intCast(p.nodes.items.len - 1);
    }

    fn link(p: *Parser, from: u32, to: u32) !void {
        try p.nodes.items[from].out.append(p.gpa, to);
    }

    fn peek(p: *const Parser) ?u8 {
        return if (p.pos < p.pattern.len) p.pattern[p.pos] else null;
    }

    fn take(p: *Parser) ParseError!u8 {
        const c = p.peek() orelse return error.Invalid;
        p.pos += 1;
        return c;
    }

    fn escaped(p: *Parser) ParseError!u8 {
        const c = try p.take();
        if (std.mem.indexOfScalar(u8, escapable, c) == null) return error.Invalid;
        return c;
    }

    /// Parses the whole pattern and returns the start node. The accepting node comes last.
    fn parse(p: *Parser) ParseError!u32 {
        const f = try p.alternation();
        if (p.pos != p.pattern.len) return error.Invalid;
        const accept = try p.node(.match, 0);
        try p.link(f.exit, accept);
        return f.entry;
    }

    fn alternation(p: *Parser) ParseError!Fragment {
        const entry = try p.node(.epsilon, 0);
        const exit = try p.node(.epsilon, 0);
        while (true) {
            const f = try p.sequence();
            try p.link(entry, f.entry);
            try p.link(f.exit, exit);
            if (p.peek() != '|') return .{ .entry = entry, .exit = exit };
            p.pos += 1;
        }
    }

    fn sequence(p: *Parser) ParseError!Fragment {
        const entry = try p.node(.epsilon, 0);
        var last = entry;
        while (p.peek()) |c| {
            if (c == '|' or c == ')') break;
            const f = try p.item();
            try p.link(last, f.entry);
            last = f.exit;
        }
        return .{ .entry = entry, .exit = last };
    }

    fn item(p: *Parser) ParseError!Fragment {
        var f = try p.atom();
        switch (p.peek() orelse return f) {
            '*' => {
                p.pos += 1;
                const loop = try p.node(.epsilon, 0);
                try p.link(loop, f.entry);
                try p.link(f.exit, loop);
                f = .{ .entry = loop, .exit = loop };
            },
            '+' => {
                p.pos += 1;
                try p.link(f.exit, f.entry);
            },
            '?' => {
                p.pos += 1;
                const skip = try p.node(.epsilon, 0);
                try p.link(skip, f.entry);
                try p.link(skip, f.exit);
                f.entry = skip;
            },
            else => {},
        }
        return f;
    }

    fn atom(p: *Parser) ParseError!Fragment {
        const c = try p.take();
        const chars: u128 = switch (c) {
            '(' => {
                const f = try p.alternation();
                if (try p.take() != ')') return error.Invalid;
                return f;
            },
            '*', '+', '?' => return error.Invalid,
            '^' => return p.wrap(.begin, 0),
            '$' => return p.wrap(.end, 0),
            '.' => std.math.maxInt(u128),
            '[' => try p.bracket(),
            '\\' => @as(u128, 1) << @intCast(try p.escaped()),
            else => @as(u128, 1) << @intCast(c),
        };
        return p.wrap(.char, chars);
    }

    fn wrap(p: *Parser, kind: Kind, chars: u128) ParseError!Fragment {
        const n = try p.node(kind, chars);
        const exit = try p.node(.epsilon, 0);
        try p.link(n, exit);
        return .{ .entry = n, .exit = exit };
    }

    fn bracket(p: *Parser) ParseError!u128 {
        const negated = p.peek() == '^';
        if (negated) p.pos += 1;
        const Token = struct { c: u8, dash: bool };
        var body: std.ArrayList(Token) = .empty;
        defer body.deinit(p.gpa);
        while (true) {
            const c = try p.take();
            if (c == ']') break;
            const token: Token = if (c == '\\') .{ .c = try p.escaped(), .dash = false } else .{ .c = c, .dash = c == '-' };
            try body.append(p.gpa, token);
        }
        const tokens = body.items;
        if (tokens.len == 0) return error.Invalid;
        var chars: u128 = 0;
        var start: usize = 0;
        var stop = tokens.len;
        if (tokens[0].dash) {
            chars |= 1 << '-';
            start = 1;
        }
        if (stop > start and tokens[stop - 1].dash) {
            chars |= 1 << '-';
            stop -= 1;
        }
        var i = start;
        while (i < stop) {
            if (tokens[i].dash) return error.Invalid;
            if (i + 1 < stop and tokens[i + 1].dash) {
                if (i + 2 >= stop or tokens[i + 2].dash or tokens[i].c > tokens[i + 2].c) return error.Invalid;
                for (tokens[i].c..@as(usize, tokens[i + 2].c) + 1) |c| chars |= @as(u128, 1) << @intCast(c);
                i += 3;
            } else {
                chars |= @as(u128, 1) << @intCast(tokens[i].c);
                i += 1;
            }
        }
        return if (negated) ~chars else chars;
    }
};

fn setBit(bits: []u64, k: usize) void {
    bits[k / 64] |= @as(u64, 1) << @intCast(k % 64);
}

fn intersects(a: []const u64, b: []const u64) bool {
    for (a, b) |x, y| if (x & y != 0) return true;
    return false;
}

/// One interned DFA state: a bitset over the kernel and what is cached about it.
const State = struct {
    bits: []u64,
    /// Reverse transitions by character, -1 until computed.
    back: [128]i32 = @splat(-1),
    /// The state after every member consumes a character, or -1.
    next: i32 = -1,
    /// This state intersected with the start closure at an inner position, or -1.
    entry: i32 = -1,
    accepts: bool,
    consumes: bool,
    starts: bool,
};

/// Counts leftmost-longest matches in linear time per line.
///
/// The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
/// node that consumes a character. A reverse pass records at each position which kernel states
/// can still reach acceptance from there. Forward scans start only where a match starts and keep
/// only states that can still accept, so each scan stops right after its longest match. The
/// forward DFA reads the reverse pass's state at each position as its input symbol.
const Matcher = struct {
    gpa: Allocator,
    words: usize,
    /// Per kernel state, the kernel states after it consumes a character.
    follow: [][]u64,
    consumers: [128][]u32,
    /// The start closure, indexed by [at line start][at line end].
    start: [2][2][]u64,
    /// Per character, what can accept from the last position when it holds that character.
    last: [128][]u64,
    scratch: []u64,
    /// Holds the bitsets of the interned states, and empties when the cache resets.
    cache: std.heap.ArenaAllocator,
    ids: std.StringHashMapUnmanaged(i32) = .empty,
    states: std.ArrayList(State) = .empty,
    forward: std.AutoHashMapUnmanaged(u64, i32) = .empty,
    last_ids: [128]i32 = undefined,
    accept_only: i32 = 0,
    alive: std.ArrayList(i32) = .empty,

    fn init(gpa: Allocator, arena: Allocator, nodes: []const Node, start_node: u32) !Matcher {
        const kernel = try arena.alloc(usize, nodes.len);
        var size: usize = 1;
        for (nodes, kernel) |n, *k| {
            k.* = std.math.maxInt(usize);
            if (n.kind == .char) {
                k.* = size;
                size += 1;
            }
        }
        kernel[nodes.len - 1] = 0;
        const words = (size + 63) / 64;

        var closure: Closure = .{
            .nodes = nodes,
            .kernel = kernel,
            .words = words,
            .arena = arena,
            .mark = try arena.alloc(u32, nodes.len),
            .stack = try arena.alloc(u32, nodes.len),
        };
        @memset(closure.mark, 0);

        const follow = try arena.alloc([]u64, size);
        const follow_end = try arena.alloc([]u64, size);
        follow[0] = try closure.empty();
        var consumers: [128]std.ArrayList(u32) = @splat(.empty);
        for (nodes, kernel) |n, k| {
            if (k == 0 or k == std.math.maxInt(usize)) continue;
            follow[k] = try closure.of(n.out.items, false, false);
            follow_end[k] = try closure.of(n.out.items, false, true);
            for (32..127) |c| {
                if (n.chars >> @intCast(c) & 1 == 1) try consumers[c].append(arena, @intCast(k));
            }
        }
        var m: Matcher = .{
            .gpa = gpa,
            .words = words,
            .follow = follow,
            .consumers = undefined,
            .start = undefined,
            .last = undefined,
            .scratch = try arena.alloc(u64, words),
            .cache = .init(gpa),
        };
        for (&m.consumers, consumers) |*list, built| list.* = built.items;
        for (0..2) |b| {
            for (0..2) |e| m.start[b][e] = try closure.of(&.{start_node}, b == 1, e == 1);
        }
        for (&m.last, m.consumers) |*last, list| {
            last.* = try closure.empty();
            setBit(last.*, 0);
            for (list) |k| {
                if (follow_end[k][0] & 1 == 1) setBit(last.*, k);
            }
        }
        try m.reset();
        return m;
    }

    fn reset(m: *Matcher) !void {
        m.ids.clearRetainingCapacity();
        m.states.clearRetainingCapacity();
        m.forward.clearRetainingCapacity();
        _ = m.cache.reset(.retain_capacity);
        @memset(m.scratch, 0);
        setBit(m.scratch, 0);
        m.accept_only = try m.intern();
        for (&m.last_ids, m.last) |*id, bits| {
            @memcpy(m.scratch, bits);
            id.* = try m.intern();
        }
    }

    /// The state holding the bitset in `scratch`, added when it is new.
    fn intern(m: *Matcher) !i32 {
        if (m.ids.get(std.mem.sliceAsBytes(m.scratch))) |id| return id;
        const bits = try m.cache.allocator().dupe(u64, m.scratch);
        const id: i32 = @intCast(m.states.items.len);
        var consumes = bits[0] > 1;
        for (bits[1..]) |word| consumes = consumes or word != 0;
        try m.states.append(m.gpa, .{
            .bits = bits,
            .accepts = bits[0] & 1 == 1,
            .consumes = consumes,
            .starts = intersects(m.start[0][0], bits),
        });
        try m.ids.put(m.gpa, std.mem.sliceAsBytes(bits), id);
        return id;
    }

    /// The state holding the members of both `a` and state `b`.
    fn meet(m: *Matcher, a: []const u64, b: i32) !i32 {
        for (m.scratch, a, m.states.items[@intCast(b)].bits) |*s, x, y| s.* = x & y;
        return m.intern();
    }

    /// The kernel states that can accept from a position holding `c`, given those that can
    /// accept from the next position.
    fn stepBack(m: *Matcher, state: i32, c: u8) !i32 {
        @memset(m.scratch, 0);
        setBit(m.scratch, 0);
        const ahead = m.states.items[@intCast(state)].bits;
        for (m.consumers[c]) |k| {
            if (intersects(m.follow[k], ahead)) setBit(m.scratch, k);
        }
        const next = try m.intern();
        m.states.items[@intCast(state)].back[c] = next;
        return next;
    }

    /// The state after every member of `state` consumes a character. The character is implied:
    /// every state in a pruned set accepts the character at its position.
    fn consume(m: *Matcher, state: i32) !i32 {
        const cached = m.states.items[@intCast(state)].next;
        if (cached >= 0) return cached;
        @memset(m.scratch, 0);
        for (m.states.items[@intCast(state)].bits, 0..) |word, w| {
            var rest = if (w == 0) word & ~@as(u64, 1) else word;
            while (rest != 0) : (rest &= rest - 1) {
                for (m.scratch, m.follow[w * 64 + @ctz(rest)]) |*s, f| s.* |= f;
            }
        }
        const next = try m.intern();
        m.states.items[@intCast(state)].next = next;
        return next;
    }

    /// The pruned forward state after `state`, at a position whose reverse state is `alive`.
    fn step(m: *Matcher, state: i32, alive: i32) !i32 {
        const key = @as(u64, @intCast(state)) << 32 | @as(u64, @intCast(alive));
        if (m.forward.get(key)) |next| return next;
        const consumed = try m.consume(state);
        const next = try m.meet(m.states.items[@intCast(consumed)].bits, alive);
        try m.forward.put(m.gpa, key, next);
        return next;
    }

    /// The forward state at a match candidate `s` of a line of `n` characters.
    fn entryAt(m: *Matcher, s: usize, n: usize, alive: i32) !i32 {
        if (s == 0 or s == n) return m.meet(m.start[@intFromBool(s == 0)][@intFromBool(s == n)], alive);
        const cached = m.states.items[@intCast(alive)].entry;
        if (cached >= 0) return cached;
        const entry = try m.meet(m.start[0][0], alive);
        m.states.items[@intCast(alive)].entry = entry;
        return entry;
    }

    fn at(m: *const Matcher, id: i32) *const State {
        return &m.states.items[@intCast(id)];
    }

    fn count(m: *Matcher, line: []const u8) !u32 {
        if (m.states.items.len + m.forward.count() > state_limit) try m.reset();
        const n = line.len;
        try m.alive.resize(m.gpa, n + 1);
        const alive = m.alive.items;
        alive[n] = m.accept_only;
        if (n > 0) {
            var id = m.last_ids[line[n - 1]];
            alive[n - 1] = id;
            var i = n - 1;
            while (i > 0) {
                i -= 1;
                const c = line[i];
                var next = m.at(id).back[c];
                if (next < 0) next = try m.stepBack(id, c);
                alive[i] = next;
                id = next;
            }
        }

        var matches: u32 = 0;
        var p: usize = 0;
        while (p <= n) {
            var s = p;
            while (0 < s and s < n and !m.at(alive[s]).starts) s += 1;
            var current = try m.entryAt(s, n, alive[s]);
            if (!m.at(current).accepts and !m.at(current).consumes) {
                if (s >= n) break;
                p = s + 1;
                continue;
            }
            var end: ?usize = if (m.at(current).accepts) s else null;
            var i = s;
            while (m.at(current).consumes) {
                i += 1;
                current = if (i == n) m.accept_only else try m.step(current, alive[i]);
                if (m.at(current).accepts) end = i;
            }
            matches += 1;
            p = if (end != null and end.? > s) end.? else s + 1;
        }
        return matches;
    }
};

/// The epsilon closures of the NFA: the kernel numbering and a reusable depth-first search.
const Closure = struct {
    nodes: []const Node,
    kernel: []const usize,
    words: usize,
    arena: Allocator,
    mark: []u32,
    stack: []u32,
    stamp: u32 = 0,

    fn empty(c: *Closure) ![]u64 {
        const bits = try c.arena.alloc(u64, c.words);
        @memset(bits, 0);
        return bits;
    }

    /// The kernel states reachable from `roots` without consuming a character, at a position
    /// that is or is not the line's start and end.
    fn of(c: *Closure, roots: []const u32, at_begin: bool, at_end: bool) ![]u64 {
        const bits = try c.empty();
        c.stamp += 1;
        var top: usize = 0;
        for (roots) |root| c.visit(root, &top);
        while (top > 0) {
            top -= 1;
            const v = c.stack[top];
            switch (c.nodes[v].kind) {
                .char, .match => {
                    setBit(bits, c.kernel[v]);
                    continue;
                },
                .begin => if (!at_begin) continue,
                .end => if (!at_end) continue,
                .epsilon => {},
            }
            for (c.nodes[v].out.items) |next| c.visit(next, &top);
        }
        return bits;
    }

    fn visit(c: *Closure, v: u32, top: *usize) void {
        if (c.mark[v] == c.stamp) return;
        c.mark[v] = c.stamp;
        c.stack[top.*] = v;
        top.* += 1;
    }
};

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    const newline = std.mem.indexOfScalar(u8, input, '\n') orelse input.len;

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;

    var parser: Parser = .{ .gpa = arena, .pattern = input[0..newline] };
    if (parser.parse()) |start| {
        var matcher = try Matcher.init(init.gpa, arena, parser.nodes.items, start);
        var lines = std.mem.splitScalar(u8, input[@min(newline + 1, input.len)..], '\n');
        while (lines.next()) |line| {
            if (lines.peek() == null) break;
            try out.print("{d}\n", .{try matcher.count(line)});
        }
    } else |err| switch (err) {
        error.Invalid => try out.writeAll("invalid\n"),
        error.OutOfMemory => return err,
    }
    try out.flush();
}
