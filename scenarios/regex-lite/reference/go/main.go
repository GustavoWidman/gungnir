package main

import (
	"bufio"
	"bytes"
	"encoding/binary"
	"errors"
	"io"
	"log"
	"math/bits"
	"os"
	"slices"
	"strconv"
	"strings"
)

const escapable = "\\.*+?|()[]^$-"

// A text line has at most 100000 characters, so one line adds at most about 300000 DFA states.
const stateLimit = 10000

type kind uint8

const (
	char kind = iota
	epsilon
	begin
	end
	match
)

var errInvalid = errors.New("invalid pattern")

type node struct {
	kind  kind
	chars [2]uint64
	out   []int
}

type fragment struct{ entry, exit int }

// parser builds a Thompson NFA while parsing. Every fragment is an (entry, exit) pair whose exit
// is an epsilon node that later gets the edges to whatever follows.
type parser struct {
	pattern []byte
	pos     int
	nodes   []node
}

func (p *parser) node(k kind, chars [2]uint64) int {
	p.nodes = append(p.nodes, node{kind: k, chars: chars})
	return len(p.nodes) - 1
}

func (p *parser) link(from, to int) {
	p.nodes[from].out = append(p.nodes[from].out, to)
}

func (p *parser) peek() int {
	if p.pos < len(p.pattern) {
		return int(p.pattern[p.pos])
	}
	return -1
}

func (p *parser) take() (byte, error) {
	if p.pos >= len(p.pattern) {
		return 0, errInvalid
	}
	p.pos++
	return p.pattern[p.pos-1], nil
}

func (p *parser) escaped() (byte, error) {
	c, err := p.take()
	if err != nil || strings.IndexByte(escapable, c) < 0 {
		return 0, errInvalid
	}
	return c, nil
}

// parse parses the whole pattern and returns the start node. The accepting node comes last.
func (p *parser) parse() (int, error) {
	f, err := p.alternation()
	if err != nil || p.pos != len(p.pattern) {
		return 0, errInvalid
	}
	p.link(f.exit, p.node(match, [2]uint64{}))
	return f.entry, nil
}

func (p *parser) alternation() (fragment, error) {
	entry, exit := p.node(epsilon, [2]uint64{}), p.node(epsilon, [2]uint64{})
	for {
		f, err := p.sequence()
		if err != nil {
			return f, err
		}
		p.link(entry, f.entry)
		p.link(f.exit, exit)
		if p.peek() != '|' {
			return fragment{entry, exit}, nil
		}
		p.pos++
	}
}

func (p *parser) sequence() (fragment, error) {
	entry := p.node(epsilon, [2]uint64{})
	last := entry
	for c := p.peek(); c != -1 && c != '|' && c != ')'; c = p.peek() {
		f, err := p.item()
		if err != nil {
			return f, err
		}
		p.link(last, f.entry)
		last = f.exit
	}
	return fragment{entry, last}, nil
}

func (p *parser) item() (fragment, error) {
	f, err := p.atom()
	if err != nil {
		return f, err
	}
	switch p.peek() {
	case '*':
		p.pos++
		loop := p.node(epsilon, [2]uint64{})
		p.link(loop, f.entry)
		p.link(f.exit, loop)
		return fragment{loop, loop}, nil
	case '+':
		p.pos++
		p.link(f.exit, f.entry)
	case '?':
		p.pos++
		skip := p.node(epsilon, [2]uint64{})
		p.link(skip, f.entry)
		p.link(skip, f.exit)
		f.entry = skip
	}
	return f, nil
}

func (p *parser) atom() (fragment, error) {
	c, err := p.take()
	if err != nil {
		return fragment{}, err
	}
	var chars [2]uint64
	switch c {
	case '(':
		f, err := p.alternation()
		if err != nil {
			return f, err
		}
		if c, err := p.take(); err != nil || c != ')' {
			return f, errInvalid
		}
		return f, nil
	case '*', '+', '?':
		return fragment{}, errInvalid
	case '^':
		return p.wrap(begin, chars), nil
	case '$':
		return p.wrap(end, chars), nil
	case '.':
		chars = [2]uint64{^uint64(0), ^uint64(0)}
	case '[':
		if chars, err = p.bracket(); err != nil {
			return fragment{}, err
		}
	case '\\':
		if c, err = p.escaped(); err != nil {
			return fragment{}, err
		}
		addRange(&chars, c, c)
	default:
		addRange(&chars, c, c)
	}
	return p.wrap(char, chars), nil
}

func (p *parser) wrap(k kind, chars [2]uint64) fragment {
	n := p.node(k, chars)
	exit := p.node(epsilon, [2]uint64{})
	p.link(n, exit)
	return fragment{n, exit}
}

func addRange(chars *[2]uint64, low, high byte) {
	for c := int(low); c <= int(high); c++ {
		chars[c>>6] |= 1 << (c & 63)
	}
}

func (p *parser) bracket() ([2]uint64, error) {
	var chars [2]uint64
	negated := p.peek() == '^'
	if negated {
		p.pos++
	}
	type token struct {
		c    byte
		dash bool // an unescaped '-'
	}
	var body []token
	for {
		c, err := p.take()
		if err != nil {
			return chars, err
		}
		if c == ']' {
			break
		}
		if c == '\\' {
			if c, err = p.escaped(); err != nil {
				return chars, err
			}
			body = append(body, token{c, false})
		} else {
			body = append(body, token{c, c == '-'})
		}
	}
	if len(body) == 0 {
		return chars, errInvalid
	}
	start, stop := 0, len(body)
	if body[0].dash {
		addRange(&chars, '-', '-')
		start = 1
	}
	if stop > start && body[stop-1].dash {
		addRange(&chars, '-', '-')
		stop--
	}
	for i := start; i < stop; {
		switch {
		case body[i].dash:
			return chars, errInvalid
		case i+1 < stop && body[i+1].dash:
			if i+2 >= stop || body[i+2].dash || body[i].c > body[i+2].c {
				return chars, errInvalid
			}
			addRange(&chars, body[i].c, body[i+2].c)
			i += 3
		default:
			addRange(&chars, body[i].c, body[i].c)
			i++
		}
	}
	if negated {
		chars = [2]uint64{^chars[0], ^chars[1]}
	}
	return chars, nil
}

func set(b []uint64, k int) {
	b[k/64] |= 1 << (k % 64)
}

func intersects(a, b []uint64) bool {
	for w := range a {
		if a[w]&b[w] != 0 {
			return true
		}
	}
	return false
}

// state is one interned DFA state: a bitset over the kernel and what is cached about it.
type state struct {
	bits     []uint64
	back     [128]int32 // reverse transitions by character, -1 until computed
	next     int32      // the state after every member consumes a character, or -1
	entry    int32      // this state intersected with the start closure at an inner position
	accepts  bool
	consumes bool
	starts   bool
}

// matcher counts leftmost-longest matches in linear time per line.
//
// The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
// node that consumes a character. A reverse pass records at each position which kernel states
// can still reach acceptance from there. Forward scans start only where a match starts and keep
// only states that can still accept, so each scan stops right after its longest match. The
// forward DFA reads the reverse pass's state at each position as its input symbol.
type matcher struct {
	words     int
	follow    [][]uint64
	consumers [128][]int
	start     [2][2][]uint64 // the start closure, indexed by [at line start][at line end]
	last      [128][]uint64  // per character: what can accept from the last position
	scratch   []uint64
	key       []byte
	ids       map[string]int32
	states    []*state
	forward   map[uint64]int32
	lastIDs   [128]int32
	accept    int32
	alive     []int32
}

func newMatcher(nodes []node, startNode int) *matcher {
	kernel := make([]int, len(nodes))
	size := 1
	for v := range nodes {
		kernel[v] = -1
		if nodes[v].kind == char {
			kernel[v] = size
			size++
		}
	}
	kernel[len(nodes)-1] = 0
	words := (size + 63) / 64
	m := &matcher{words: words, scratch: make([]uint64, words), key: make([]byte, 8*words)}

	mark := make([]uint32, len(nodes))
	var stamp uint32
	var stack []int
	closure := func(roots []int, atBegin, atEnd bool) []uint64 {
		stamp++
		b := make([]uint64, words)
		visit := func(v int) {
			if mark[v] != stamp {
				mark[v] = stamp
				stack = append(stack, v)
			}
		}
		for _, v := range roots {
			visit(v)
		}
		for len(stack) > 0 {
			v := stack[len(stack)-1]
			stack = stack[:len(stack)-1]
			switch k := nodes[v].kind; {
			case k == char || k == match:
				set(b, kernel[v])
				continue
			case k == begin && !atBegin, k == end && !atEnd:
				continue
			}
			for _, w := range nodes[v].out {
				visit(w)
			}
		}
		return b
	}

	m.follow = make([][]uint64, size)
	followEnd := make([][]uint64, size)
	for v := range nodes {
		k := kernel[v]
		if k <= 0 {
			continue
		}
		m.follow[k] = closure(nodes[v].out, false, false)
		followEnd[k] = closure(nodes[v].out, false, true)
		for c := 32; c < 127; c++ {
			if nodes[v].chars[c>>6]>>(c&63)&1 == 1 {
				m.consumers[c] = append(m.consumers[c], k)
			}
		}
	}
	for b := range 2 {
		for e := range 2 {
			m.start[b][e] = closure([]int{startNode}, b == 1, e == 1)
		}
	}
	for c := range 128 {
		m.last[c] = make([]uint64, words)
		set(m.last[c], 0)
		for _, k := range m.consumers[c] {
			if followEnd[k][0]&1 == 1 {
				set(m.last[c], k)
			}
		}
	}
	m.reset()
	return m
}

func (m *matcher) reset() {
	m.ids = make(map[string]int32)
	m.states = m.states[:0]
	m.forward = make(map[uint64]int32)
	clear(m.scratch)
	set(m.scratch, 0)
	m.accept = m.intern()
	for c := range 128 {
		copy(m.scratch, m.last[c])
		m.lastIDs[c] = m.intern()
	}
}

// intern returns the state holding the bitset in m.scratch, adding it when it is new.
func (m *matcher) intern() int32 {
	for w, word := range m.scratch {
		binary.LittleEndian.PutUint64(m.key[8*w:], word)
	}
	if id, ok := m.ids[string(m.key)]; ok {
		return id
	}
	id := int32(len(m.states))
	b := slices.Clone(m.scratch)
	s := &state{bits: b, next: -1, entry: -1, accepts: b[0]&1 == 1, consumes: b[0] > 1, starts: intersects(m.start[0][0], b)}
	for _, word := range b[1:] {
		s.consumes = s.consumes || word != 0
	}
	for c := range s.back {
		s.back[c] = -1
	}
	m.states = append(m.states, s)
	m.ids[string(m.key)] = id
	return id
}

// meet returns the state holding the members of both a and state b.
func (m *matcher) meet(a []uint64, b int32) int32 {
	for w, word := range m.states[b].bits {
		m.scratch[w] = a[w] & word
	}
	return m.intern()
}

// stepBack returns the kernel states that can accept from a position holding c, given those
// that can accept from the next position.
func (m *matcher) stepBack(id int32, c byte) int32 {
	clear(m.scratch)
	set(m.scratch, 0)
	ahead := m.states[id].bits
	for _, k := range m.consumers[c] {
		if intersects(m.follow[k], ahead) {
			set(m.scratch, k)
		}
	}
	next := m.intern()
	m.states[id].back[c] = next
	return next
}

// consume returns the state after every member of a state consumes a character. The character
// is implied: every state in a pruned set accepts the character at its position.
func (m *matcher) consume(id int32) int32 {
	if next := m.states[id].next; next >= 0 {
		return next
	}
	clear(m.scratch)
	for w, word := range m.states[id].bits {
		if w == 0 {
			word &^= 1
		}
		for ; word != 0; word &= word - 1 {
			for v, f := range m.follow[64*w+bits.TrailingZeros64(word)] {
				m.scratch[v] |= f
			}
		}
	}
	next := m.intern()
	m.states[id].next = next
	return next
}

// step returns the pruned forward state after a state, at a position whose reverse state is alive.
func (m *matcher) step(id, alive int32) int32 {
	key := uint64(id)<<32 | uint64(uint32(alive))
	if next, ok := m.forward[key]; ok {
		return next
	}
	next := m.meet(m.states[m.consume(id)].bits, alive)
	m.forward[key] = next
	return next
}

// entryAt returns the forward state at a match candidate s of a line of n characters.
func (m *matcher) entryAt(s, n int, alive int32) int32 {
	if s == 0 || s == n {
		return m.meet(m.start[boolIndex(s == 0)][boolIndex(s == n)], alive)
	}
	if m.states[alive].entry < 0 {
		m.states[alive].entry = m.meet(m.start[0][0], alive)
	}
	return m.states[alive].entry
}

func boolIndex(b bool) int {
	if b {
		return 1
	}
	return 0
}

func (m *matcher) count(line []byte) int {
	if len(m.states)+len(m.forward) > stateLimit {
		m.reset()
	}
	n := len(line)
	m.alive = slices.Grow(m.alive[:0], n+1)[:n+1]
	alive := m.alive
	alive[n] = m.accept
	if n > 0 {
		id := m.lastIDs[line[n-1]]
		alive[n-1] = id
		for i := n - 2; i >= 0; i-- {
			c := line[i]
			next := m.states[id].back[c]
			if next < 0 {
				next = m.stepBack(id, c)
			}
			alive[i] = next
			id = next
		}
	}

	count := 0
	for p := 0; p <= n; {
		s := p
		for 0 < s && s < n && !m.states[alive[s]].starts {
			s++
		}
		current := m.entryAt(s, n, alive[s])
		if !m.states[current].accepts && !m.states[current].consumes {
			if s >= n {
				break
			}
			p = s + 1
			continue
		}
		end := -1
		if m.states[current].accepts {
			end = s
		}
		i := s
		for m.states[current].consumes {
			i++
			if i == n {
				current = m.accept
			} else {
				current = m.step(current, alive[i])
			}
			if m.states[current].accepts {
				end = i
			}
		}
		count++
		if end > s {
			p = end
		} else {
			p = s + 1
		}
	}
	return count
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	pattern, text, _ := bytes.Cut(input, []byte{'\n'})
	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	p := &parser{pattern: pattern}
	if start, err := p.parse(); err != nil {
		out.WriteString("invalid\n")
	} else {
		m := newMatcher(p.nodes, start)
		for len(text) > 0 {
			line, rest, _ := bytes.Cut(text, []byte{'\n'})
			out.Write(strconv.AppendInt(out.AvailableBuffer(), int64(m.count(line)), 10))
			out.WriteByte('\n')
			text = rest
		}
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
