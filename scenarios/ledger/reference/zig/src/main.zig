const std = @import("std");
const Allocator = std.mem.Allocator;
const Writer = std.Io.Writer;

/// A failed check, named after the error line it prints.
const Rejection = error{
    AccountExists,
    NoAccount,
    AccountClosed,
    InsufficientFunds,
    SameAccount,
    NoHold,
    HoldInactive,
    ExceedsHold,
    NoTransaction,
    NotReversible,
    AlreadyReversed,
    LimitTooLow,
    HoldsActive,
    NonzeroBalance,
    BadRange,
};

fn message(rejection: Rejection) []const u8 {
    return switch (rejection) {
        error.AccountExists => "ERR ACCOUNT_EXISTS",
        error.NoAccount => "ERR NO_ACCOUNT",
        error.AccountClosed => "ERR ACCOUNT_CLOSED",
        error.InsufficientFunds => "ERR INSUFFICIENT_FUNDS",
        error.SameAccount => "ERR SAME_ACCOUNT",
        error.NoHold => "ERR NO_HOLD",
        error.HoldInactive => "ERR HOLD_INACTIVE",
        error.ExceedsHold => "ERR EXCEEDS_HOLD",
        error.NoTransaction => "ERR NO_TRANSACTION",
        error.NotReversible => "ERR NOT_REVERSIBLE",
        error.AlreadyReversed => "ERR ALREADY_REVERSED",
        error.LimitTooLow => "ERR LIMIT_TOO_LOW",
        error.HoldsActive => "ERR HOLDS_ACTIVE",
        error.NonzeroBalance => "ERR NONZERO_BALANCE",
        error.BadRange => "ERR BAD_RANGE",
    };
}

const Command = enum { open, deposit, withdraw, transfer, hold, release, capture, reverse, limit, close };
const Query = enum { balance, balance_at, statement };

const commands: std.StaticStringMap(Command) = .initComptime(.{
    .{ "OPEN", .open },
    .{ "DEPOSIT", .deposit },
    .{ "WITHDRAW", .withdraw },
    .{ "TRANSFER", .transfer },
    .{ "HOLD", .hold },
    .{ "RELEASE", .release },
    .{ "CAPTURE", .capture },
    .{ "REVERSE", .reverse },
    .{ "LIMIT", .limit },
    .{ "CLOSE", .close },
});

const queries: std.StaticStringMap(Query) = .initComptime(.{
    .{ "BALANCE", .balance },
    .{ "BALANCEAT", .balance_at },
    .{ "STATEMENT", .statement },
});

const Account = struct {
    closed: bool = false,
    ledger: i64 = 0,
    held: i64 = 0,
    limit: i64 = 0,
    // Entries arrive in timestamp order. Before entry i, at times[i], the ledger balance was
    // balances[i] and the credits so far summed to credits[i]; the last element is the total.
    times: std.ArrayList(i64) = .empty,
    balances: std.ArrayList(i64) = .empty,
    credits: std.ArrayList(i64) = .empty,

    fn create(allocator: Allocator) !*Account {
        const account = try allocator.create(Account);
        account.* = .{};
        try account.balances.append(allocator, 0);
        try account.credits.append(allocator, 0);
        return account;
    }

    fn post(self: *Account, allocator: Allocator, t: i64, amount: i64) !void {
        self.ledger += amount;
        try self.times.append(allocator, t);
        try self.balances.append(allocator, self.ledger);
        try self.credits.append(allocator, self.credits.getLast() + @max(amount, 0));
    }

    fn available(self: *const Account) i64 {
        return self.ledger - self.held;
    }

    /// The overdraft rule for taking `amount` out of the available balance.
    fn checkSpend(self: *const Account, amount: i64) Rejection!void {
        if (self.available() - amount < -self.limit) return error.InsufficientFunds;
    }

    /// The number of entries posted before `time`.
    fn countBefore(self: *const Account, time: i64) usize {
        return std.sort.partitionPoint(i64, self.times.items, time, below);
    }

    /// The number of entries posted at or before `time`.
    fn countAtMost(self: *const Account, time: i64) usize {
        return std.sort.partitionPoint(i64, self.times.items, time, atMost);
    }
};

fn below(x: i64, t: i64) bool {
    return t < x;
}

fn atMost(x: i64, t: i64) bool {
    return t <= x;
}

const Hold = struct {
    account: *Account,
    amount: i64,
    active: bool = true,
};

const Posting = struct {
    account: *Account,
    amount: i64,
};

/// An accepted deposit, withdrawal, transfer or capture: the entries it posted.
const Movement = struct {
    postings: []const Posting,
    reversed: bool = false,
};

/// What a recorded request did, as the commands that look it up see it.
const Effect = union(enum) {
    rejected: Rejection,
    hold: Hold,
    movement: Movement,
    /// An accepted command that is neither a hold nor a movement.
    other,

    fn reply(self: Effect) []const u8 {
        return switch (self) {
            .rejected => |rejection| message(rejection),
            else => "OK",
        };
    }
};

const Request = struct {
    text: []const u8,
    effect: Effect,
};

const Ledger = struct {
    allocator: Allocator,
    clock: i64 = 0,
    accounts: std.StringHashMapUnmanaged(*Account) = .empty,
    // Every recorded request id: its line after the timestamp, and what it did.
    requests: std.StringHashMapUnmanaged(Request) = .empty,

    fn process(self: *Ledger, line: []const u8, out: *Writer) !void {
        var fields: [6][]const u8 = undefined;
        var len: usize = 0;
        var it = std.mem.splitScalar(u8, line, ' ');
        while (it.next()) |field| : (len += 1) fields[len] = field;
        const stamp = fields[0];
        const t = try number(stamp);
        if (t < self.clock) return out.writeAll("ERR OUT_OF_ORDER\n");
        self.clock = t;
        const args = fields[2..len];
        if (queries.get(fields[1])) |query| return self.answer(query, args, out);
        const rid = args[0];
        const text = line[stamp.len + 1 ..];
        if (self.requests.get(rid)) |recorded| {
            const reply = if (std.mem.eql(u8, recorded.text, text)) recorded.effect.reply() else "ERR ID_CONFLICT";
            return out.print("{s}\n", .{reply});
        }
        const effect = try self.execute(commands.get(fields[1]).?, t, args[1..]);
        try self.requests.put(self.allocator, rid, .{ .text = text, .effect = effect });
        try out.print("{s}\n", .{effect.reply()});
    }

    fn execute(self: *Ledger, command: Command, t: i64, args: []const []const u8) !Effect {
        const outcome = switch (command) {
            .open => self.open(args[0]),
            .deposit => self.deposit(t, args[0], try number(args[1])),
            .withdraw => self.withdraw(t, args[0], try number(args[1])),
            .transfer => self.transfer(t, args[0], args[1], try number(args[2])),
            .hold => self.placeHold(args[0], try number(args[1])),
            .release => self.release(args[0]),
            .capture => self.capture(t, args[0], try number(args[1])),
            .reverse => self.reverse(t, args[0]),
            .limit => self.setLimit(args[0], try number(args[1])),
            .close => self.close(args[0]),
        };
        return outcome catch |err| switch (err) {
            error.OutOfMemory => |e| return e,
            else => |rejection| .{ .rejected = rejection },
        };
    }

    fn answer(self: *Ledger, query: Query, args: []const []const u8, out: *Writer) !void {
        const outcome = switch (query) {
            .balance => self.balance(args[0], out),
            .balance_at => self.balanceAt(args[0], try number(args[1]), out),
            .statement => self.statement(args[0], try number(args[1]), try number(args[2]), out),
        };
        outcome catch |err| switch (err) {
            error.WriteFailed => |e| return e,
            else => |rejection| try out.print("{s}\n", .{message(rejection)}),
        };
    }

    fn findAccount(self: *Ledger, name: []const u8) Rejection!*Account {
        return self.accounts.get(name) orelse error.NoAccount;
    }

    fn findOpenAccount(self: *Ledger, name: []const u8) Rejection!*Account {
        const account = try self.findAccount(name);
        if (account.closed) return error.AccountClosed;
        return account;
    }

    fn findActiveHold(self: *Ledger, id: []const u8) Rejection!*Hold {
        const recorded = self.requests.getPtr(id) orelse return error.NoHold;
        const hold = switch (recorded.effect) {
            .hold => |*h| h,
            else => return error.NoHold,
        };
        if (!hold.active) return error.HoldInactive;
        return hold;
    }

    fn move(self: *Ledger, t: i64, postings: []const Posting) !Effect {
        for (postings) |posting| try posting.account.post(self.allocator, t, posting.amount);
        return .{ .movement = .{ .postings = try self.allocator.dupe(Posting, postings) } };
    }

    fn open(self: *Ledger, name: []const u8) !Effect {
        const slot = try self.accounts.getOrPut(self.allocator, name);
        if (slot.found_existing) return error.AccountExists;
        slot.value_ptr.* = try Account.create(self.allocator);
        return .other;
    }

    fn deposit(self: *Ledger, t: i64, name: []const u8, amount: i64) !Effect {
        const account = try self.findOpenAccount(name);
        return self.move(t, &.{.{ .account = account, .amount = amount }});
    }

    fn withdraw(self: *Ledger, t: i64, name: []const u8, amount: i64) !Effect {
        const account = try self.findOpenAccount(name);
        try account.checkSpend(amount);
        return self.move(t, &.{.{ .account = account, .amount = -amount }});
    }

    fn transfer(self: *Ledger, t: i64, from: []const u8, to: []const u8, amount: i64) !Effect {
        if (std.mem.eql(u8, from, to)) return error.SameAccount;
        const payer = try self.findAccount(from);
        const payee = try self.findAccount(to);
        if (payer.closed or payee.closed) return error.AccountClosed;
        try payer.checkSpend(amount);
        return self.move(t, &.{
            .{ .account = payer, .amount = -amount },
            .{ .account = payee, .amount = amount },
        });
    }

    fn placeHold(self: *Ledger, name: []const u8, amount: i64) Rejection!Effect {
        const account = try self.findOpenAccount(name);
        try account.checkSpend(amount);
        account.held += amount;
        return .{ .hold = .{ .account = account, .amount = amount } };
    }

    fn release(self: *Ledger, hold_id: []const u8) Rejection!Effect {
        const hold = try self.findActiveHold(hold_id);
        hold.active = false;
        hold.account.held -= hold.amount;
        return .other;
    }

    fn capture(self: *Ledger, t: i64, hold_id: []const u8, amount: i64) !Effect {
        const hold = try self.findActiveHold(hold_id);
        if (amount > hold.amount) return error.ExceedsHold;
        hold.active = false;
        hold.account.held -= hold.amount;
        return self.move(t, &.{.{ .account = hold.account, .amount = -amount }});
    }

    fn reverse(self: *Ledger, t: i64, target: []const u8) !Effect {
        const recorded = self.requests.getPtr(target) orelse return error.NoTransaction;
        const movement = switch (recorded.effect) {
            .rejected => return error.NoTransaction,
            .movement => |*m| m,
            .hold, .other => return error.NotReversible,
        };
        if (movement.reversed) return error.AlreadyReversed;
        for (movement.postings) |posting| {
            if (posting.account.closed) return error.AccountClosed;
        }
        for (movement.postings) |posting| {
            if (posting.amount > 0) try posting.account.checkSpend(posting.amount);
        }
        movement.reversed = true;
        for (movement.postings) |posting| try posting.account.post(self.allocator, t, -posting.amount);
        return .other;
    }

    fn setLimit(self: *Ledger, name: []const u8, limit: i64) Rejection!Effect {
        const account = try self.findOpenAccount(name);
        if (account.available() < -limit) return error.LimitTooLow;
        account.limit = limit;
        return .other;
    }

    fn close(self: *Ledger, name: []const u8) Rejection!Effect {
        const account = try self.findOpenAccount(name);
        if (account.held != 0) return error.HoldsActive;
        if (account.ledger != 0) return error.NonzeroBalance;
        account.closed = true;
        return .other;
    }

    fn balance(self: *Ledger, name: []const u8, out: *Writer) !void {
        const account = try self.findAccount(name);
        try out.print("{d} {d}\n", .{ account.ledger, account.available() });
    }

    fn balanceAt(self: *Ledger, name: []const u8, time: i64, out: *Writer) !void {
        const account = try self.findAccount(name);
        try out.print("{d}\n", .{account.balances.items[account.countAtMost(time)]});
    }

    fn statement(self: *Ledger, name: []const u8, start: i64, end: i64, out: *Writer) !void {
        if (start > end) return error.BadRange;
        const account = try self.findAccount(name);
        const lo = account.countBefore(start);
        const hi = account.countAtMost(end);
        const opening = account.balances.items[lo];
        const closing = account.balances.items[hi];
        const credits = account.credits.items[hi] - account.credits.items[lo];
        const debits = credits - (closing - opening);
        try out.print("{d} {d} {d} {d} {d}\n", .{ opening, credits, debits, closing, hi - lo });
    }
};

fn number(text: []const u8) !i64 {
    return std.fmt.parseInt(i64, text, 10);
}

pub fn main(init: std.process.Init) !void {
    const io = init.io;
    const arena = init.arena.allocator();

    var in_buf: [1 << 16]u8 = undefined;
    var stdin = std.Io.File.stdin().reader(io, &in_buf);
    const input = try stdin.interface.allocRemaining(arena, .unlimited);
    var lines = std.mem.splitScalar(u8, input, '\n');
    const n = try std.fmt.parseInt(usize, lines.next().?, 10);

    var out_buf: [1 << 16]u8 = undefined;
    var stdout = std.Io.File.stdout().writer(io, &out_buf);
    const out = &stdout.interface;
    var ledger: Ledger = .{ .allocator = arena };
    for (0..n) |_| try ledger.process(lines.next().?, out);
    try out.flush();
}
