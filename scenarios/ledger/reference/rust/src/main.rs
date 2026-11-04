use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, Read, Write};

#[derive(Clone, Copy)]
enum Error {
    OutOfOrder,
    IdConflict,
    AccountExists,
    NoAccount,
    AccountClosed,
    SameAccount,
    InsufficientFunds,
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
}

impl Error {
    fn line(self) -> &'static str {
        match self {
            Error::OutOfOrder => "ERR OUT_OF_ORDER",
            Error::IdConflict => "ERR ID_CONFLICT",
            Error::AccountExists => "ERR ACCOUNT_EXISTS",
            Error::NoAccount => "ERR NO_ACCOUNT",
            Error::AccountClosed => "ERR ACCOUNT_CLOSED",
            Error::SameAccount => "ERR SAME_ACCOUNT",
            Error::InsufficientFunds => "ERR INSUFFICIENT_FUNDS",
            Error::NoHold => "ERR NO_HOLD",
            Error::HoldInactive => "ERR HOLD_INACTIVE",
            Error::ExceedsHold => "ERR EXCEEDS_HOLD",
            Error::NoTransaction => "ERR NO_TRANSACTION",
            Error::NotReversible => "ERR NOT_REVERSIBLE",
            Error::AlreadyReversed => "ERR ALREADY_REVERSED",
            Error::LimitTooLow => "ERR LIMIT_TOO_LOW",
            Error::HoldsActive => "ERR HOLDS_ACTIVE",
            Error::NonzeroBalance => "ERR NONZERO_BALANCE",
            Error::BadRange => "ERR BAD_RANGE",
        }
    }
}

struct Account {
    closed: bool,
    ledger: i64,
    held: i64,
    limit: i64,
    // Entries arrive in timestamp order. balances[i] and credits[i] are the ledger balance and the
    // credits posted before entry i, so each has one more element than times.
    times: Vec<i64>,
    balances: Vec<i64>,
    credits: Vec<i64>,
}

impl Account {
    fn new() -> Self {
        Account {
            closed: false,
            ledger: 0,
            held: 0,
            limit: 0,
            times: Vec::new(),
            balances: vec![0],
            credits: vec![0],
        }
    }

    fn post(&mut self, t: i64, amount: i64) {
        self.ledger += amount;
        let credits = self.credits[self.credits.len() - 1] + amount.max(0);
        self.times.push(t);
        self.balances.push(self.ledger);
        self.credits.push(credits);
    }

    /// The overdraft rule for taking `amount` out of the available balance.
    fn check_spend(&self, amount: i64) -> Result<(), Error> {
        if self.ledger - self.held - amount < -self.limit {
            Err(Error::InsufficientFunds)
        } else {
            Ok(())
        }
    }

    /// The number of entries whose timestamp is at most `time`.
    fn entries_until(&self, time: i64) -> usize {
        self.times.partition_point(|&t| t <= time)
    }
}

/// An entry an accepted command posted: the account's index and the signed amount.
type Posting = (usize, i64);

/// What an accepted command leaves for later `RELEASE`, `CAPTURE` and `REVERSE` lookups.
enum Effect {
    Other,
    Hold {
        account: usize,
        amount: i64,
        active: bool,
    },
    Movement {
        first: Posting,
        second: Option<Posting>,
        reversed: bool,
    },
}

struct Request<'a> {
    /// The line after its timestamp, which a retry must repeat exactly.
    text: &'a str,
    outcome: Result<Effect, Error>,
}

#[derive(Default)]
struct Ledger<'a> {
    clock: i64,
    names: HashMap<&'a str, usize>,
    accounts: Vec<Account>,
    requests: HashMap<&'a str, Request<'a>>,
}

impl<'a> Ledger<'a> {
    fn process(&mut self, line: &'a str, out: &mut String) {
        let mut fields = line.split(' ');
        let stamp = fields.next().unwrap();
        let name = fields.next().unwrap();
        let mut args = [""; 4];
        for (slot, field) in args.iter_mut().zip(fields) {
            *slot = field;
        }
        let t: i64 = stamp.parse().unwrap();
        if t < self.clock {
            out.push_str(Error::OutOfOrder.line());
        } else {
            self.clock = t;
            match name {
                "BALANCE" | "BALANCEAT" | "STATEMENT" => {
                    if let Err(error) = self.query(name, &args, out) {
                        out.push_str(error.line());
                    }
                }
                _ => {
                    let text = &line[stamp.len() + 1..];
                    out.push_str(self.command(t, name, text, &args));
                }
            }
        }
        out.push('\n');
    }

    fn command(&mut self, t: i64, name: &str, text: &'a str, args: &[&'a str; 4]) -> &'static str {
        let line = |outcome: &Result<Effect, Error>| match outcome {
            Ok(_) => "OK",
            Err(error) => error.line(),
        };
        if let Some(request) = self.requests.get(args[0]) {
            return if request.text == text {
                line(&request.outcome)
            } else {
                Error::IdConflict.line()
            };
        }
        let outcome = self.apply(t, name, args);
        let result = line(&outcome);
        self.requests.insert(args[0], Request { text, outcome });
        result
    }

    fn apply(&mut self, t: i64, name: &str, args: &[&'a str; 4]) -> Result<Effect, Error> {
        match name {
            "OPEN" => {
                if self.names.contains_key(args[1]) {
                    return Err(Error::AccountExists);
                }
                self.names.insert(args[1], self.accounts.len());
                self.accounts.push(Account::new());
                Ok(Effect::Other)
            }
            "DEPOSIT" => {
                let account = self.open_account(args[1])?;
                Ok(self.post(t, (account, number(args[2])), None))
            }
            "WITHDRAW" => {
                let account = self.open_account(args[1])?;
                let amount = number(args[2]);
                self.accounts[account].check_spend(amount)?;
                Ok(self.post(t, (account, -amount), None))
            }
            "TRANSFER" => {
                if args[1] == args[2] {
                    return Err(Error::SameAccount);
                }
                let (Some(&payer), Some(&payee)) = (self.names.get(args[1]), self.names.get(args[2]))
                else {
                    return Err(Error::NoAccount);
                };
                if self.accounts[payer].closed || self.accounts[payee].closed {
                    return Err(Error::AccountClosed);
                }
                let amount = number(args[3]);
                self.accounts[payer].check_spend(amount)?;
                Ok(self.post(t, (payer, -amount), Some((payee, amount))))
            }
            "HOLD" => {
                let account = self.open_account(args[1])?;
                let amount = number(args[2]);
                self.accounts[account].check_spend(amount)?;
                self.accounts[account].held += amount;
                Ok(Effect::Hold { account, amount, active: true })
            }
            "RELEASE" => {
                let (active, account, amount) = active_hold(&mut self.requests, args[1])?;
                *active = false;
                self.accounts[account].held -= amount;
                Ok(Effect::Other)
            }
            "CAPTURE" => {
                let (active, account, held) = active_hold(&mut self.requests, args[1])?;
                let amount = number(args[2]);
                if amount > held {
                    return Err(Error::ExceedsHold);
                }
                *active = false;
                self.accounts[account].held -= held;
                Ok(self.post(t, (account, -amount), None))
            }
            "REVERSE" => {
                let Some(Request { outcome: Ok(effect), .. }) = self.requests.get_mut(args[1]) else {
                    return Err(Error::NoTransaction);
                };
                let Effect::Movement { first, second, reversed } = effect else {
                    return Err(Error::NotReversible);
                };
                if *reversed {
                    return Err(Error::AlreadyReversed);
                }
                let postings = [Some(*first), *second];
                let accounts = &mut self.accounts;
                if postings.iter().flatten().any(|&(account, _)| accounts[account].closed) {
                    return Err(Error::AccountClosed);
                }
                for &(account, amount) in postings.iter().flatten() {
                    if amount > 0 {
                        accounts[account].check_spend(amount)?;
                    }
                }
                *reversed = true;
                for &(account, amount) in postings.iter().flatten() {
                    accounts[account].post(t, -amount);
                }
                Ok(Effect::Other)
            }
            "LIMIT" => {
                let index = self.open_account(args[1])?;
                let account = &mut self.accounts[index];
                let limit = number(args[2]);
                if account.ledger - account.held < -limit {
                    return Err(Error::LimitTooLow);
                }
                account.limit = limit;
                Ok(Effect::Other)
            }
            "CLOSE" => {
                let index = self.open_account(args[1])?;
                let account = &mut self.accounts[index];
                if account.held != 0 {
                    return Err(Error::HoldsActive);
                }
                if account.ledger != 0 {
                    return Err(Error::NonzeroBalance);
                }
                account.closed = true;
                Ok(Effect::Other)
            }
            _ => unreachable!("unknown command {name}"),
        }
    }

    fn query(&self, name: &str, args: &[&str; 4], out: &mut String) -> Result<(), Error> {
        match name {
            "BALANCE" => {
                let account = self.account(args[0])?;
                write!(out, "{} {}", account.ledger, account.ledger - account.held).unwrap();
            }
            "BALANCEAT" => {
                let account = self.account(args[0])?;
                let balance = account.balances[account.entries_until(number(args[1]))];
                write!(out, "{balance}").unwrap();
            }
            _ => {
                let (start, end) = (number(args[1]), number(args[2]));
                if start > end {
                    return Err(Error::BadRange);
                }
                let account = self.account(args[0])?;
                let lo = account.times.partition_point(|&t| t < start);
                let hi = account.entries_until(end);
                let (opening, closing) = (account.balances[lo], account.balances[hi]);
                let credits = account.credits[hi] - account.credits[lo];
                let debits = credits - (closing - opening);
                let count = hi - lo;
                write!(out, "{opening} {credits} {debits} {closing} {count}").unwrap();
            }
        }
        Ok(())
    }

    fn account(&self, name: &str) -> Result<&Account, Error> {
        self.names
            .get(name)
            .map(|&index| &self.accounts[index])
            .ok_or(Error::NoAccount)
    }

    fn open_account(&self, name: &str) -> Result<usize, Error> {
        let &index = self.names.get(name).ok_or(Error::NoAccount)?;
        if self.accounts[index].closed {
            return Err(Error::AccountClosed);
        }
        Ok(index)
    }

    fn post(&mut self, t: i64, first: Posting, second: Option<Posting>) -> Effect {
        for (account, amount) in [Some(first), second].into_iter().flatten() {
            self.accounts[account].post(t, amount);
        }
        Effect::Movement { first, second, reversed: false }
    }
}

/// The hold named `id` if it is active: its flag, its account and its amount.
fn active_hold<'r>(
    requests: &'r mut HashMap<&str, Request<'_>>,
    id: &str,
) -> Result<(&'r mut bool, usize, i64), Error> {
    match requests.get_mut(id) {
        Some(Request { outcome: Ok(Effect::Hold { account, amount, active }), .. }) => {
            if *active {
                Ok((active, *account, *amount))
            } else {
                Err(Error::HoldInactive)
            }
        }
        _ => Err(Error::NoHold),
    }
}

fn number(text: &str) -> i64 {
    text.parse().unwrap()
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut lines = input.split('\n');
    let n: usize = lines.next().unwrap().parse().unwrap();
    let mut ledger = Ledger::default();
    let mut out = String::with_capacity(16 * n);
    for line in lines.take(n) {
        ledger.process(line, &mut out);
    }
    io::stdout().lock().write_all(out.as_bytes()).unwrap();
}
