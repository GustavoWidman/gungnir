import sys
from bisect import bisect_left, bisect_right


class Rejected(Exception):
    """A failed check. Its argument is the error code."""


class Account:
    __slots__ = ("closed", "ledger", "held", "limit", "times", "balances", "credits")

    def __init__(self) -> None:
        self.closed = False
        self.ledger = 0
        self.held = 0
        self.limit = 0
        # Entries arrive in timestamp order. Before entry i, at times[i], the ledger balance was
        # balances[i] and the credits so far summed to credits[i]; the last element is the total.
        self.times: list[int] = []
        self.balances = [0]
        self.credits = [0]

    def post(self, t: int, amount: int) -> None:
        self.ledger += amount
        self.times.append(t)
        self.balances.append(self.ledger)
        self.credits.append(self.credits[-1] + amount if amount > 0 else self.credits[-1])

    def check_spend(self, amount: int) -> None:
        """The overdraft rule for taking `amount` out of the available balance."""
        if self.ledger - self.held - amount < -self.limit:
            raise Rejected("INSUFFICIENT_FUNDS")


class Hold:
    __slots__ = ("account", "amount", "active")

    def __init__(self, account: Account, amount: int) -> None:
        self.account = account
        self.amount = amount
        self.active = True


class Movement:
    """An accepted deposit, withdrawal, transfer or capture: the entries it posted."""

    __slots__ = ("postings", "reversed")

    def __init__(self, postings: tuple[tuple[Account, int], ...]) -> None:
        self.postings = postings
        self.reversed = False


class Ledger:
    def __init__(self) -> None:
        self.clock = 0
        self.accounts: dict[str, Account] = {}
        # Every recorded request id: its line after the timestamp, and the result it printed.
        self.requests: dict[str, tuple[str, str]] = {}
        self.holds: dict[str, Hold] = {}
        self.movements: dict[str, Movement] = {}
        self.commands = {
            "OPEN": self.open,
            "DEPOSIT": self.deposit,
            "WITHDRAW": self.withdraw,
            "TRANSFER": self.transfer,
            "HOLD": self.hold,
            "RELEASE": self.release,
            "CAPTURE": self.capture,
            "REVERSE": self.reverse,
            "LIMIT": self.limit,
            "CLOSE": self.close,
        }
        self.queries = {
            "BALANCE": self.balance,
            "BALANCEAT": self.balance_at,
            "STATEMENT": self.statement,
        }

    def process(self, line: str) -> str:
        stamp, name, *args = line.split(" ")
        t = int(stamp)
        if t < self.clock:
            return "ERR OUT_OF_ORDER"
        self.clock = t
        query = self.queries.get(name)
        if query is not None:
            try:
                return query(*args)
            except Rejected as rejection:
                return f"ERR {rejection.args[0]}"
        rid = args[0]
        text = line[len(stamp) + 1 :]
        recorded = self.requests.get(rid)
        if recorded is not None:
            return recorded[1] if recorded[0] == text else "ERR ID_CONFLICT"
        try:
            self.commands[name](t, rid, *args[1:])
            result = "OK"
        except Rejected as rejection:
            result = f"ERR {rejection.args[0]}"
        self.requests[rid] = (text, result)
        return result

    def account(self, name: str) -> Account:
        account = self.accounts.get(name)
        if account is None:
            raise Rejected("NO_ACCOUNT")
        return account

    def open_account(self, name: str) -> Account:
        account = self.account(name)
        if account.closed:
            raise Rejected("ACCOUNT_CLOSED")
        return account

    def active_hold(self, hold_id: str) -> Hold:
        hold = self.holds.get(hold_id)
        if hold is None:
            raise Rejected("NO_HOLD")
        if not hold.active:
            raise Rejected("HOLD_INACTIVE")
        return hold

    def move(self, rid: str, t: int, postings: tuple[tuple[Account, int], ...]) -> None:
        for account, amount in postings:
            account.post(t, amount)
        self.movements[rid] = Movement(postings)

    def open(self, t: int, rid: str, name: str) -> None:
        if name in self.accounts:
            raise Rejected("ACCOUNT_EXISTS")
        self.accounts[name] = Account()

    def deposit(self, t: int, rid: str, name: str, amount: str) -> None:
        account = self.open_account(name)
        self.move(rid, t, ((account, int(amount)),))

    def withdraw(self, t: int, rid: str, name: str, amount: str) -> None:
        account = self.open_account(name)
        value = int(amount)
        account.check_spend(value)
        self.move(rid, t, ((account, -value),))

    def transfer(self, t: int, rid: str, source: str, target: str, amount: str) -> None:
        if source == target:
            raise Rejected("SAME_ACCOUNT")
        payer = self.accounts.get(source)
        payee = self.accounts.get(target)
        if payer is None or payee is None:
            raise Rejected("NO_ACCOUNT")
        if payer.closed or payee.closed:
            raise Rejected("ACCOUNT_CLOSED")
        value = int(amount)
        payer.check_spend(value)
        self.move(rid, t, ((payer, -value), (payee, value)))

    def hold(self, t: int, rid: str, name: str, amount: str) -> None:
        account = self.open_account(name)
        value = int(amount)
        account.check_spend(value)
        account.held += value
        self.holds[rid] = Hold(account, value)

    def release(self, t: int, rid: str, hold_id: str) -> None:
        hold = self.active_hold(hold_id)
        hold.active = False
        hold.account.held -= hold.amount

    def capture(self, t: int, rid: str, hold_id: str, amount: str) -> None:
        hold = self.active_hold(hold_id)
        value = int(amount)
        if value > hold.amount:
            raise Rejected("EXCEEDS_HOLD")
        hold.active = False
        hold.account.held -= hold.amount
        self.move(rid, t, ((hold.account, -value),))

    def reverse(self, t: int, rid: str, target: str) -> None:
        recorded = self.requests.get(target)
        if recorded is None or recorded[1] != "OK":
            raise Rejected("NO_TRANSACTION")
        movement = self.movements.get(target)
        if movement is None:
            raise Rejected("NOT_REVERSIBLE")
        if movement.reversed:
            raise Rejected("ALREADY_REVERSED")
        if any(account.closed for account, _ in movement.postings):
            raise Rejected("ACCOUNT_CLOSED")
        for account, amount in movement.postings:
            if amount > 0:
                account.check_spend(amount)
        movement.reversed = True
        for account, amount in movement.postings:
            account.post(t, -amount)

    def limit(self, t: int, rid: str, name: str, limit: str) -> None:
        account = self.open_account(name)
        value = int(limit)
        if account.ledger - account.held < -value:
            raise Rejected("LIMIT_TOO_LOW")
        account.limit = value

    def close(self, t: int, rid: str, name: str) -> None:
        account = self.open_account(name)
        if account.held:
            raise Rejected("HOLDS_ACTIVE")
        if account.ledger:
            raise Rejected("NONZERO_BALANCE")
        account.closed = True

    def balance(self, name: str) -> str:
        account = self.account(name)
        return f"{account.ledger} {account.ledger - account.held}"

    def balance_at(self, name: str, time: str) -> str:
        account = self.account(name)
        return str(account.balances[bisect_right(account.times, int(time))])

    def statement(self, name: str, start: str, end: str) -> str:
        first, last = int(start), int(end)
        if first > last:
            raise Rejected("BAD_RANGE")
        account = self.account(name)
        lo = bisect_left(account.times, first)
        hi = bisect_right(account.times, last)
        opening, closing = account.balances[lo], account.balances[hi]
        credits = account.credits[hi] - account.credits[lo]
        debits = credits - (closing - opening)
        return f"{opening} {credits} {debits} {closing} {hi - lo}"


def main() -> None:
    lines = sys.stdin.buffer.read().decode("ascii").split("\n")
    n = int(lines[0])
    ledger = Ledger()
    results = [ledger.process(line) for line in lines[1 : n + 1]]
    if results:
        sys.stdout.write("\n".join(results))
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
