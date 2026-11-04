const std = @import("std");

const none = std.math.maxInt(u32);

const Hunk = struct { x: usize, a: usize, y: usize, b: usize };

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;

    var pos = std.mem.indexOfScalar(u8, input, '\n').?;
    const pairs = try std.fmt.parseInt(usize, input[0..pos], 10);
    pos += 1;
    var ids: std.StringHashMapUnmanaged(u32) = .empty;
    var a: std.ArrayList(u32) = .empty;
    var b: std.ArrayList(u32) = .empty;
    var rounds: std.ArrayList(u32) = .empty;
    var hunks: std.ArrayList(Hunk) = .empty;
    for (0..pairs) |_| {
        const end = std.mem.indexOfScalarPos(u8, input, pos, '\n').?;
        const space = std.mem.indexOfScalarPos(u8, input, pos, ' ').?;
        const n = try std.fmt.parseInt(usize, input[pos..space], 10);
        const m = try std.fmt.parseInt(usize, input[space + 1 .. end], 10);
        const first = input[end + 1 ..][0..n];
        const second = input[end + 1 + n ..][0..m];
        pos = end + 1 + n + m;

        ids.clearRetainingCapacity();
        try intern(arena, first, &ids, &a);
        try intern(arena, second, &ids, &b);
        try diff(arena, a.items, b.items, &rounds, &hunks);
        var deleted: usize = 0;
        var inserted: usize = 0;
        for (hunks.items) |h| {
            deleted += h.a;
            inserted += h.b;
        }
        try out.print("{d} {d} {d}\n", .{ deleted, inserted, hunks.items.len });
        for (hunks.items) |h| {
            try out.print("{d} {d} {d} {d}\n", .{ h.x, h.a, h.y, h.b });
        }
    }
    try out.flush();
}

/// Numbers the lines of `doc` into `lines`, giving equal lines equal numbers. A line keeps its
/// line feed, so a last line without one differs from the same text with one.
fn intern(gpa: std.mem.Allocator, doc: []const u8, ids: *std.StringHashMapUnmanaged(u32), lines: *std.ArrayList(u32)) !void {
    lines.clearRetainingCapacity();
    var rest = doc;
    while (rest.len > 0) {
        const length = if (std.mem.indexOfScalar(u8, rest, '\n')) |i| i + 1 else rest.len;
        const slot = try ids.getOrPut(gpa, rest[0..length]);
        if (!slot.found_existing) slot.value_ptr.* = ids.count() - 1;
        try lines.append(gpa, slot.value_ptr.*);
        rest = rest[length..];
    }
}

/// Where round `e` starts in the flat list of rounds: rounds 0 to e - 1 hold 1 to e entries.
fn offset(e: usize) usize {
    return e * (e + 1) / 2;
}

/// Myers' search run from the end of both documents. Entry i of round e is the smallest x on
/// diagonal k = a.len - b.len - e + 2i such that (x, x - k) is at most e edits from the end, or
/// `none` when no point of that diagonal is. Returns the first round that reaches (0, 0), which
/// is the shortest script's cost.
fn searchBack(gpa: std.mem.Allocator, a: []const u32, b: []const u32, rounds: *std.ArrayList(u32)) !usize {
    const n: i64 = @intCast(a.len);
    const m: i64 = @intCast(b.len);
    const kend = n - m;
    var x = a.len;
    var y = b.len;
    while (x > 0 and y > 0 and a[x - 1] == b[y - 1]) {
        x -= 1;
        y -= 1;
    }
    rounds.clearRetainingCapacity();
    try rounds.append(gpa, @intCast(x));
    var e: i64 = 0;
    while (true) : (e += 1) {
        const at = e - kend;
        if (@mod(at, 2) == 0 and 0 <= at and at <= 2 * e and
            rounds.items[offset(@intCast(e)) + @as(usize, @intCast(@divExact(at, 2)))] == 0)
            return @intCast(e);
        const next = e + 1;
        const prev = offset(@intCast(e));
        const cur = offset(@intCast(next));
        try rounds.appendNTimes(gpa, none, @intCast(next + 1));
        // Only the diagonals -m <= k <= n cross the grid.
        const first = @max(0, @divFloor(next - n + 1, 2));
        const last = @min(next, @divFloor(m + next, 2));
        var i = first;
        while (i <= last) : (i += 1) {
            const k = kend - next + 2 * i;
            const lo = @max(k, 0);
            const at_i: usize = @intCast(i);
            var best: i64 = std.math.maxInt(i64);
            if (i < next and rounds.items[prev + at_i] != none) {
                // Back from diagonal k + 1 over a deletion.
                best = @max(@as(i64, rounds.items[prev + at_i]) - 1, lo);
            }
            if (i > 0 and rounds.items[prev + at_i - 1] != none) {
                // Back from diagonal k - 1 over an insertion.
                best = @min(best, @max(@as(i64, rounds.items[prev + at_i - 1]), lo));
            }
            if (best != std.math.maxInt(i64)) {
                var sx: usize = @intCast(best);
                var sy: usize = @intCast(best - k);
                while (sx > 0 and sy > 0 and a[sx - 1] == b[sy - 1]) {
                    sx -= 1;
                    sy -= 1;
                }
                rounds.items[cur + at_i] = @intCast(sx);
            }
        }
    }
}

/// Writes the hunks of the first shortest edit script in keep, delete, insert order to `hunks`.
fn diff(gpa: std.mem.Allocator, a: []const u32, b: []const u32, rounds: *std.ArrayList(u32), hunks: *std.ArrayList(Hunk)) !void {
    const n = a.len;
    const m = b.len;
    const kend = @as(i64, @intCast(n)) - @as(i64, @intCast(m));
    var e = try searchBack(gpa, a, b, rounds);
    hunks.clearRetainingCapacity();
    var x: usize = 0;
    var y: usize = 0;
    while (true) {
        while (x < n and y < m and a[x] == b[y]) {
            x += 1;
            y += 1;
        }
        if (x == n and y == m) return;
        const x0 = x;
        const y0 = y;
        while (true) {
            // Delete when (x + 1, y) is still within e - 1 edits of the end.
            const diagonal = @as(i64, @intCast(x)) - @as(i64, @intCast(y)) + @as(i64, @intCast(e)) - kend;
            const i: usize = @intCast(@divExact(diagonal, 2));
            if (x < n and i < e and rounds.items[offset(e - 1) + i] <= x + 1) x += 1 else y += 1;
            e -= 1;
            if ((x < n and y < m and a[x] == b[y]) or (x == n and y == m)) break;
        }
        try hunks.append(gpa, .{ .x = x0, .a = x - x0, .y = y0, .b = y - y0 });
    }
}
