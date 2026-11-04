"""Writes the cases (examples/ and hidden/) or the workloads/ for ledger.

Inputs come from fixed seeds. Expected outputs come from the Python reference. The cases are
committed; the workloads are large, so they are regenerated on demand and never committed.
Random streams are steered by the reference's own ledger state, so that boundary amounts,
settled closes and holds that are still active come up often.
"""

import hashlib
import random
import subprocess
import sys
from itertools import accumulate
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REFERENCE = ROOT / "reference" / "python" / "main.py"

sys.dont_write_bytecode = True
sys.path.insert(0, str(REFERENCE.parent))
from main import Ledger  # noqa: E402

MAX_T = 10**12
MAX_AMOUNT = 10**9
NAME_WORDS = ["acct", "alice", "bob", "carol", "dave", "erin", "frank", "ops", "payroll", "savings", "x"]
ID_PREFIXES = "abcdefghjkmnpqrstuvwxyz"


def write(directory: str, name: str, lines: list[str], expect: bool = True):
    path = ROOT / directory / f"{name}.in"
    path.parent.mkdir(exist_ok=True)
    path.write_text(f"{len(lines)}\n" + "".join(f"{line}\n" for line in lines))
    if expect:
        out = subprocess.run([sys.executable, REFERENCE], stdin=path.open("rb"), capture_output=True, check=True)
        path.with_suffix(".out").write_bytes(out.stdout)


def script(block: str) -> list[str]:
    """The lines of a hand-written case, one command per non-blank line."""
    return [line.strip() for line in block.strip().splitlines() if line.strip()]


class Stream:
    """A random command stream. Each command's arguments are drawn with the ledger's current state
    in view, so many commands land exactly on a rule's boundary or on one of its interactions."""

    def __init__(
        self,
        rng: random.Random,
        mix: dict[str, int],
        *,
        accounts: int,
        start: int = 1,
        steps: tuple[int, ...] = (0, 1, 1, 2, 3),
        scale: int = 1000,
        stray: float = 0.04,
        edge: float = 0.15,
    ):
        self.rng = rng
        self.actions = [getattr(self, action) for action in mix]
        self.cum_weights = list(accumulate(mix.values()))
        self.steps = steps
        self.scale = scale
        self.stray = stray
        self.edge = edge
        self.ledger = Ledger()
        self.lines: list[str] = []
        self.start = self.t = start
        self.serial = 0
        self.names: list[str] = []
        self.live: list[str] = []
        self.holds_of: dict[str, list[str]] = {}
        self.texts: list[str] = []
        self.targets: dict[str, list[str]] = {"movement": [], "hold": [], "reverse": [], "other": []}
        for _ in range(accounts):
            self.open_new()

    def run(self, count: int) -> list[str]:
        for action in self.rng.choices(self.actions, cum_weights=self.cum_weights, k=count):
            action()
        return self.lines

    def emit(self, text: str, kind: str | None = None, t: int | None = None):
        if t is None:
            self.t = t = min(MAX_T, self.t + self.rng.choice(self.steps))
        line = f"{t} {text}"
        self.ledger.process(line)
        self.lines.append(line)
        if kind is not None:
            rid = text.split(" ", 2)[1]
            self.texts.append(text)
            accepted = {"hold": self.ledger.holds, "movement": self.ledger.movements}.get(kind)
            self.targets["other" if accepted is not None and rid not in accepted else kind].append(rid)

    def fresh_id(self) -> str:
        self.serial += 1
        return f"{self.rng.choice(ID_PREFIXES)}{self.serial}"

    def recent(self, items: list[str]) -> str:
        """Leans toward the newest items, which are likelier to be active holds or unreversed."""
        back = min(len(items) - 1, int(self.rng.expovariate(1 / 8)))
        return items[-1 - back] if self.rng.random() < 0.7 else self.rng.choice(items)

    def any_id(self) -> str:
        pools = [pool for pool in self.targets.values() if pool]
        if not pools or self.rng.random() < 0.3:
            return f"none{self.rng.randrange(10**4)}"
        return self.rng.choice(self.rng.choice(pools))

    def open_new(self):
        name = f"{self.rng.choice(NAME_WORDS)}{self.rng.choice('-_')}{len(self.names) + 1}"
        self.names.append(name)
        self.live.append(name)
        self.emit(f"OPEN {self.fresh_id()} {name}", "other")

    def some_account(self) -> str:
        """Usually an open account, sometimes any account ever opened, sometimes one never opened."""
        roll = self.rng.random()
        if roll < self.stray:
            return f"ghost{self.rng.randrange(100)}"
        if roll < 2 * self.stray or not self.live:
            return self.rng.choice(self.names)
        return self.rng.choice(self.live)

    def amount(self) -> int:
        if self.rng.random() < 0.01:
            return self.rng.randint(1, MAX_AMOUNT)
        return self.rng.randint(1, self.scale)

    def headroom(self, name: str) -> int:
        """How much the overdraft rule lets the account spend now."""
        account = self.ledger.accounts.get(name)
        return 0 if account is None else account.ledger - account.held + account.limit

    def spend(self, name: str) -> int:
        room = self.headroom(name)
        if self.rng.random() < self.edge:
            return min(MAX_AMOUNT, max(1, room + self.rng.choice((0, 0, 1))))
        return self.rng.randint(1, min(MAX_AMOUNT, max(1, room + room // 4)))

    def open(self):
        if self.rng.random() < 0.1:
            self.emit(f"OPEN {self.fresh_id()} {self.rng.choice(self.names)}", "other")
        else:
            self.open_new()

    def deposit(self):
        self.emit(f"DEPOSIT {self.fresh_id()} {self.some_account()} {self.amount()}", "movement")

    def withdraw(self):
        name = self.some_account()
        self.emit(f"WITHDRAW {self.fresh_id()} {name} {self.spend(name)}", "movement")

    def transfer(self):
        payer = self.some_account()
        payee = payer if self.rng.random() < self.stray else self.some_account()
        self.emit(f"TRANSFER {self.fresh_id()} {payer} {payee} {self.spend(payer)}", "movement")

    def hold(self):
        name = self.some_account()
        rid = self.fresh_id()
        self.emit(f"HOLD {rid} {name} {self.spend(name)}", "hold")
        self.holds_of.setdefault(name, []).append(rid)

    def pick_hold(self) -> str:
        holds = self.targets["hold"]
        if holds and self.rng.random() >= 2 * self.stray:
            return self.recent(holds)
        return self.any_id()

    def release(self):
        self.emit(f"RELEASE {self.fresh_id()} {self.pick_hold()}", "other")

    def capture(self):
        hold_id = self.pick_hold()
        hold = self.ledger.holds.get(hold_id)
        size = hold.amount if hold else self.amount()
        roll = self.rng.random()
        amount = size if roll < 0.3 else size % MAX_AMOUNT + 1 if roll < 0.3 + self.edge else self.rng.randint(1, size)
        self.emit(f"CAPTURE {self.fresh_id()} {hold_id} {amount}", "movement")

    def reverse(self):
        roll = self.rng.random()
        if roll < 0.8 and self.targets["movement"]:
            target = self.recent(self.targets["movement"])
        elif roll < 0.88 and self.targets["reverse"]:
            target = self.recent(self.targets["reverse"])
        else:
            target = self.any_id()
        self.emit(f"REVERSE {self.fresh_id()} {target}", "reverse")

    def limit(self):
        name = self.some_account()
        account = self.ledger.accounts.get(name)
        available = account.ledger - account.held if account else 0
        roll = self.rng.random()
        if available < 0 and roll < 0.5:
            value = max(0, -available - self.rng.choice((0, 0, 1)))
        elif roll < 0.6:
            value = 0
        else:
            value = self.rng.randint(1, self.scale)
        self.emit(f"LIMIT {self.fresh_id()} {name} {value}", "other")

    def close(self):
        """Usually settles the account first: releases its active holds and zeroes its ledger."""
        name = self.some_account()
        account = self.ledger.accounts.get(name)
        if account and not account.closed and self.rng.random() < 0.7:
            for hold_id in self.holds_of.pop(name, []):
                if self.ledger.holds.get(hold_id) and self.ledger.holds[hold_id].active:
                    self.emit(f"RELEASE {self.fresh_id()} {hold_id}", "other")
            while account.ledger > 0:
                self.emit(f"WITHDRAW {self.fresh_id()} {name} {min(MAX_AMOUNT, account.ledger)}", "movement")
            if account.ledger < 0:
                self.emit(f"DEPOSIT {self.fresh_id()} {name} {-account.ledger}", "movement")
        self.emit(f"CLOSE {self.fresh_id()} {name}", "other")
        if account and account.closed and name in self.live:
            self.live.remove(name)
        if not self.live or self.rng.random() < 0.5:
            self.open_new()

    def retry(self):
        if self.texts:
            self.emit(self.recent(self.texts))

    def conflict(self):
        if self.texts:
            command, rid, rest = self.recent(self.texts).split(" ", 2)
            if self.rng.random() < 0.5 and command in ("DEPOSIT", "WITHDRAW", "HOLD", "CAPTURE", "LIMIT"):
                head, amount = rest.rsplit(" ", 1)
                self.emit(f"{command} {rid} {head} {int(amount) % MAX_AMOUNT + 1}")
            else:
                self.emit(f"DEPOSIT {rid} {self.some_account()} {self.amount()}")

    def out_of_order(self):
        if self.t > self.start:
            t = self.rng.randint(max(self.start, self.t - 5), self.t - 1)
            if self.texts and self.rng.random() < 0.5:
                self.emit(self.recent(self.texts), t=t)
            else:
                self.emit(f"DEPOSIT {self.fresh_id()} {self.some_account()} {self.amount()}", t=t)

    def past(self) -> int:
        if self.rng.random() < 0.05:
            return self.rng.randint(0, min(MAX_T, self.t + 10))
        return self.rng.randint(self.start, self.t)

    def balance(self):
        self.emit(f"BALANCE {self.some_account()}")

    def balance_at(self):
        self.emit(f"BALANCEAT {self.some_account()} {self.past()}")

    def statement(self):
        start, end = self.past(), self.past()
        if self.rng.random() >= self.stray:
            start, end = min(start, end), max(start, end)
        self.emit(f"STATEMENT {self.some_account()} {start} {end}")


MIXED = dict(
    open=2, deposit=20, withdraw=14, transfer=14, hold=6, release=3, capture=3, reverse=5, limit=2,
    close=1, retry=2, conflict=1, out_of_order=1, balance=8, balance_at=10, statement=9,
)
HOLDS = dict(deposit=10, withdraw=4, hold=16, release=8, capture=10, reverse=6, limit=2, close=2, retry=2, balance=8, balance_at=3, statement=3)
REVERSALS = dict(deposit=12, withdraw=8, transfer=10, hold=3, capture=3, reverse=20, close=3, retry=3, conflict=1, balance=6, balance_at=6, statement=6)
ERRORS = dict(
    open=3, deposit=8, withdraw=10, transfer=10, hold=5, release=5, capture=5, reverse=8, limit=5,
    close=6, retry=8, conflict=6, out_of_order=6, balance=4, balance_at=4, statement=4,
)
HISTORY = dict(deposit=24, withdraw=14, transfer=18, hold=2, release=1, capture=1, reverse=2, balance=4, balance_at=18, statement=16)


def edge_cases():
    yield "empty", ""
    yield "single-open", "0 OPEN a acct"
    yield "queries-missing-account", """
        1 OPEN o1 real
        2 BALANCE ghost
        3 BALANCEAT ghost 2
        4 STATEMENT ghost 0 5
        5 BALANCE real
        6 BALANCEAT real 6
        7 STATEMENT real 0 7
    """
    yield "fresh-account-queries", """
        5 OPEN o1 a
        5 BALANCE a
        5 BALANCEAT a 0
        5 BALANCEAT a 1000000000000
        5 STATEMENT a 0 1000000000000
        5 STATEMENT a 5 5
    """
    yield "deposit-withdraw-basic", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 500
        3 WITHDRAW w1 a 200
        4 DEPOSIT d2 a 1
        5 WITHDRAW w2 a 301
        6 BALANCE a
        7 BALANCEAT a 2
        7 BALANCEAT a 3
        7 STATEMENT a 0 7
    """
    yield "names-shared-by-accounts-and-ids", """
        1 OPEN alice alice
        2 OPEN bob bob
        3 DEPOSIT carol alice 100
        4 OPEN dave carol
        5 TRANSFER bob alice carol 40
        6 REVERSE x1 alice
        7 REVERSE x2 carol
        8 REVERSE x3 bob
        9 RELEASE x4 alice
        10 BALANCE alice
        10 BALANCE carol
        10 BALANCE bob
        10 BALANCE dave
    """
    yield "large-values", """
        0 OPEN o1 big
        0 OPEN o2 other
        1 DEPOSIT d1 big 1000000000
        2 DEPOSIT d2 big 1000000000
        3 DEPOSIT d3 big 1000000000
        4 DEPOSIT d4 big 1000000000
        5 DEPOSIT d5 big 1000000000
        6 BALANCE big
        7 LIMIT l1 other 1000000000
        8 WITHDRAW w1 other 1000000000
        9 HOLD h1 big 1000000000
        10 TRANSFER t1 big other 1000000000
        11 BALANCE big
        11 BALANCE other
        999999999999 DEPOSIT d6 other 1000000000
        1000000000000 DEPOSIT d7 big 1000000000
        1000000000000 BALANCEAT big 999999999999
        1000000000000 STATEMENT big 0 1000000000000
        1000000000000 STATEMENT other 1000000000000 1000000000000
    """


def timestamp_cases():
    yield "out-of-order-basic", """
        10 OPEN o1 a
        10 DEPOSIT d1 a 5
        9 DEPOSIT d2 a 7
        10 DEPOSIT d3 a 11
        12 BALANCE a
        11 BALANCE a
        12 BALANCEAT a 12
    """
    yield "out-of-order-id-not-recorded", """
        10 OPEN o1 a
        5 DEPOSIT d1 a 10
        11 DEPOSIT d1 a 20
        12 DEPOSIT d1 a 20
        13 BALANCE a
        3 OPEN o2 b
        14 OPEN o2 b
        15 BALANCE b
    """
    yield "out-of-order-query", """
        100 OPEN o1 a
        200 DEPOSIT d1 a 1
        150 BALANCE a
        150 BALANCEAT a 300
        150 STATEMENT a 0 300
        200 BALANCEAT a 300
    """
    yield "rejected-line-advances-clock", """
        1 OPEN o1 a
        10 WITHDRAW w1 a 999
        9 DEPOSIT d1 a 5
        20 BALANCE ghost
        19 DEPOSIT d2 a 5
        30 OPEN o1 a
        29 DEPOSIT d3 a 5
        40 OPEN o9 a
        39 DEPOSIT d4 a 5
        40 BALANCE a
    """
    yield "retry-and-conflict-advance-clock", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 50
        20 DEPOSIT d1 a 50
        15 WITHDRAW w1 a 10
        30 DEPOSIT d1 a 51
        25 WITHDRAW w2 a 10
        30 WITHDRAW w3 a 10
        30 BALANCE a
    """
    yield "out-of-order-before-id-check", """
        5 OPEN o1 a
        6 DEPOSIT d1 a 10
        4 DEPOSIT d1 a 10
        4 DEPOSIT d1 a 99
        7 DEPOSIT d1 a 10
        7 BALANCE a
    """
    yield "equal-timestamps", """
        7 OPEN o1 a
        7 OPEN o2 b
        7 DEPOSIT d1 a 100
        7 TRANSFER t1 a b 30
        7 WITHDRAW w1 b 30
        7 DEPOSIT d2 b 5
        7 BALANCEAT a 6
        7 BALANCEAT a 7
        7 BALANCEAT b 7
        7 STATEMENT a 7 7
        7 STATEMENT b 7 7
        8 STATEMENT b 0 6
    """
    yield "timestamp-extremes", """
        0 OPEN o1 a
        0 DEPOSIT d1 a 1
        0 BALANCEAT a 0
        1000000000000 DEPOSIT d2 a 2
        1000000000000 BALANCEAT a 999999999999
        1000000000000 BALANCEAT a 1000000000000
        999999999999 DEPOSIT d3 a 4
        1000000000000 STATEMENT a 0 0
        1000000000000 STATEMENT a 1 1000000000000
    """


def retry_cases():
    yield "retry-ok-deposit-not-reapplied", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 DEPOSIT d1 a 100
        4 DEPOSIT d1 a 100
        5 BALANCE a
        5 STATEMENT a 0 5
    """
    yield "retry-rejected-stays-rejected", """
        1 OPEN o1 a
        2 WITHDRAW w1 a 50
        3 DEPOSIT d1 a 100
        4 WITHDRAW w1 a 50
        5 WITHDRAW w2 a 50
        6 BALANCE a
    """
    yield "retry-ok-withdraw-after-drain", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 80
        4 WITHDRAW w2 a 20
        5 WITHDRAW w1 a 80
        6 BALANCE a
        7 WITHDRAW w3 a 80
    """
    yield "conflict-variants", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 DEPOSIT d1 a 101
        4 DEPOSIT d1 b 100
        5 WITHDRAW d1 a 100
        6 HOLD d1 a 100
        7 OPEN d1 c
        8 DEPOSIT d1 a 100
        9 BALANCE a
        9 BALANCE b
        9 BALANCE c
    """
    yield "conflict-keeps-original", """
        1 OPEN o1 a
        2 WITHDRAW w1 a 10
        3 WITHDRAW w1 a 5
        4 DEPOSIT d1 a 20
        5 WITHDRAW w1 a 5
        6 WITHDRAW w1 a 10
        7 BALANCE a
    """
    yield "retry-open-after-close", """
        1 OPEN o1 a
        2 CLOSE c1 a
        3 OPEN o1 a
        4 OPEN o2 a
        5 DEPOSIT d1 a 5
        6 CLOSE c1 a
        7 CLOSE c2 a
        8 BALANCE a
    """
    yield "retry-reverse", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 REVERSE r1 d1
        4 REVERSE r1 d1
        5 REVERSE r2 d1
        6 BALANCE a
        7 STATEMENT a 0 7
    """
    yield "retry-hold-release-capture", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 40
        4 HOLD h1 a 40
        5 BALANCE a
        6 RELEASE r1 h1
        7 RELEASE r1 h1
        8 RELEASE r2 h1
        9 HOLD h2 a 30
        10 CAPTURE c1 h2 10
        11 CAPTURE c1 h2 10
        12 CAPTURE c2 h2 10
        13 BALANCE a
    """
    yield "retry-rejected-no-account-then-open", """
        1 DEPOSIT d1 a 10
        2 OPEN o1 a
        3 DEPOSIT d1 a 10
        4 DEPOSIT d2 a 10
        5 BALANCE a
        6 REVERSE r1 d1
        7 REVERSE r2 d2
        8 BALANCE a
    """
    yield "retry-with-later-timestamp", """
        1 OPEN o1 a
        2 OPEN o2 b
        3 DEPOSIT d1 a 70
        4 TRANSFER t1 a b 50
        100 TRANSFER t1 a b 50
        100 BALANCE a
        100 BALANCE b
        100 STATEMENT a 0 99
        100 STATEMENT a 100 100
    """
    yield "id-namespace-shared-by-all-commands", """
        1 OPEN x a
        2 DEPOSIT x a 5
        3 LIMIT x a 5
        4 HOLD h a 5
        5 RELEASE h h
        6 CAPTURE h h 5
        7 REVERSE h h
        8 CLOSE h a
        9 BALANCE a
    """


def account_cases():
    yield "open-existing-and-closed", """
        1 OPEN o1 a
        2 OPEN o2 a
        3 CLOSE c1 a
        4 OPEN o3 a
        5 OPEN o4 b
        6 BALANCE a
        6 BALANCE b
    """
    yield "close-nonzero-balance", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 1
        3 CLOSE c1 a
        4 LIMIT l1 b 10
        5 WITHDRAW w1 b 1
        6 CLOSE c2 b
        7 WITHDRAW w2 a 1
        8 CLOSE c3 a
        9 DEPOSIT d2 b 1
        10 CLOSE c4 b
    """
    yield "close-hold-on-zero-ledger", """
        1 OPEN o1 a
        2 LIMIT l1 a 100
        3 HOLD h1 a 50
        4 BALANCE a
        5 CLOSE c1 a
        6 RELEASE r1 h1
        7 CLOSE c2 a
        8 BALANCE a
    """
    yield "close-holds-checked-before-balance", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 10
        4 CLOSE c1 a
        5 CAPTURE k1 h1 10
        6 CLOSE c2 a
        7 WITHDRAW w1 a 90
        8 CLOSE c3 a
    """
    yield "close-twice", """
        1 OPEN o1 a
        2 CLOSE c1 a
        3 CLOSE c2 a
        4 CLOSE c3 ghost
    """
    yield "closed-account-rejects-changes", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 CLOSE c1 a
        3 DEPOSIT d1 a 10
        4 WITHDRAW w1 a 10
        5 HOLD h1 a 10
        6 LIMIT l1 a 10
        7 DEPOSIT d2 b 10
        8 TRANSFER t1 b a 5
        9 TRANSFER t2 a b 5
        10 CLOSE c2 a
        11 BALANCE a
        11 BALANCE b
        11 RELEASE r1 h1
    """
    yield "closed-account-keeps-history", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 30
        4 WITHDRAW w2 a 70
        5 CLOSE c1 a
        6 BALANCE a
        6 BALANCEAT a 2
        6 BALANCEAT a 3
        6 STATEMENT a 2 4
        6 STATEMENT a 3 6
    """
    yield "close-after-full-capture", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 100
        4 CAPTURE k1 h1 100
        5 CLOSE c1 a
        6 BALANCE a
    """
    yield "close-after-partial-capture", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 100
        4 CAPTURE k1 h1 60
        5 CLOSE c1 a
        6 BALANCE a
        7 WITHDRAW w1 a 40
        8 CLOSE c2 a
    """
    yield "close-after-reversal", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 CLOSE c1 a
        4 REVERSE r1 d1
        5 CLOSE c2 a
        6 STATEMENT a 0 6
    """


def funds_cases():
    yield "withdraw-exact-available", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 101
        4 WITHDRAW w2 a 100
        5 WITHDRAW w3 a 1
        6 BALANCE a
    """
    yield "overdraft-boundary", """
        1 OPEN o1 a
        2 LIMIT l1 a 100
        3 WITHDRAW w1 a 101
        4 WITHDRAW w2 a 100
        5 WITHDRAW w3 a 1
        6 BALANCE a
        7 DEPOSIT d1 a 30
        8 WITHDRAW w4 a 31
        9 WITHDRAW w5 a 30
        10 BALANCE a
    """
    yield "holds-reduce-available", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 60
        4 WITHDRAW w1 a 41
        5 WITHDRAW w2 a 40
        6 BALANCE a
        7 HOLD h2 a 1
        8 RELEASE r1 h1
        9 HOLD h3 a 60
        10 BALANCE a
    """
    yield "hold-uses-overdraft", """
        1 OPEN o1 a
        2 LIMIT l1 a 50
        3 HOLD h1 a 51
        4 HOLD h2 a 50
        5 BALANCE a
        6 DEPOSIT d1 a 20
        7 WITHDRAW w1 a 21
        8 WITHDRAW w2 a 20
        9 BALANCE a
    """
    yield "limit-lowering", """
        1 OPEN o1 a
        2 LIMIT l1 a 100
        3 WITHDRAW w1 a 50
        4 LIMIT l2 a 49
        5 LIMIT l3 a 50
        6 LIMIT l4 a 0
        7 DEPOSIT d1 a 60
        8 LIMIT l5 a 0
        9 WITHDRAW w2 a 11
        10 BALANCE a
    """
    yield "limit-counts-holds", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 LIMIT l1 a 50
        4 HOLD h1 a 150
        5 LIMIT l2 a 0
        6 LIMIT l3 a 49
        7 LIMIT l4 a 50
        8 RELEASE r1 h1
        9 LIMIT l5 a 0
        10 BALANCE a
    """
    yield "funds-use-available-not-ledger", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 100
        4 WITHDRAW w1 a 1
        5 TRANSFER t1 a b 1
        6 HOLD h2 a 1
        7 BALANCE a
        8 CAPTURE k1 h1 30
        9 WITHDRAW w2 a 70
        10 BALANCE a
    """
    yield "limit-raise-and-reuse", """
        1 OPEN o1 a
        2 LIMIT l1 a 0
        3 WITHDRAW w1 a 1
        4 LIMIT l2 a 1000
        5 WITHDRAW w2 a 1000
        6 LIMIT l3 a 999
        7 LIMIT l4 a 1000000000
        8 WITHDRAW w3 a 999999000
        9 BALANCE a
        10 WITHDRAW w4 a 1
    """


def transfer_cases():
    yield "transfer-same-account-first", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 10
        3 TRANSFER t1 a a 5
        4 TRANSFER t2 ghost ghost 5
        5 CLOSE c1 b
        6 OPEN o2 b
        7 CLOSE c2 b
        8 TRANSFER t3 b b 5
        9 TRANSFER t4 a a 500
        10 BALANCE a
    """
    yield "transfer-missing-before-closed", """
        1 OPEN o1 a
        2 OPEN o2 c
        3 CLOSE c1 c
        4 TRANSFER t1 c ghost 5
        5 TRANSFER t2 ghost c 5
        6 TRANSFER t3 a ghost 5
        7 TRANSFER t4 ghost a 5
        8 TRANSFER t5 a c 5
        9 TRANSFER t6 c a 5
    """
    yield "transfer-funds-checked-on-payer", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 LIMIT l1 b 100
        3 WITHDRAW w1 b 100
        4 DEPOSIT d1 a 50
        5 TRANSFER t1 a b 51
        6 TRANSFER t2 a b 50
        7 TRANSFER t3 b a 50
        8 TRANSFER t4 b a 1
        9 BALANCE a
        9 BALANCE b
    """
    yield "transfer-statements-both-sides", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 1000
        3 TRANSFER t1 a b 300
        4 TRANSFER t2 b a 100
        5 TRANSFER t3 a b 250
        6 STATEMENT a 0 6
        6 STATEMENT b 0 6
        6 STATEMENT a 3 4
        6 STATEMENT b 4 5
        6 BALANCEAT b 4
    """


def hold_cases():
    yield "release-unknown-and-non-hold", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 RELEASE r1 nope
        4 RELEASE r2 d1
        5 RELEASE r3 a
        6 RELEASE r4 o1
        7 CAPTURE k1 d1 5
        8 CAPTURE k2 nope 5
        9 BALANCE a
    """
    yield "release-rejected-hold", """
        1 OPEN o1 a
        2 HOLD h1 a 10
        3 RELEASE r1 h1
        4 CAPTURE k1 h1 10
        5 DEPOSIT d1 a 10
        6 HOLD h1 a 10
        7 RELEASE r2 h1
        8 BALANCE a
    """
    yield "release-twice", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 30
        4 RELEASE r1 h1
        5 RELEASE r2 h1
        6 BALANCE a
    """
    yield "capture-partial-ends-hold", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 80
        4 CAPTURE k1 h1 30
        5 BALANCE a
        6 RELEASE r1 h1
        7 CAPTURE k2 h1 10
        8 WITHDRAW w1 a 70
        9 BALANCE a
    """
    yield "capture-exceeds-keeps-hold", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 40
        4 CAPTURE k1 h1 41
        5 BALANCE a
        6 CAPTURE k2 h1 40
        7 BALANCE a
        8 CAPTURE k3 h1 1
    """
    yield "capture-inactive-before-exceeds", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 40
        4 RELEASE r1 h1
        5 CAPTURE k1 h1 1000
        6 HOLD h2 a 10
        7 CAPTURE k2 h2 5
        8 CAPTURE k3 h2 1000
    """
    yield "holds-leave-history-alone", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 50
        4 HOLD h2 a 20
        5 RELEASE r1 h1
        6 BALANCEAT a 4
        6 STATEMENT a 0 6
        6 STATEMENT a 3 6
        7 CAPTURE k1 h2 15
        8 BALANCEAT a 6
        8 BALANCEAT a 7
        8 STATEMENT a 3 8
        8 BALANCE a
    """
    yield "many-holds", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 10
        3 HOLD h2 a 20
        3 HOLD h3 a 30
        3 HOLD h4 a 40
        3 HOLD h5 a 1
        4 BALANCE a
        5 RELEASE r1 h2
        5 CAPTURE k1 h3 30
        5 CAPTURE k2 h4 1
        6 BALANCE a
        7 HOLD h6 a 50
        8 HOLD h7 a 49
        9 BALANCE a
    """
    yield "capture-own-id", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 10
        3 CAPTURE k1 k1 5
        4 RELEASE r1 r1
        5 HOLD h1 a 5
        6 CAPTURE h1 h1 5
        7 BALANCE a
    """
    yield "capture-on-overdrawn-account", """
        1 OPEN o1 a
        2 LIMIT l1 a 100
        3 HOLD h1 a 100
        4 CAPTURE k1 h1 100
        5 BALANCE a
        6 LIMIT l2 a 99
        7 HOLD h2 a 1
        8 DEPOSIT d1 a 100
        9 CLOSE c1 a
    """


def reversal_cases():
    yield "reverse-deposit-history", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        5 REVERSE r1 d1
        6 BALANCE a
        6 BALANCEAT a 1
        6 BALANCEAT a 2
        6 BALANCEAT a 4
        6 BALANCEAT a 5
        6 STATEMENT a 0 6
        6 STATEMENT a 3 6
        6 STATEMENT a 0 4
    """
    yield "reverse-withdraw", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 100
        4 WITHDRAW w2 a 1
        5 REVERSE r1 w1
        6 BALANCE a
        7 STATEMENT a 3 5
    """
    yield "reverse-transfer-both-sides", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 60
        4 REVERSE r1 t1
        5 BALANCE a
        5 BALANCE b
        5 BALANCEAT b 3
        5 STATEMENT a 0 5
        5 STATEMENT b 0 5
    """
    yield "reverse-capture-hold-stays-ended", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 70
        4 CAPTURE k1 h1 50
        5 REVERSE r1 k1
        6 BALANCE a
        7 RELEASE r2 h1
        8 CAPTURE k2 h1 10
        9 REVERSE r3 h1
        10 STATEMENT a 0 10
    """
    yield "reverse-reversal-not-reversible", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 REVERSE r1 d1
        4 REVERSE r2 r1
        5 REVERSE r3 r1
        6 BALANCE a
    """
    yield "reverse-twice", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 40
        4 REVERSE r1 w1
        5 REVERSE r2 w1
        6 REVERSE r3 w1
        7 BALANCE a
    """
    yield "reverse-unknown-rejected-self", """
        1 OPEN o1 a
        2 WITHDRAW w1 a 10
        3 REVERSE r1 nope
        4 REVERSE r2 w1
        5 REVERSE r3 r3
        6 REVERSE r4 a
        7 REVERSE r5 r2
        8 BALANCE a
    """
    yield "reverse-non-movements", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 10
        4 RELEASE e1 h1
        5 LIMIT l1 a 5
        6 OPEN o2 b
        7 CLOSE c1 b
        8 REVERSE r1 o1
        9 REVERSE r2 h1
        10 REVERSE r3 e1
        11 REVERSE r4 l1
        12 REVERSE r5 c1
        13 REVERSE r6 d1
        14 REVERSE r7 r6
        15 BALANCE a
    """
    yield "reverse-deposit-insufficient", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 30
        4 REVERSE r1 d1
        5 LIMIT l1 a 29
        6 REVERSE r2 d1
        7 LIMIT l2 a 30
        8 REVERSE r3 d1
        9 BALANCE a
        10 REVERSE r1 d1
    """
    yield "reverse-deposit-blocked-by-hold", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 DEPOSIT d2 a 50
        4 HOLD h1 a 60
        5 REVERSE r1 d1
        6 REVERSE r2 d2
        7 BALANCE a
        8 RELEASE e1 h1
        9 REVERSE r3 d1
        10 BALANCE a
    """
    yield "reverse-transfer-payee-lacks-funds", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 100
        4 WITHDRAW w1 b 1
        5 REVERSE r1 t1
        6 LIMIT l1 b 1
        7 REVERSE r2 t1
        8 BALANCE a
        8 BALANCE b
    """
    yield "reverse-transfer-closed-sides", """
        1 OPEN o1 a
        1 OPEN o2 b
        1 OPEN o3 c
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 100
        4 WITHDRAW w1 b 100
        5 CLOSE c1 b
        6 REVERSE r1 t1
        7 DEPOSIT d2 c 10
        8 TRANSFER t3 c a 10
        9 CLOSE c2 c
        10 REVERSE r2 t3
        11 REVERSE r3 w1
    """
    yield "reverse-credit-to-closed-account", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 WITHDRAW w1 a 100
        4 CLOSE c1 a
        5 REVERSE r1 w1
        6 REVERSE r2 d1
        7 BALANCE a
    """
    yield "reverse-capture-closed-account", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 HOLD h1 a 100
        4 CAPTURE k1 h1 100
        5 CLOSE c1 a
        6 REVERSE r1 k1
        7 BALANCE a
    """
    yield "reverse-already-before-closed", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 100
        3 REVERSE r1 d1
        4 CLOSE c1 a
        5 REVERSE r2 d1
        6 REVERSE r3 r1
        7 REVERSE r4 o1
    """
    yield "reverse-failed-then-new-id", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 100
        4 TRANSFER t2 b a 100
        5 REVERSE r1 t1
        6 DEPOSIT d2 b 100
        7 REVERSE r1 t1
        8 REVERSE r2 t1
        9 REVERSE r3 t1
        10 BALANCE a
        10 BALANCE b
    """
    yield "reverse-same-timestamp", """
        3 OPEN o1 a
        3 DEPOSIT d1 a 100
        3 REVERSE r1 d1
        3 DEPOSIT d2 a 7
        3 BALANCEAT a 2
        3 BALANCEAT a 3
        3 STATEMENT a 3 3
    """
    yield "reverse-transfer-into-overdraft", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 100
        4 LIMIT l1 b 1000
        5 WITHDRAW w1 b 500
        6 REVERSE r1 t1
        7 BALANCE a
        7 BALANCE b
        8 LIMIT l2 b 499
        9 LIMIT l3 b 500
    """


def query_cases():
    yield "balanceat-boundaries", """
        10 OPEN o1 a
        20 DEPOSIT d1 a 1
        30 DEPOSIT d2 a 10
        30 DEPOSIT d3 a 100
        40 DEPOSIT d4 a 1000
        50 BALANCEAT a 0
        50 BALANCEAT a 19
        50 BALANCEAT a 20
        50 BALANCEAT a 21
        50 BALANCEAT a 29
        50 BALANCEAT a 30
        50 BALANCEAT a 39
        50 BALANCEAT a 40
        50 BALANCEAT a 41
        50 BALANCEAT a 1000000000000
    """
    yield "statement-bounds-inclusive", """
        10 OPEN o1 a
        20 DEPOSIT d1 a 1
        30 WITHDRAW w1 a 1
        30 DEPOSIT d2 a 100
        40 WITHDRAW w2 a 10
        50 STATEMENT a 20 40
        50 STATEMENT a 21 39
        50 STATEMENT a 30 30
        50 STATEMENT a 31 39
        50 STATEMENT a 41 50
        50 STATEMENT a 0 19
        50 STATEMENT a 0 0
        50 STATEMENT a 40 1000000000000
    """
    yield "statement-bad-range-first", """
        1 OPEN o1 a
        2 STATEMENT a 5 4
        3 STATEMENT ghost 5 4
        4 STATEMENT ghost 4 5
        5 STATEMENT a 4 4
        6 STATEMENT a 1000000000000 0
    """
    yield "statement-debits-are-magnitudes", """
        1 OPEN o1 a
        2 LIMIT l1 a 1000
        3 WITHDRAW w1 a 300
        4 WITHDRAW w2 a 200
        5 DEPOSIT d1 a 50
        6 STATEMENT a 0 6
        6 STATEMENT a 4 6
        6 BALANCEAT a 4
    """
    yield "query-future-then-more-entries", """
        1 OPEN o1 a
        2 DEPOSIT d1 a 5
        3 BALANCEAT a 100
        3 STATEMENT a 0 100
        50 DEPOSIT d2 a 7
        60 BALANCEAT a 100
        60 STATEMENT a 0 100
        60 BALANCEAT a 49
    """
    yield "many-entries-same-timestamp", """
        1 OPEN o1 a
        1 OPEN o2 b
        5 DEPOSIT d1 a 10
        5 DEPOSIT d2 a 20
        5 TRANSFER t1 a b 5
        5 TRANSFER t2 b a 3
        5 WITHDRAW w1 a 1
        5 REVERSE r1 d2
        6 STATEMENT a 5 5
        6 STATEMENT b 5 5
        6 BALANCEAT a 4
        6 BALANCEAT a 5
        6 STATEMENT a 0 4
        6 STATEMENT a 6 6
    """


def story_cases():
    yield "story-card-authorization", """
        100 OPEN o1 card
        100 OPEN o2 shop
        101 DEPOSIT pay1 card 5000
        102 HOLD auth1 card 1200
        103 HOLD auth2 card 3000
        104 HOLD auth3 card 900
        105 BALANCE card
        106 CAPTURE cap1 auth1 1150
        107 RELEASE rel1 auth2
        108 TRANSFER tr1 card shop 3000
        109 CAPTURE cap2 auth3 901
        110 CAPTURE cap2 auth3 901
        111 CAPTURE cap3 auth3 900
        112 BALANCE card
        113 REVERSE rv1 cap1
        114 REVERSE rv2 tr1
        115 BALANCE card
        115 BALANCE shop
        116 STATEMENT card 100 116
        116 STATEMENT card 106 112
        116 BALANCEAT card 109
    """
    yield "story-overdraft-and-close", """
        1 OPEN o1 main
        1 OPEN o2 side
        2 LIMIT l1 main 500
        3 TRANSFER t1 main side 500
        4 TRANSFER t2 main side 1
        5 CLOSE c1 main
        6 LIMIT l2 main 0
        7 TRANSFER t3 side main 500
        8 LIMIT l2 main 0
        9 LIMIT l3 main 0
        10 CLOSE c2 main
        11 CLOSE c3 side
        12 REVERSE r1 t3
        13 REVERSE r2 t1
        14 BALANCE main
        14 BALANCE side
        14 STATEMENT main 0 14
    """
    yield "story-retries-across-closing", """
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 300
        3 TRANSFER t1 a b 300
        4 CLOSE c1 a
        5 TRANSFER t1 a b 300
        6 DEPOSIT d1 a 300
        7 DEPOSIT d2 a 300
        8 REVERSE r1 t1
        9 WITHDRAW w1 b 300
        10 CLOSE c2 b
        11 REVERSE r1 t1
        12 REVERSE r2 d1
        13 STATEMENT a 0 13
        13 STATEMENT b 0 13
    """
    yield "story-history-replay", """
        10 OPEN o1 acct
        11 DEPOSIT d1 acct 1000
        12 WITHDRAW w1 acct 100
        13 DEPOSIT d2 acct 10
        14 REVERSE r1 w1
        15 WITHDRAW w2 acct 500
        16 REVERSE r2 d2
        17 HOLD h1 acct 400
        18 CAPTURE k1 h1 400
        19 REVERSE r3 k1
        20 BALANCEAT acct 10
        20 BALANCEAT acct 11
        20 BALANCEAT acct 12
        20 BALANCEAT acct 13
        20 BALANCEAT acct 14
        20 BALANCEAT acct 15
        20 BALANCEAT acct 16
        20 BALANCEAT acct 17
        20 BALANCEAT acct 18
        20 BALANCEAT acct 19
        20 STATEMENT acct 12 16
        20 STATEMENT acct 14 19
        20 BALANCE acct
    """
    yield "story-id-collisions", """
        1 OPEN acct acct
        2 OPEN o2 acct
        3 DEPOSIT o2 acct 5
        4 DEPOSIT dep acct 5
        5 HOLD dep acct 5
        6 HOLD hold acct 3
        7 RELEASE dep hold
        8 RELEASE rel hold
        9 RELEASE rel hold
        10 REVERSE rev rel
        11 REVERSE rev2 dep
        12 REVERSE rev2 dep
        13 REVERSE rev3 o2
        14 REVERSE self self
        15 REVERSE self self
        16 RELEASE own own
        17 CAPTURE mine mine 1
        18 BALANCE acct
    """
    yield "story-chain-of-transfers", """
        1 OPEN o1 a
        1 OPEN o2 b
        1 OPEN o3 c
        2 DEPOSIT d1 a 100
        3 TRANSFER t1 a b 100
        4 TRANSFER t2 b c 100
        5 REVERSE r1 t1
        6 REVERSE r2 t2
        7 REVERSE r3 t1
        8 BALANCE a
        8 BALANCE b
        8 BALANCE c
        9 STATEMENT b 0 9
        9 BALANCEAT b 3
        9 BALANCEAT b 4
        9 BALANCEAT b 6
    """


def random_cases(rng: random.Random):
    for i in range(10):
        stream = Stream(rng, MIXED, accounts=rng.randint(1, 3), scale=50, steps=(0, 1, 1, 2), stray=0.08, edge=0.25)
        yield f"random-tiny-{i + 1}", stream.run(rng.randint(10, 40))
    for i in range(8):
        stream = Stream(rng, MIXED, accounts=rng.randint(3, 30), scale=rng.choice((100, 10_000)))
        yield f"random-medium-{i + 1}", stream.run(rng.randint(500, 3000))
    for i in range(4):
        stream = Stream(rng, ERRORS, accounts=rng.randint(2, 8), scale=200, stray=0.15, edge=0.3)
        yield f"random-error-heavy-{i + 1}", stream.run(rng.randint(500, 1500))
    for i in range(3):
        stream = Stream(rng, HOLDS, accounts=rng.randint(2, 6), scale=500, edge=0.25)
        yield f"random-hold-heavy-{i + 1}", stream.run(rng.randint(500, 1500))
    for i in range(3):
        stream = Stream(rng, REVERSALS, accounts=rng.randint(2, 6), scale=500, edge=0.25)
        yield f"random-reversal-heavy-{i + 1}", stream.run(rng.randint(500, 1500))


def large_cases(rng: random.Random):
    yield "large-mixed", Stream(rng, MIXED, accounts=60, scale=10**6, start=10**9).run(15_000)
    yield "large-deep-history", Stream(rng, HISTORY, accounts=3, scale=10**5, steps=(0, 1, 2, 3, 50)).run(15_000)
    yield "large-many-accounts", Stream(rng, dict(MIXED, open=30, close=12), accounts=500, scale=1000).run(15_000)
    extremes = Stream(rng, MIXED, accounts=5, scale=MAX_AMOUNT, start=MAX_T - 20_000, edge=0.3)
    yield "large-extremes", extremes.run(12_000)


def cases():
    write("examples", "holds-and-overdraft", script("""
        1 OPEN r1 alice
        1 OPEN r2 bob
        2 DEPOSIT d1 alice 1000
        3 HOLD h1 alice 700
        4 WITHDRAW w1 alice 400
        5 LIMIT l1 alice 100
        6 WITHDRAW w2 alice 400
        7 TRANSFER t1 alice bob 1
        8 CAPTURE c1 h1 500
        9 BALANCE alice
        9 STATEMENT alice 2 8
    """))
    write("examples", "retries-and-reversals", script("""
        1 OPEN o1 acct
        2 DEPOSIT d1 acct 50
        3 WITHDRAW w1 acct 80
        4 DEPOSIT d2 acct 30
        5 WITHDRAW w1 acct 80
        6 WITHDRAW w1 acct 70
        4 BALANCE acct
        7 REVERSE x1 d1
        8 REVERSE x2 x1
        9 REVERSE x3 d1
        10 BALANCEAT acct 3
        10 BALANCEAT acct 6
        10 BALANCE acct
    """))
    write("examples", "transfers-and-closing", script("""
        1 OPEN o1 a
        1 OPEN o2 b
        2 DEPOSIT d1 a 250
        3 TRANSFER t1 a b 100
        4 TRANSFER t2 a a 10
        5 TRANSFER t3 a c 10
        6 CLOSE c1 b
        7 WITHDRAW w1 b 100
        8 CLOSE c2 b
        9 REVERSE r1 t1
        10 DEPOSIT d2 b 1
        11 BALANCE b
        11 STATEMENT b 0 11
        11 STATEMENT b 11 3
    """))

    rng = random.Random(20261104)
    groups = [
        edge_cases(), timestamp_cases(), retry_cases(), account_cases(), funds_cases(), transfer_cases(),
        hold_cases(), reversal_cases(), query_cases(), story_cases(),
    ]
    named = [(name, script(block)) for group in groups for name, block in group]
    for number, (name, lines) in enumerate([*named, *random_cases(rng), *large_cases(rng)], start=1):
        write("hidden", f"{number:03d}-{name}", lines)


def workloads():
    rng = random.Random(20261105)
    write("workloads", "mixed", Stream(rng, MIXED, accounts=2000, scale=10**6, start=10**11).run(450_000), expect=False)
    history = Stream(rng, HISTORY, accounts=12, scale=10**6, steps=(0, 1, 2, 3, 50), stray=0.01, edge=0.05)
    write("workloads", "deep-history", history.run(400_000), expect=False)


def write_digests():
    """Records the workloads' SHA-256 so a regeneration is checked byte for byte."""
    lines = []
    for path in sorted((ROOT / "workloads").glob("*.in")):
        lines.append(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
    (ROOT / "workloads.sha256").write_text("".join(lines))


def main():
    commands = {"cases": cases, "workloads": lambda: (workloads(), write_digests())}
    if len(sys.argv) != 2 or sys.argv[1] not in commands:
        sys.exit(f"usage: {sys.argv[0]} {{{'|'.join(commands)}}}")
    commands[sys.argv[1]]()


if __name__ == "__main__":
    main()
