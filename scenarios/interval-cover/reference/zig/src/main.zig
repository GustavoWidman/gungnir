const std = @import("std");

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    var tokens = std.mem.tokenizeAny(u8, input, " \t\r\n");

    const n = try std.fmt.parseInt(usize, tokens.next().?, 10);
    const q = try std.fmt.parseInt(usize, tokens.next().?, 10);
    const starts = try arena.alloc(i64, n);
    const ends = try arena.alloc(i64, n);
    for (starts, ends) |*s, *e| {
        s.* = try std.fmt.parseInt(i64, tokens.next().?, 10);
        e.* = try std.fmt.parseInt(i64, tokens.next().?, 10);
    }
    std.mem.sortUnstable(i64, starts, {}, std.sort.asc(i64));
    std.mem.sortUnstable(i64, ends, {}, std.sort.asc(i64));

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;
    for (0..q) |_| {
        const x = try std.fmt.parseInt(i64, tokens.next().?, 10);
        // Intervals containing x: those starting at or before x, minus those ending before x.
        const count = std.sort.partitionPoint(i64, starts, x, atMost) -
            std.sort.partitionPoint(i64, ends, x, below);
        try out.print("{d}\n", .{count});
    }
    try out.flush();
}

fn atMost(x: i64, start: i64) bool {
    return start <= x;
}

fn below(x: i64, end: i64) bool {
    return end < x;
}
