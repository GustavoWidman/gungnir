const std = @import("std");

const Road = struct { to: u32, fuel: u32, time: u64 };

const State = struct {
    cost: u64,
    time: u64,
    place: u32,
    fuel: u32,

    fn order(_: void, a: State, b: State) std.math.Order {
        return switch (std.math.order(a.cost, b.cost)) {
            .eq => std.math.order(a.time, b.time),
            else => |o| o,
        };
    }
};

const Heap = std.PriorityQueue(State, void, State.order);

const Search = struct {
    capacity: u32,
    /// The price per unit at each place, or null where there is no station.
    price: []const ?u64,
    roads: []const []const Road,
    /// The most fuel each place was expanded with during the trip numbered trip[place].
    most: []u32,
    trip: []usize,
    heap: Heap,

    fn expanded(self: *const Search, id: usize, place: u32) ?u32 {
        return if (self.trip[place] == id) self.most[place] else null;
    }

    /// Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step. A
    /// state is dominated once its place was expanded with at least as much fuel, since that
    /// earlier state was no costlier and no slower.
    fn bestTrip(self: *Search, gpa: std.mem.Allocator, id: usize, start: u32, fuel: u32, target: u32) !?State {
        self.heap.clearRetainingCapacity();
        try self.heap.push(gpa, .{ .cost = 0, .time = 0, .place = start, .fuel = fuel });
        while (self.heap.pop()) |cur| {
            if (self.expanded(id, cur.place)) |most| if (most >= cur.fuel) continue;
            if (cur.place == target) return cur;
            self.trip[cur.place] = id;
            self.most[cur.place] = cur.fuel;
            if (self.price[cur.place]) |price| if (cur.fuel < self.capacity) {
                try self.heap.push(gpa, .{ .cost = cur.cost + price, .time = cur.time, .place = cur.place, .fuel = cur.fuel + 1 });
            };
            for (self.roads[cur.place]) |road| {
                if (road.fuel > cur.fuel) continue;
                const left = cur.fuel - road.fuel;
                if (self.expanded(id, road.to)) |most| if (most >= left) continue;
                try self.heap.push(gpa, .{ .cost = cur.cost, .time = cur.time + road.time, .place = road.to, .fuel = left });
            }
        }
        return null;
    }
};

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    var tokens = std.mem.tokenizeAny(u8, input, " \t\r\n");
    const next = struct {
        fn next(it: *std.mem.TokenIterator(u8, .any)) !u32 {
            return std.fmt.parseInt(u32, it.next() orelse return error.EndOfStream, 10);
        }
    }.next;

    const n = try next(&tokens);
    const capacity = try next(&tokens);
    const stations = try next(&tokens);
    const m = try next(&tokens);
    const q = try next(&tokens);

    const price = try arena.alloc(?u64, n + 1);
    @memset(price, null);
    for (0..stations) |_| {
        const place = try next(&tokens);
        price[place] = try next(&tokens);
    }

    // Roads that need more than the capacity can never be driven, so they are dropped.
    var lists = try arena.alloc(std.ArrayList(Road), n + 1);
    @memset(lists, .empty);
    for (0..m) |_| {
        const from = try next(&tokens);
        const to = try next(&tokens);
        const fuel = try next(&tokens);
        const time = try next(&tokens);
        if (fuel <= capacity) try lists[from].append(arena, .{ .to = to, .fuel = fuel, .time = time });
    }
    const roads = try arena.alloc([]const Road, n + 1);
    for (roads, lists) |*r, list| r.* = list.items;

    var search: Search = .{
        .capacity = capacity,
        .price = price,
        .roads = roads,
        .most = try arena.alloc(u32, n + 1),
        .trip = try arena.alloc(usize, n + 1),
        .heap = .empty,
    };
    @memset(search.trip, std.math.maxInt(usize));

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;
    for (0..q) |id| {
        const start = try next(&tokens);
        const fuel = try next(&tokens);
        const target = try next(&tokens);
        if (try search.bestTrip(arena, id, start, fuel, target)) |best| {
            try out.print("{d} {d}\n", .{ best.cost, best.time });
        } else {
            try out.writeAll("unreachable\n");
        }
    }
    try out.flush();
}
