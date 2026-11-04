const OK = "OK";

class Account {
  closed = false;
  ledger = 0;
  held = 0;
  limit = 0;
  // Entries arrive in timestamp order. Before entry i, at times[i], the ledger balance was
  // balances[i] and the credits so far summed to credits[i]; the last element is the total.
  readonly times: number[] = [];
  readonly balances: number[] = [0];
  readonly credits: number[] = [0];

  post(t: number, amount: number): void {
    this.ledger += amount;
    this.times.push(t);
    this.balances.push(this.ledger);
    const credited = this.credits[this.credits.length - 1]!;
    this.credits.push(amount > 0 ? credited + amount : credited);
  }

  /** Whether taking `amount` out of the available balance keeps to the overdraft rule. */
  canSpend(amount: number): boolean {
    return this.ledger - this.held - amount >= -this.limit;
  }
}

class Hold {
  active = true;
  readonly account: Account;
  readonly amount: number;

  constructor(account: Account, amount: number) {
    this.account = account;
    this.amount = amount;
  }

  end(): void {
    this.active = false;
    this.account.held -= this.amount;
  }
}

type Posting = readonly [account: Account, amount: number];

/** An accepted deposit, withdrawal, transfer or capture: the entries it posted. */
class Movement {
  reversed = false;
  readonly postings: readonly Posting[];

  constructor(postings: readonly Posting[]) {
    this.postings = postings;
  }
}

interface Recorded {
  readonly text: string;
  readonly result: string;
}

class Ledger {
  clock = 0;
  readonly accounts = new Map<string, Account>();
  // Every recorded request id: its line after the timestamp, and the result it printed.
  readonly requests = new Map<string, Recorded>();
  readonly holds = new Map<string, Hold>();
  readonly movements = new Map<string, Movement>();

  process(line: string): string {
    const args = line.split(" ");
    const stamp = args[0]!;
    const t = Number(stamp);
    if (t < this.clock) return "ERR OUT_OF_ORDER";
    this.clock = t;
    switch (args[1]) {
      case "BALANCE":
        return this.balance(args[2]!);
      case "BALANCEAT":
        return this.balanceAt(args[2]!, Number(args[3]));
      case "STATEMENT":
        return this.statement(args[2]!, Number(args[3]), Number(args[4]));
    }
    const rid = args[2]!;
    const text = line.slice(stamp.length + 1);
    const recorded = this.requests.get(rid);
    if (recorded !== undefined) {
      return recorded.text === text ? recorded.result : "ERR ID_CONFLICT";
    }
    const result = this.execute(t, rid, args);
    this.requests.set(rid, { text, result });
    return result;
  }

  execute(t: number, rid: string, args: readonly string[]): string {
    switch (args[1]) {
      case "OPEN":
        return this.open(args[3]!);
      case "DEPOSIT":
        return this.deposit(t, rid, args[3]!, Number(args[4]));
      case "WITHDRAW":
        return this.withdraw(t, rid, args[3]!, Number(args[4]));
      case "TRANSFER":
        return this.transfer(t, rid, args[3]!, args[4]!, Number(args[5]));
      case "HOLD":
        return this.hold(rid, args[3]!, Number(args[4]));
      case "RELEASE":
        return this.release(args[3]!);
      case "CAPTURE":
        return this.capture(t, rid, args[3]!, Number(args[4]));
      case "REVERSE":
        return this.reverse(t, args[3]!);
      case "LIMIT":
        return this.limit(args[3]!, Number(args[4]));
      case "CLOSE":
        return this.close(args[3]!);
      default:
        throw new Error(`unknown command: ${args[1]}`);
    }
  }

  /** The open account named `name`, or the error line when there is none. */
  openAccount(name: string): Account | string {
    const account = this.accounts.get(name);
    if (account === undefined) return "ERR NO_ACCOUNT";
    if (account.closed) return "ERR ACCOUNT_CLOSED";
    return account;
  }

  /** The active hold placed by request `id`, or the error line when there is none. */
  activeHold(id: string): Hold | string {
    const hold = this.holds.get(id);
    if (hold === undefined) return "ERR NO_HOLD";
    if (!hold.active) return "ERR HOLD_INACTIVE";
    return hold;
  }

  move(rid: string, t: number, postings: readonly Posting[]): void {
    for (const [account, amount] of postings) account.post(t, amount);
    this.movements.set(rid, new Movement(postings));
  }

  open(name: string): string {
    if (this.accounts.has(name)) return "ERR ACCOUNT_EXISTS";
    this.accounts.set(name, new Account());
    return OK;
  }

  deposit(t: number, rid: string, name: string, amount: number): string {
    const account = this.openAccount(name);
    if (typeof account === "string") return account;
    this.move(rid, t, [[account, amount]]);
    return OK;
  }

  withdraw(t: number, rid: string, name: string, amount: number): string {
    const account = this.openAccount(name);
    if (typeof account === "string") return account;
    if (!account.canSpend(amount)) return "ERR INSUFFICIENT_FUNDS";
    this.move(rid, t, [[account, -amount]]);
    return OK;
  }

  transfer(t: number, rid: string, source: string, target: string, amount: number): string {
    if (source === target) return "ERR SAME_ACCOUNT";
    const payer = this.accounts.get(source);
    const payee = this.accounts.get(target);
    if (payer === undefined || payee === undefined) return "ERR NO_ACCOUNT";
    if (payer.closed || payee.closed) return "ERR ACCOUNT_CLOSED";
    if (!payer.canSpend(amount)) return "ERR INSUFFICIENT_FUNDS";
    this.move(rid, t, [
      [payer, -amount],
      [payee, amount],
    ]);
    return OK;
  }

  hold(rid: string, name: string, amount: number): string {
    const account = this.openAccount(name);
    if (typeof account === "string") return account;
    if (!account.canSpend(amount)) return "ERR INSUFFICIENT_FUNDS";
    account.held += amount;
    this.holds.set(rid, new Hold(account, amount));
    return OK;
  }

  release(id: string): string {
    const hold = this.activeHold(id);
    if (typeof hold === "string") return hold;
    hold.end();
    return OK;
  }

  capture(t: number, rid: string, id: string, amount: number): string {
    const hold = this.activeHold(id);
    if (typeof hold === "string") return hold;
    if (amount > hold.amount) return "ERR EXCEEDS_HOLD";
    hold.end();
    this.move(rid, t, [[hold.account, -amount]]);
    return OK;
  }

  reverse(t: number, target: string): string {
    const recorded = this.requests.get(target);
    if (recorded === undefined || recorded.result !== OK) return "ERR NO_TRANSACTION";
    const movement = this.movements.get(target);
    if (movement === undefined) return "ERR NOT_REVERSIBLE";
    if (movement.reversed) return "ERR ALREADY_REVERSED";
    const { postings } = movement;
    if (postings.some(([account]) => account.closed)) return "ERR ACCOUNT_CLOSED";
    // Undoing a credit is the only posting that debits an account.
    for (const [account, amount] of postings) {
      if (amount > 0 && !account.canSpend(amount)) return "ERR INSUFFICIENT_FUNDS";
    }
    movement.reversed = true;
    for (const [account, amount] of postings) account.post(t, -amount);
    return OK;
  }

  limit(name: string, value: number): string {
    const account = this.openAccount(name);
    if (typeof account === "string") return account;
    if (account.ledger - account.held < -value) return "ERR LIMIT_TOO_LOW";
    account.limit = value;
    return OK;
  }

  close(name: string): string {
    const account = this.openAccount(name);
    if (typeof account === "string") return account;
    if (account.held !== 0) return "ERR HOLDS_ACTIVE";
    if (account.ledger !== 0) return "ERR NONZERO_BALANCE";
    account.closed = true;
    return OK;
  }

  balance(name: string): string {
    const account = this.accounts.get(name);
    if (account === undefined) return "ERR NO_ACCOUNT";
    return `${account.ledger} ${account.ledger - account.held}`;
  }

  balanceAt(name: string, time: number): string {
    const account = this.accounts.get(name);
    if (account === undefined) return "ERR NO_ACCOUNT";
    return String(account.balances[partitionPoint(account.times, (t) => t <= time)]);
  }

  statement(name: string, start: number, end: number): string {
    if (start > end) return "ERR BAD_RANGE";
    const account = this.accounts.get(name);
    if (account === undefined) return "ERR NO_ACCOUNT";
    const { times, balances, credits } = account;
    const lo = partitionPoint(times, (t) => t < start);
    const hi = partitionPoint(times, (t) => t <= end);
    const opening = balances[lo]!;
    const closing = balances[hi]!;
    const credited = credits[hi]! - credits[lo]!;
    const debited = credited - (closing - opening);
    return `${opening} ${credited} ${debited} ${closing} ${hi - lo}`;
  }
}

/** The number of leading values of `sorted` that satisfy `pred`, which holds for a prefix. */
function partitionPoint(sorted: readonly number[], pred: (value: number) => boolean): number {
  let lo = 0;
  let hi = sorted.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (pred(sorted[mid]!)) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

async function main(): Promise<void> {
  const lines = (await Bun.stdin.text()).split("\n");
  const n = Number(lines[0]);
  const ledger = new Ledger();
  const results = lines.slice(1, n + 1).map((line) => ledger.process(line));
  if (n > 0) {
    await Bun.write(Bun.stdout, results.join("\n") + "\n");
  }
}

await main();
