package main

import (
	"bufio"
	"io"
	"log"
	"os"
	"sort"
	"strconv"
	"strings"
)

type account struct {
	closed bool
	ledger int64
	held   int64
	limit  int64
	// Entries arrive in timestamp order. balances[i] and credits[i] are the ledger balance and the
	// credits posted before entry i, so each has one more element than times.
	times    []int64
	balances []int64
	credits  []int64
}

func newAccount() *account {
	return &account{balances: []int64{0}, credits: []int64{0}}
}

func (a *account) post(t, amount int64) {
	a.ledger += amount
	credits := a.credits[len(a.credits)-1] + max(amount, 0)
	a.times = append(a.times, t)
	a.balances = append(a.balances, a.ledger)
	a.credits = append(a.credits, credits)
}

// canSpend is the overdraft rule for taking amount out of the available balance.
func (a *account) canSpend(amount int64) bool {
	return a.ledger-a.held-amount >= -a.limit
}

// entriesBefore counts the entries whose timestamp is less than time.
func (a *account) entriesBefore(time int64) int {
	return sort.Search(len(a.times), func(i int) bool { return a.times[i] >= time })
}

type posting struct {
	account *account
	amount  int64
}

type hold struct {
	account *account
	amount  int64
	active  bool
}

func (h *hold) end() {
	h.active = false
	h.account.held -= h.amount
}

// movement is an accepted deposit, withdrawal, transfer or capture: the entries it posted.
type movement struct {
	postings []posting
	reversed bool
}

// request is a recorded request id. An accepted HOLD keeps its hold, and an accepted movement
// keeps its postings, for the commands that look them up later.
type request struct {
	text     string // the line after its timestamp, which a retry must repeat exactly
	result   string
	hold     *hold
	movement *movement
}

type ledger struct {
	clock    int64
	accounts map[string]*account
	requests map[string]*request
}

func (l *ledger) process(line string, out []byte) []byte {
	fields := strings.Split(line, " ")
	t := number(fields[0])
	if t < l.clock {
		return append(out, "ERR OUT_OF_ORDER"...)
	}
	l.clock = t
	switch fields[1] {
	case "BALANCE", "BALANCEAT", "STATEMENT":
		return l.query(fields, out)
	}
	id := fields[2]
	text := line[len(fields[0])+1:]
	if recorded, ok := l.requests[id]; ok {
		if recorded.text == text {
			return append(out, recorded.result...)
		}
		return append(out, "ERR ID_CONFLICT"...)
	}
	r := &request{text: text}
	r.result = l.execute(t, fields[1], fields[3:], r)
	l.requests[id] = r
	return append(out, r.result...)
}

// execute runs a command's checks and, when they pass, its effect, and returns its result line.
func (l *ledger) execute(t int64, name string, args []string, r *request) string {
	switch name {
	case "OPEN":
		if _, ok := l.accounts[args[0]]; ok {
			return "ERR ACCOUNT_EXISTS"
		}
		l.accounts[args[0]] = newAccount()
	case "DEPOSIT":
		a, rejected := l.openAccount(args[0])
		if rejected != "" {
			return rejected
		}
		r.movement = move(t, posting{a, number(args[1])})
	case "WITHDRAW":
		a, rejected := l.openAccount(args[0])
		if rejected != "" {
			return rejected
		}
		amount := number(args[1])
		if !a.canSpend(amount) {
			return "ERR INSUFFICIENT_FUNDS"
		}
		r.movement = move(t, posting{a, -amount})
	case "TRANSFER":
		if args[0] == args[1] {
			return "ERR SAME_ACCOUNT"
		}
		payer, payee := l.accounts[args[0]], l.accounts[args[1]]
		if payer == nil || payee == nil {
			return "ERR NO_ACCOUNT"
		}
		if payer.closed || payee.closed {
			return "ERR ACCOUNT_CLOSED"
		}
		amount := number(args[2])
		if !payer.canSpend(amount) {
			return "ERR INSUFFICIENT_FUNDS"
		}
		r.movement = move(t, posting{payer, -amount}, posting{payee, amount})
	case "HOLD":
		a, rejected := l.openAccount(args[0])
		if rejected != "" {
			return rejected
		}
		amount := number(args[1])
		if !a.canSpend(amount) {
			return "ERR INSUFFICIENT_FUNDS"
		}
		a.held += amount
		r.hold = &hold{account: a, amount: amount, active: true}
	case "RELEASE":
		h, rejected := l.activeHold(args[0])
		if rejected != "" {
			return rejected
		}
		h.end()
	case "CAPTURE":
		h, rejected := l.activeHold(args[0])
		if rejected != "" {
			return rejected
		}
		amount := number(args[1])
		if amount > h.amount {
			return "ERR EXCEEDS_HOLD"
		}
		h.end()
		r.movement = move(t, posting{h.account, -amount})
	case "REVERSE":
		return l.reverse(t, args[0])
	case "LIMIT":
		a, rejected := l.openAccount(args[0])
		if rejected != "" {
			return rejected
		}
		limit := number(args[1])
		if a.ledger-a.held < -limit {
			return "ERR LIMIT_TOO_LOW"
		}
		a.limit = limit
	case "CLOSE":
		a, rejected := l.openAccount(args[0])
		if rejected != "" {
			return rejected
		}
		if a.held != 0 {
			return "ERR HOLDS_ACTIVE"
		}
		if a.ledger != 0 {
			return "ERR NONZERO_BALANCE"
		}
		a.closed = true
	default:
		log.Fatalf("unknown command %q", name)
	}
	return "OK"
}

func (l *ledger) reverse(t int64, target string) string {
	recorded, ok := l.requests[target]
	if !ok || recorded.result != "OK" {
		return "ERR NO_TRANSACTION"
	}
	m := recorded.movement
	if m == nil {
		return "ERR NOT_REVERSIBLE"
	}
	if m.reversed {
		return "ERR ALREADY_REVERSED"
	}
	for _, p := range m.postings {
		if p.account.closed {
			return "ERR ACCOUNT_CLOSED"
		}
	}
	for _, p := range m.postings {
		if p.amount > 0 && !p.account.canSpend(p.amount) {
			return "ERR INSUFFICIENT_FUNDS"
		}
	}
	m.reversed = true
	for _, p := range m.postings {
		p.account.post(t, -p.amount)
	}
	return "OK"
}

func (l *ledger) query(fields []string, out []byte) []byte {
	if fields[1] == "STATEMENT" && number(fields[3]) > number(fields[4]) {
		return append(out, "ERR BAD_RANGE"...)
	}
	a, ok := l.accounts[fields[2]]
	if !ok {
		return append(out, "ERR NO_ACCOUNT"...)
	}
	switch fields[1] {
	case "BALANCE":
		out = strconv.AppendInt(out, a.ledger, 10)
		out = append(out, ' ')
		return strconv.AppendInt(out, a.ledger-a.held, 10)
	case "BALANCEAT":
		return strconv.AppendInt(out, a.balances[a.entriesBefore(number(fields[3])+1)], 10)
	}
	lo, hi := a.entriesBefore(number(fields[3])), a.entriesBefore(number(fields[4])+1)
	opening, closing := a.balances[lo], a.balances[hi]
	credits := a.credits[hi] - a.credits[lo]
	debits := credits - (closing - opening)
	for _, value := range []int64{opening, credits, debits, closing, int64(hi - lo)} {
		out = strconv.AppendInt(out, value, 10)
		out = append(out, ' ')
	}
	return out[:len(out)-1]
}

func (l *ledger) openAccount(name string) (*account, string) {
	a, ok := l.accounts[name]
	if !ok {
		return nil, "ERR NO_ACCOUNT"
	}
	if a.closed {
		return nil, "ERR ACCOUNT_CLOSED"
	}
	return a, ""
}

func (l *ledger) activeHold(id string) (*hold, string) {
	recorded, ok := l.requests[id]
	if !ok || recorded.hold == nil {
		return nil, "ERR NO_HOLD"
	}
	if !recorded.hold.active {
		return nil, "ERR HOLD_INACTIVE"
	}
	return recorded.hold, ""
}

func move(t int64, postings ...posting) *movement {
	for _, p := range postings {
		p.account.post(t, p.amount)
	}
	return &movement{postings: postings}
}

func number(text string) int64 {
	value, err := strconv.ParseInt(text, 10, 64)
	if err != nil {
		log.Fatal(err)
	}
	return value
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	lines := strings.Split(string(input), "\n")
	n := int(number(lines[0]))
	l := &ledger{accounts: make(map[string]*account), requests: make(map[string]*request)}
	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	var line []byte
	for _, text := range lines[1 : n+1] {
		line = append(l.process(text, line[:0]), '\n')
		out.Write(line)
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
