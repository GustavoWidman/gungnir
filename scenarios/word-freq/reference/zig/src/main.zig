const std = @import("std");

const Entry = struct {
    word: []const u8,
    count: u32,
};

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    const newline = std.mem.indexOfScalar(u8, input, '\n') orelse input.len;
    const k = try std.fmt.parseInt(usize, input[0..newline], 10);
    const text = input[@min(newline + 1, input.len)..];
    _ = std.ascii.lowerString(text, text);

    var counts: std.StringHashMapUnmanaged(u32) = .empty;
    var i: usize = 0;
    while (i < text.len) {
        while (i < text.len and !std.ascii.isAlphabetic(text[i])) i += 1;
        const start = i;
        while (i < text.len and std.ascii.isAlphabetic(text[i])) i += 1;
        if (i == start) break;
        const slot = try counts.getOrPut(arena, text[start..i]);
        if (!slot.found_existing) slot.value_ptr.* = 0;
        slot.value_ptr.* += 1;
    }

    const ranked = try arena.alloc(Entry, counts.count());
    var it = counts.iterator();
    for (ranked) |*e| {
        const kv = it.next().?;
        e.* = .{ .word = kv.key_ptr.*, .count = kv.value_ptr.* };
    }
    std.mem.sortUnstable(Entry, ranked, {}, moreFrequentFirst);

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;
    for (ranked[0..@min(k, ranked.len)]) |e| {
        try out.print("{s} {d}\n", .{ e.word, e.count });
    }
    try out.flush();
}

fn moreFrequentFirst(_: void, a: Entry, b: Entry) bool {
    if (a.count != b.count) return a.count > b.count;
    return std.mem.order(u8, a.word, b.word) == .lt;
}
