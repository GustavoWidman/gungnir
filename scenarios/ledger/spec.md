# Ledger

Run an event-sourced bank ledger. Read a stream of timestamped commands that open accounts, move money, place holds, reverse earlier transactions and query balances, and print one result line per command.

Amounts are whole numbers of cents. Every change to a balance is recorded as an entry that is never changed or removed, so the ledger can also report what an account's balance was at any earlier time.

## Input

The input is ASCII text read from standard input.

- The first line holds one integer `N`, the number of commands.
- Each of the next `N` lines holds one command: its timestamp `T`, its name, and its arguments, separated by single spaces.

Every line, including the last, ends with a line feed (byte 10). No line has leading or trailing spaces.

There are thirteen commands:

| Line | Meaning |
|---|---|
| `T OPEN id account` | Open an account. |
| `T DEPOSIT id account amount` | Add money to an account. |
| `T WITHDRAW id account amount` | Take money out of an account. |
| `T TRANSFER id from to amount` | Move money from account `from` to account `to`. |
| `T HOLD id account amount` | Reserve money in an account without moving it. |
| `T RELEASE id hold` | End a hold without moving money. |
| `T CAPTURE id hold amount` | End a hold and take up to its amount out of its account. |
| `T REVERSE id target` | Undo an earlier deposit, withdrawal, transfer or capture. |
| `T LIMIT id account limit` | Set an account's overdraft limit. |
| `T CLOSE id account` | Close an account. |
| `T BALANCE account` | Print an account's current balances. |
| `T BALANCEAT account time` | Print an account's ledger balance at a given time. |
| `T STATEMENT account start end` | Summarize an account's entries in a time range. |

The first ten commands change the ledger and carry a request id, `id`. The last three are queries and carry none. In `RELEASE` and `CAPTURE`, `hold` is the request id of the `HOLD` that placed the hold. In `REVERSE`, `target` is the request id of the command to undo.

`T`, `amount`, `limit`, `time`, `start` and `end` are integers written in decimal with no sign and no leading zeros. Account names and request ids are 1 to 24 characters long, and each character is a lowercase ASCII letter (`a` to `z`), a digit, `-` or `_`. Account names and request ids are separate: an account and a request id may share a name and have nothing to do with each other.

The input is always well formed. Every line has one of the forms above, with every value within the constraints below. Everything else that can go wrong, such as naming an account that does not exist, is reported in the output.

## Accounts, entries and holds

An account has a name, a status (open or closed), a ledger balance, an available balance, an overdraft limit, and zero or more active holds. `OPEN` creates an open account with no entries, no holds and an overdraft limit of 0. An account is never removed: a closed account stays closed, can still be queried, and its name can never be opened again.

An entry is a signed amount posted to one account at one timestamp, the timestamp of the command that posts it. A positive entry is a credit and a negative entry is a debit. Entries are never changed or removed. An account's ledger balance is the sum of all its entries. Its ledger balance at time `X` is the sum of its entries whose timestamp is at most `X`.

A hold reserves part of an account's money without posting an entry. It is active from the `HOLD` that places it until a `RELEASE` or a `CAPTURE` ends it, and an ended hold never becomes active again. An account's available balance is its ledger balance minus the sum of the amounts of its active holds.

**The overdraft rule.** No command may leave an account's available balance below the negative of its overdraft limit. With a limit of 0 the available balance may go down to 0, and with a limit of 500 it may go down to -500. The commands that could break the rule check it, as their sections say, and are rejected when they would break it.

## Processing a line

The lines are processed one at a time, in input order, and each prints exactly one result line. A line goes through these steps. The first step that prints ends the processing of the line.

1. **Timestamp.** If `T` is smaller than the timestamp of any earlier line, print `ERR OUT_OF_ORDER`. The line has no other effect, and its request id is not recorded. Equal timestamps are allowed.
2. **Request id.** This step applies only to the ten commands that carry a request id. If the request id was recorded by an earlier line, the line changes nothing:
   - If this line's text after its timestamp is identical to the recorded line's text after its timestamp, print the result line recorded for that request id, unchanged. This is a retry, and it prints the original result even when running the command again would now give a different one.
   - Otherwise print `ERR ID_CONFLICT`.
3. **Checks.** Run the command's checks in the order its section lists them. The first check that fails prints its error, and the command changes nothing.
4. **Effect.** Apply the command's effect and print its result. Every one of the ten commands that change the ledger prints `OK`.

A command with a request id that reaches step 3 records that id, together with its text after the timestamp and the result line it printed, whether that is `OK` or an error. The id is recorded after the checks, so a command's own request id is never found by its own checks. A command that reaches step 4 is accepted. Every other command is rejected, and only accepted commands are found by the `hold` and `target` lookups below.

## Commands

Each section lists a command's checks in order, with the error line each check prints when it fails, and then the command's effect.

### OPEN id account

1. An account named `account` already exists, open or closed: `ERR ACCOUNT_EXISTS`.

Creates the account.

### DEPOSIT id account amount

1. No account named `account` exists: `ERR NO_ACCOUNT`.
2. The account is closed: `ERR ACCOUNT_CLOSED`.

Posts an entry of `+amount` to the account.

### WITHDRAW id account amount

1. No account named `account` exists: `ERR NO_ACCOUNT`.
2. The account is closed: `ERR ACCOUNT_CLOSED`.
3. The account's available balance minus `amount` is less than the negative of its overdraft limit: `ERR INSUFFICIENT_FUNDS`.

Posts an entry of `-amount` to the account.

### TRANSFER id from to amount

1. `from` and `to` are the same name: `ERR SAME_ACCOUNT`.
2. No account named `from` exists, or no account named `to` exists: `ERR NO_ACCOUNT`.
3. Account `from` is closed, or account `to` is closed: `ERR ACCOUNT_CLOSED`.
4. The available balance of `from` minus `amount` is less than the negative of the overdraft limit of `from`: `ERR INSUFFICIENT_FUNDS`.

Posts an entry of `-amount` to `from` and an entry of `+amount` to `to`.

### HOLD id account amount

1. No account named `account` exists: `ERR NO_ACCOUNT`.
2. The account is closed: `ERR ACCOUNT_CLOSED`.
3. The account's available balance minus `amount` is less than the negative of its overdraft limit: `ERR INSUFFICIENT_FUNDS`.

Places an active hold of `amount` on the account, named by this command's request id `id`. No entry is posted, so the ledger balance stays the same and the available balance falls by `amount`.

### RELEASE id hold

1. `hold` is not the request id of an accepted `HOLD`: `ERR NO_HOLD`.
2. The hold is not active, because a `RELEASE` or `CAPTURE` already ended it: `ERR HOLD_INACTIVE`.

Ends the hold. No entry is posted, so the ledger balance stays the same and the available balance rises by the hold's amount.

### CAPTURE id hold amount

1. `hold` is not the request id of an accepted `HOLD`: `ERR NO_HOLD`.
2. The hold is not active, because a `RELEASE` or `CAPTURE` already ended it: `ERR HOLD_INACTIVE`.
3. `amount` is greater than the hold's amount: `ERR EXCEEDS_HOLD`.

Ends the whole hold and posts an entry of `-amount` to the hold's account. When `amount` is less than the hold's amount, the rest is simply no longer held. A capture never lowers the available balance, so it has no overdraft check.

### REVERSE id target

1. `target` is not the request id of an accepted command: `ERR NO_TRANSACTION`.
2. The target is not a `DEPOSIT`, `WITHDRAW`, `TRANSFER` or `CAPTURE`: `ERR NOT_REVERSIBLE`. In particular, a reversal cannot itself be reversed.
3. An accepted `REVERSE` has already reversed the target: `ERR ALREADY_REVERSED`.
4. An account that the reversal would post an entry to is closed: `ERR ACCOUNT_CLOSED`.
5. The account that the reversal would debit would break the overdraft rule: its available balance minus the debited amount is less than the negative of its overdraft limit. The error is `ERR INSUFFICIENT_FUNDS`.

For every entry the target posted, posts an entry with the opposite sign and the same amount to the same account, at this command's timestamp `T`. The target's own entries stay where they are, so the ledger balance at a time between the target and its reversal still includes the target. In detail:

- Reversing a `DEPOSIT` posts `-amount` to its account. Step 5 checks that account.
- Reversing a `WITHDRAW` posts `+amount` to its account. Step 5 has nothing to check.
- Reversing a `TRANSFER` posts `+amount` to its `from` account and `-amount` to its `to` account. Step 4 checks both accounts, and step 5 checks the `to` account.
- Reversing a `CAPTURE` posts `+amount`, the captured amount, to the hold's account. Step 5 has nothing to check. The hold stays ended.

### LIMIT id account limit

1. No account named `account` exists: `ERR NO_ACCOUNT`.
2. The account is closed: `ERR ACCOUNT_CLOSED`.
3. The account's available balance is less than `-limit`: `ERR LIMIT_TOO_LOW`.

Sets the account's overdraft limit to `limit`. The limit may be raised or lowered.

### CLOSE id account

1. No account named `account` exists: `ERR NO_ACCOUNT`.
2. The account is already closed: `ERR ACCOUNT_CLOSED`.
3. The account has an active hold: `ERR HOLDS_ACTIVE`.
4. The account's ledger balance is not 0: `ERR NONZERO_BALANCE`.

Closes the account.

### BALANCE account

1. No account named `account` exists: `ERR NO_ACCOUNT`.

Prints the account's ledger balance, one space, and its available balance.

### BALANCEAT account time

1. No account named `account` exists: `ERR NO_ACCOUNT`.

Prints the account's ledger balance at time `time`: the sum of its entries, posted by earlier lines, whose timestamp is at most `time`. A `time` before the account's first entry gives 0, and a `time` at or after the timestamp of its last entry gives its current ledger balance.

### STATEMENT account start end

1. `start` is greater than `end`: `ERR BAD_RANGE`.
2. No account named `account` exists: `ERR NO_ACCOUNT`.

Considers the account's entries whose timestamp `t` satisfies `start <= t <= end`, which are the entries in the range, and prints five integers separated by single spaces:

1. The opening balance: the sum of the account's entries whose timestamp is less than `start`.
2. The credits: the sum of the positive entries in the range.
3. The debits: the sum of the absolute values of the negative entries in the range.
4. The closing balance: the account's ledger balance at time `end`. It equals the opening balance plus the credits minus the debits.
5. The number of entries in the range.

## Output

Write exactly `N` lines, one per command, in input order. A line is `OK`, an error such as `ERR NO_ACCOUNT`, or the integers a query prints, and it ends with a line feed. Integers are written in decimal with no leading zeros, and a negative integer has a leading `-`. When `N` is 0, write nothing.

## Constraints

- `0 <= N <= 2000000`
- `0 <= T <= 1000000000000`, and `time`, `start` and `end` are in the same range.
- `1 <= amount <= 1000000000`
- `0 <= limit <= 1000000000`
- Every balance, sum and count the ledger keeps or prints is at most `2000000000000000` in absolute value.

Some inputs have hundreds of thousands of commands, with more than a hundred thousand `BALANCEAT` and `STATEMENT` queries on accounts that each have tens of thousands of entries. A solution that adds up an account's entries again for every query is far too slow for them.

## Examples

Input:

```text
11
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
```

Output:

```text
OK
OK
OK
OK
ERR INSUFFICIENT_FUNDS
OK
OK
ERR INSUFFICIENT_FUNDS
OK
100 100
0 1000 900 100 3
```

The hold leaves alice 300 of her 1000 available, too little for `w1`. With an overdraft limit of 100, `w2` takes her available balance to exactly -100, and `t1` would take it to -101. Capturing 500 of the 700 held posts a debit of 500 and ends the hold, so her available balance is her ledger balance again. The statement counts the deposit, `w2` and the capture. The hold and the rejected commands posted no entries.

Input:

```text
13
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
```

Output:

```text
OK
OK
ERR INSUFFICIENT_FUNDS
OK
ERR INSUFFICIENT_FUNDS
ERR ID_CONFLICT
ERR OUT_OF_ORDER
OK
ERR NOT_REVERSIBLE
ERR ALREADY_REVERSED
50
80
30 30
```

The retry of `w1` at time 5 repeats the recorded error even though the account now holds 80. Changing its amount is a conflict. The `BALANCE` at time 4 comes after a line at time 6, so it is out of order. Reversing `d1` posts a debit of 50 at time 7, so the balance at time 6 still includes both deposits.
