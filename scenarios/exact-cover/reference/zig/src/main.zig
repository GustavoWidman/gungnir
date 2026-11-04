const std = @import("std");

const Cell = struct { r: i32, c: i32 };

/// Up to 8 by 8 cells, sorted by row and then column, shifted to row and column 0.
const Shape = struct {
    len: usize = 0,
    cells: [64]Cell = undefined,

    fn slice(self: *const Shape) []const Cell {
        return self.cells[0..self.len];
    }
};

fn cellLess(_: void, a: Cell, b: Cell) bool {
    return a.r < b.r or (a.r == b.r and a.c < b.c);
}

fn order(a: *const Shape, b: *const Shape) std.math.Order {
    if (a.len != b.len) return std.math.order(a.len, b.len);
    for (a.slice(), b.slice()) |x, y| {
        if (x.r != y.r) return std.math.order(x.r, y.r);
        if (x.c != y.c) return std.math.order(x.c, y.c);
    }
    return .eq;
}

fn normalize(s: *Shape) void {
    var top: i32 = std.math.maxInt(i32);
    var left: i32 = std.math.maxInt(i32);
    for (s.slice()) |x| {
        top = @min(top, x.r);
        left = @min(left, x.c);
    }
    for (s.cells[0..s.len]) |*x| x.* = .{ .r = x.r - top, .c = x.c - left };
    std.mem.sortUnstable(Cell, s.cells[0..s.len], {}, cellLess);
}

/// Fills out with the distinct shapes the 4 rotations and their mirror images give, the
/// smallest first, and returns how many there are.
fn orientations(shape: Shape, out: *[8]Shape) usize {
    var s = shape;
    var found: usize = 0;
    for (0..2) |_| {
        for (0..4) |_| {
            for (s.cells[0..s.len]) |*x| {
                const old = x.*;
                x.* = .{ .r = old.c, .c = -old.r };
            }
            var t = s;
            normalize(&t);
            for (out[0..found]) |*seen| {
                if (order(seen, &t) == .eq) break;
            } else {
                out[found] = t;
                found += 1;
            }
        }
        for (s.cells[0..s.len]) |*x| x.c = -x.c;
    }
    for (1..found) |k| {
        if (order(&out[k], &out[0]) == .lt) std.mem.swap(Shape, &out[0], &out[k]);
    }
    return found;
}

/// Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
/// covered once all its copies are placed, so copies of a shape are never told apart.
const Dlx = struct {
    cells: usize,
    root: usize,
    copies: []u32,
    left: std.ArrayList(usize) = .empty,
    right: std.ArrayList(usize) = .empty,
    up: std.ArrayList(usize) = .empty,
    down: std.ArrayList(usize) = .empty,
    column: std.ArrayList(usize) = .empty,
    size: []u32,

    fn init(gpa: std.mem.Allocator, cells: usize, copies: []u32) !Dlx {
        const root = cells + copies.len;
        var x: Dlx = .{ .cells = cells, .root = root, .copies = copies, .size = try gpa.alloc(u32, root + 1) };
        @memset(x.size, 0);
        for (0..root + 1) |c| {
            try x.left.append(gpa, c);
            try x.right.append(gpa, c);
            try x.up.append(gpa, c);
            try x.down.append(gpa, c);
            try x.column.append(gpa, c);
        }
        // Only the cell columns and the root form the header ring.
        var previous = root;
        for (0..cells) |c| {
            x.right.items[previous] = c;
            x.left.items[c] = previous;
            previous = c;
        }
        x.right.items[previous] = root;
        x.left.items[root] = previous;
        return x;
    }

    fn addRow(x: *Dlx, gpa: std.mem.Allocator, columns: []const usize) !void {
        const first = x.left.items.len;
        for (columns, 0..) |col, k| {
            const node = first + k;
            try x.column.append(gpa, col);
            try x.up.append(gpa, x.up.items[col]);
            try x.down.append(gpa, col);
            x.down.items[x.up.items[col]] = node;
            x.up.items[col] = node;
            x.size[col] += 1;
            try x.left.append(gpa, if (k > 0) node - 1 else first + columns.len - 1);
            try x.right.append(gpa, if (k + 1 < columns.len) node + 1 else first);
        }
    }

    fn cover(x: *Dlx, c: usize) void {
        const l = x.left.items;
        const r = x.right.items;
        const u = x.up.items;
        const d = x.down.items;
        l[r[c]] = l[c];
        r[l[c]] = r[c];
        var i = d[c];
        while (i != c) : (i = d[i]) {
            var j = r[i];
            while (j != i) : (j = r[j]) {
                u[d[j]] = u[j];
                d[u[j]] = d[j];
                x.size[x.column.items[j]] -= 1;
            }
        }
    }

    fn uncover(x: *Dlx, c: usize) void {
        const l = x.left.items;
        const r = x.right.items;
        const u = x.up.items;
        const d = x.down.items;
        var i = u[c];
        while (i != c) : (i = u[i]) {
            var j = l[i];
            while (j != i) : (j = l[j]) {
                x.size[x.column.items[j]] += 1;
                u[d[j]] = j;
                d[u[j]] = j;
            }
        }
        l[r[c]] = c;
        r[l[c]] = c;
    }

    fn place(x: *Dlx, row: usize) void {
        var j = x.right.items[row];
        while (j != row) : (j = x.right.items[j]) {
            const col = x.column.items[j];
            if (col < x.cells) {
                x.cover(col);
            } else {
                x.copies[col - x.cells] -= 1;
                if (x.copies[col - x.cells] == 0) x.cover(col);
            }
        }
    }

    fn unplace(x: *Dlx, row: usize) void {
        var j = x.left.items[row];
        while (j != row) : (j = x.left.items[j]) {
            const col = x.column.items[j];
            if (col < x.cells) {
                x.uncover(col);
            } else {
                if (x.copies[col - x.cells] == 0) x.uncover(col);
                x.copies[col - x.cells] += 1;
            }
        }
    }

    /// The number of exact covers, choosing at each step the cell with the fewest options.
    fn count(x: *Dlx) u64 {
        const r = x.right.items;
        var best = r[x.root];
        if (best == x.root) return 1;
        var c = r[best];
        while (c != x.root and x.size[best] > 0) : (c = r[c]) {
            if (x.size[c] < x.size[best]) best = c;
        }
        if (x.size[best] == 0) return 0;
        x.cover(best);
        var total: u64 = 0;
        var row = x.down.items[best];
        while (row != best) : (row = x.down.items[row]) {
            x.place(row);
            total += x.count();
            x.unplace(row);
        }
        x.uncover(best);
        return total;
    }
};

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    var tokens = std.mem.tokenizeAny(u8, input, " \n");

    const height = try std.fmt.parseInt(usize, tokens.next().?, 10);
    const width = try std.fmt.parseInt(usize, tokens.next().?, 10);
    const index = try arena.alloc(?usize, height * width);
    var cells: usize = 0;
    for (0..height) |r| {
        const line = tokens.next().?;
        for (0..width) |c| {
            index[r * width + c] = null;
            if (line[c] == '.') {
                index[r * width + c] = cells;
                cells += 1;
            }
        }
    }

    const pieces = try std.fmt.parseInt(usize, tokens.next().?, 10);
    const shapes = try arena.alloc(Shape, pieces);
    const copies = try arena.alloc(u32, pieces);
    var kinds: usize = 0;
    var area: usize = 0;
    for (0..pieces) |_| {
        const rows = try std.fmt.parseInt(usize, tokens.next().?, 10);
        _ = tokens.next();
        var piece: Shape = .{};
        for (0..rows) |r| {
            for (tokens.next().?, 0..) |ch, c| {
                if (ch == '#') {
                    piece.cells[piece.len] = .{ .r = @intCast(r), .c = @intCast(c) };
                    piece.len += 1;
                }
            }
        }
        area += piece.len;
        var all: [8]Shape = undefined;
        _ = orientations(piece, &all);
        const kind = for (shapes[0..kinds], 0..) |*s, k| {
            if (order(s, &all[0]) == .eq) break k;
        } else blk: {
            shapes[kinds] = all[0];
            copies[kinds] = 0;
            kinds += 1;
            break :blk kinds - 1;
        };
        copies[kind] += 1;
    }

    var found: u64 = 0;
    if (area == cells) {
        var x = try Dlx.init(arena, cells, try arena.dupe(u32, copies[0..kinds]));
        var row: [65]usize = undefined;
        for (shapes[0..kinds], 0..) |shape, kind| {
            var all: [8]Shape = undefined;
            const n = orientations(shape, &all);
            for (all[0..n]) |*oriented| {
                for (0..height) |top| {
                    for (0..width) |left| {
                        const fits = for (oriented.slice(), 0..) |p, k| {
                            const r = top + @as(usize, @intCast(p.r));
                            const c = left + @as(usize, @intCast(p.c));
                            if (r >= height or c >= width) break false;
                            row[k] = index[r * width + c] orelse break false;
                        } else true;
                        if (fits) {
                            row[oriented.len] = cells + kind;
                            try x.addRow(arena, row[0 .. oriented.len + 1]);
                        }
                    }
                }
            }
        }
        found = x.count();
        if (found > 0) {
            for (copies[0..kinds]) |k| {
                for (2..k + 1) |f| found *= f;
            }
        }
    }

    var out_buf: [64]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    try stdout.interface.print("{d}\n", .{found});
    try stdout.interface.flush();
}
