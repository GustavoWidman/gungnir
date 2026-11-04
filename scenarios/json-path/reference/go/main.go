package main

import (
	"bytes"
	"io"
	"log"
	"os"
	"slices"
	"strconv"
)

const hexDigits = "0123456789abcdef"

type kind uint8

const (
	scalar kind = iota
	array
	object
)

// A node is a value of the document. Nodes are stored in document order, so a node's index
// orders it, and its descendants are exactly the nodes from its index + 1 up to end.
type node struct {
	kind kind
	// A scalar's printed text in document.text, or a container's children in document.kids.
	start, length uint32
	// The member name printed as a string, in document.text, when the parent is an object.
	nameStart, nameLength uint32
	end                   uint32
}

type document struct {
	nodes []node
	kids  []uint32
	text  []byte
}

type selectorKind uint8

const (
	nameSelector selectorKind = iota
	wildSelector
	indexSelector
	sliceSelector
)

// A selector's name is the member name printed as a string. Printing is one-to-one on code
// unit sequences, so two names are equal exactly when their printed forms are.
type selector struct {
	kind             selectorKind
	name             []byte
	index            int64
	start, end, step int64
	hasStart, hasEnd bool
}

type segment struct {
	descendant bool
	selector   selector
}

func appendUnit(out []byte, unit uint32) []byte {
	switch {
	case unit == 0x22:
		return append(out, '\\', '"')
	case unit == 0x5C:
		return append(out, '\\', '\\')
	case unit == 0x08:
		return append(out, '\\', 'b')
	case unit == 0x0C:
		return append(out, '\\', 'f')
	case unit == 0x0A:
		return append(out, '\\', 'n')
	case unit == 0x0D:
		return append(out, '\\', 'r')
	case unit == 0x09:
		return append(out, '\\', 't')
	case unit >= 0x20 && unit <= 0x7E:
		return append(out, byte(unit))
	}
	return append(out, '\\', 'u', hexDigits[unit>>12&15], hexDigits[unit>>8&15], hexDigits[unit>>4&15], hexDigits[unit&15])
}

func hexValue(b byte) (uint32, bool) {
	switch {
	case b >= '0' && b <= '9':
		return uint32(b - '0'), true
	case b >= 'a' && b <= 'f':
		return uint32(b-'a') + 10, true
	case b >= 'A' && b <= 'F':
		return uint32(b-'A') + 10, true
	}
	return 0, false
}

var escapes = map[byte]uint32{'"': 0x22, '\\': 0x5C, '/': 0x2F, 'b': 0x08, 'f': 0x0C, 'n': 0x0A, 'r': 0x0D, 't': 0x09}

// parseString parses the string literal at src[pos], which must be '"', and appends it to out
// as it prints. It returns out, the position after the closing quote, and false if the
// literal is invalid.
func parseString(src []byte, pos int, out []byte) ([]byte, int, bool) {
	out = append(out, '"')
	pos++
	for {
		run := pos
		for run < len(src) && src[run] >= 0x20 && src[run] < 0x7F && src[run] != '"' && src[run] != '\\' {
			run++
		}
		out = append(out, src[pos:run]...)
		pos = run
		if pos >= len(src) {
			return out, 0, false
		}
		b := src[pos]
		pos++
		switch {
		case b == '"':
			return append(out, '"'), pos, true
		case b == '\\':
			if pos >= len(src) {
				return out, 0, false
			}
			unit, ok := escapes[src[pos]]
			if src[pos] == 'u' {
				if pos+5 > len(src) {
					return out, 0, false
				}
				unit, ok = 0, true
				for _, d := range src[pos+1 : pos+5] {
					v, isHex := hexValue(d)
					ok = ok && isHex
					unit = unit<<4 | v
				}
				pos += 4
			}
			if !ok {
				return out, 0, false
			}
			pos++
			out = appendUnit(out, unit)
		case b == 0x7F:
			out = appendUnit(out, 0x7F)
		case b < 0x20:
			return out, 0, false
		default:
			var count int
			var low, high byte = 0x80, 0xBF
			switch {
			case b >= 0xC2 && b <= 0xDF:
				count = 1
			case b == 0xE0:
				count, low = 2, 0xA0
			case b >= 0xE1 && b <= 0xEC, b == 0xEE, b == 0xEF:
				count = 2
			case b == 0xED:
				count, high = 2, 0x9F
			case b == 0xF0:
				count, low = 3, 0x90
			case b >= 0xF1 && b <= 0xF3:
				count = 3
			case b == 0xF4:
				count, high = 3, 0x8F
			default:
				return out, 0, false
			}
			if pos+count > len(src) || src[pos] < low || src[pos] > high {
				return out, 0, false
			}
			point := uint32(b) & (0x3F >> count)
			for _, c := range src[pos : pos+count] {
				if c < 0x80 || c > 0xBF {
					return out, 0, false
				}
				point = point<<6 | uint32(c&0x3F)
			}
			pos += count
			if point >= 0x10000 {
				out = appendUnit(out, 0xD800+(point-0x10000)>>10)
				out = appendUnit(out, 0xDC00+(point-0x10000)&0x3FF)
			} else {
				out = appendUnit(out, point)
			}
		}
	}
}

// parseInteger parses -?(0|[1-9][0-9]*) at src[pos]. It returns the end, whether the value
// fits in 64 bits, and false if there is no integer there.
func parseInteger(src []byte, pos int) (int, bool, bool) {
	negative := pos < len(src) && src[pos] == '-'
	if negative {
		pos++
	}
	first := pos
	switch {
	case pos < len(src) && src[pos] == '0':
		pos++
	case pos < len(src) && src[pos] >= '1' && src[pos] <= '9':
		for pos < len(src) && src[pos] >= '0' && src[pos] <= '9' {
			pos++
		}
	default:
		return 0, false, false
	}
	digits, limit := src[first:pos], "9223372036854775807"
	if negative {
		limit = "9223372036854775808"
	}
	fits := len(digits) < len(limit) || len(digits) == len(limit) && string(digits) <= limit
	return pos, fits, true
}

type parser struct {
	src     []byte
	pos     int
	doc     document
	pending []uint32
}

func (p *parser) skipWhitespace() {
	for p.pos < len(p.src) {
		switch p.src[p.pos] {
		case ' ', '\t', '\n', '\r':
			p.pos++
		default:
			return
		}
	}
}

func (p *parser) eat(b byte) bool {
	p.skipWhitespace()
	if p.pos < len(p.src) && p.src[p.pos] == b {
		p.pos++
		return true
	}
	return false
}

func (p *parser) text(start int) (uint32, uint32) {
	return uint32(start), uint32(len(p.doc.text) - start)
}

func (p *parser) string() (uint32, uint32, bool) {
	start := len(p.doc.text)
	var ok bool
	p.doc.text, p.pos, ok = parseString(p.src, p.pos, p.doc.text)
	s, l := p.text(start)
	return s, l, ok
}

func (p *parser) value(nameStart, nameLength uint32) bool {
	p.skipWhitespace()
	if p.pos >= len(p.src) {
		return false
	}
	id := len(p.doc.nodes)
	p.doc.nodes = append(p.doc.nodes, node{nameStart: nameStart, nameLength: nameLength})
	n := node{kind: scalar, nameStart: nameStart, nameLength: nameLength}
	switch b := p.src[p.pos]; {
	case b == '[' || b == '{':
		p.pos++
		closer := byte(']')
		n.kind = array
		if b == '{' {
			closer, n.kind = '}', object
		}
		mark := len(p.pending)
		if !p.eat(closer) {
			for {
				var ns, nl uint32
				if n.kind == object {
					p.skipWhitespace()
					if p.pos >= len(p.src) || p.src[p.pos] != '"' {
						return false
					}
					var ok bool
					if ns, nl, ok = p.string(); !ok || !p.eat(':') {
						return false
					}
				}
				p.pending = append(p.pending, uint32(len(p.doc.nodes)))
				if !p.value(ns, nl) {
					return false
				}
				if p.eat(closer) {
					break
				}
				if !p.eat(',') {
					return false
				}
			}
		}
		n.start = uint32(len(p.doc.kids))
		p.doc.kids = append(p.doc.kids, p.pending[mark:]...)
		n.length = uint32(len(p.pending) - mark)
		p.pending = p.pending[:mark]
	case b == '"':
		var ok bool
		if n.start, n.length, ok = p.string(); !ok {
			return false
		}
	case b == '-' || b >= '0' && b <= '9':
		end, fits, ok := parseInteger(p.src, p.pos)
		if !ok || !fits {
			return false
		}
		start := len(p.doc.text)
		if digits := p.src[p.pos:end]; string(digits) == "-0" {
			p.doc.text = append(p.doc.text, '0')
		} else {
			p.doc.text = append(p.doc.text, digits...)
		}
		p.pos = end
		n.start, n.length = p.text(start)
	default:
		var word string
		for _, w := range [...]string{"true", "false", "null"} {
			if bytes.HasPrefix(p.src[p.pos:], []byte(w)) {
				word = w
			}
		}
		if word == "" {
			return false
		}
		p.pos += len(word)
		start := len(p.doc.text)
		p.doc.text = append(p.doc.text, word...)
		n.start, n.length = p.text(start)
	}
	n.end = uint32(len(p.doc.nodes))
	p.doc.nodes[id] = n
	return true
}

func parseDocument(src []byte) (*document, bool) {
	p := parser{src: src, doc: document{text: make([]byte, 0, len(src))}}
	if !p.value(0, 0) {
		return nil, false
	}
	p.skipWhitespace()
	return &p.doc, p.pos == len(src)
}

// bound parses an optional slice bound or index at q[*pos]; ok is false if one is malformed.
func bound(q []byte, pos *int) (value int64, present, ok bool) {
	if *pos >= len(q) || q[*pos] != '-' && (q[*pos] < '0' || q[*pos] > '9') {
		return 0, false, true
	}
	end, fits, found := parseInteger(q, *pos)
	if !found || !fits {
		return 0, false, false
	}
	value, err := strconv.ParseInt(string(q[*pos:end]), 10, 64)
	*pos = end
	return value, true, err == nil
}

// parseBracket parses the selector of the bracket that opens at q[pos]; it returns the
// selector, the position after ']', and false if the bracket is invalid.
func parseBracket(q []byte, pos int) (selector, int, bool) {
	pos++
	var sel selector
	switch {
	case pos < len(q) && q[pos] == '*':
		sel.kind = wildSelector
		pos++
	case pos < len(q) && q[pos] == '"':
		var ok bool
		if sel.name, pos, ok = parseString(q, pos, nil); !ok {
			return sel, 0, false
		}
	default:
		var ok bool
		if sel.start, sel.hasStart, ok = bound(q, &pos); !ok {
			return sel, 0, false
		}
		if pos < len(q) && q[pos] == ':' {
			pos++
			sel.kind, sel.step = sliceSelector, 1
			if sel.end, sel.hasEnd, ok = bound(q, &pos); !ok {
				return sel, 0, false
			}
			if pos < len(q) && q[pos] == ':' {
				pos++
				step, hasStep, ok := bound(q, &pos)
				if !ok || hasStep && step < 1 {
					return sel, 0, false
				}
				if hasStep {
					sel.step = step
				}
			}
		} else {
			if !sel.hasStart {
				return sel, 0, false
			}
			sel.kind, sel.index = indexSelector, sel.start
		}
	}
	if pos >= len(q) || q[pos] != ']' {
		return sel, 0, false
	}
	return sel, pos + 1, true
}

func isNameStart(c byte) bool {
	return c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c == '_'
}

func parseQuery(q []byte) ([]segment, bool) {
	if len(q) == 0 || q[0] != '$' {
		return nil, false
	}
	var segments []segment
	for pos := 1; pos < len(q); {
		descendant, dotted := false, false
		if bytes.HasPrefix(q[pos:], []byte("..")) {
			descendant, dotted, pos = true, true, pos+2
		} else if q[pos] == '.' {
			dotted, pos = true, pos+1
		}
		var sel selector
		switch {
		case pos < len(q) && q[pos] == '[' && (descendant || !dotted):
			var ok bool
			if sel, pos, ok = parseBracket(q, pos); !ok {
				return nil, false
			}
		case dotted && pos < len(q) && q[pos] == '*':
			sel.kind = wildSelector
			pos++
		case dotted && pos < len(q) && isNameStart(q[pos]):
			start := pos
			for pos < len(q) && (isNameStart(q[pos]) || q[pos] >= '0' && q[pos] <= '9') {
				pos++
			}
			sel.name = append(append([]byte{'"'}, q[start:pos]...), '"')
		default:
			return nil, false
		}
		segments = append(segments, segment{descendant, sel})
	}
	return segments, true
}

// sliceRange normalizes a slice to its first position, the bound it stays below, and its step.
func sliceRange(sel *selector, length int) (int, int, int) {
	clamp := func(b int64) int {
		if b < 0 {
			b += int64(length)
		}
		return int(min(max(b, 0), int64(length)))
	}
	start, end := 0, length
	if sel.hasStart {
		start = clamp(sel.start)
	}
	if sel.hasEnd {
		end = clamp(sel.end)
	}
	return start, end, int(min(sel.step, int64(length)+1))
}

func indexPosition(index int64, length int) int {
	if index < 0 {
		index += int64(length)
	}
	if index < 0 || index >= int64(length) {
		return -1
	}
	return int(index)
}

func (d *document) kidsOf(n *node) []uint32 {
	if n.kind == scalar {
		return nil
	}
	return d.kids[n.start : n.start+n.length]
}

func (d *document) name(id uint32) []byte {
	n := &d.nodes[id]
	return d.text[n.nameStart : n.nameStart+n.nameLength]
}

func (d *document) children(id uint32, sel *selector, out []uint32) []uint32 {
	n := &d.nodes[id]
	kids := d.kidsOf(n)
	switch {
	case n.kind == object && sel.kind == nameSelector:
		for _, c := range kids {
			if bytes.Equal(d.name(c), sel.name) {
				out = append(out, c)
			}
		}
	case n.kind != scalar && sel.kind == wildSelector:
		out = append(out, kids...)
	case n.kind == array && sel.kind == indexSelector:
		if j := indexPosition(sel.index, len(kids)); j >= 0 {
			out = append(out, kids[j])
		}
	case n.kind == array && sel.kind == sliceSelector:
		start, end, step := sliceRange(sel, len(kids))
		for j := start; j < end; j += step {
			out = append(out, kids[j])
		}
	}
	return out
}

// descend appends what sel picks from id and each of its descendants, in document order.
func (d *document) descend(id uint32, sel *selector, out []uint32) []uint32 {
	n := &d.nodes[id]
	if n.kind == scalar {
		return out
	}
	kids := d.kidsOf(n)
	start, end, step := 0, 0, 1
	if n.kind == array && sel.kind == indexSelector {
		if j := indexPosition(sel.index, len(kids)); j >= 0 {
			start, end = j, j+1
		}
	} else if n.kind == array && sel.kind == sliceSelector {
		start, end, step = sliceRange(sel, len(kids))
	}
	for j, c := range kids {
		var picked bool
		switch sel.kind {
		case nameSelector:
			picked = n.kind == object && bytes.Equal(d.name(c), sel.name)
		case wildSelector:
			picked = true
		default:
			picked = j >= start && j < end && (j-start)%step == 0
		}
		if picked {
			out = append(out, c)
		}
		out = d.descend(c, sel, out)
	}
	return out
}

// evaluate returns the nodes a query selects, each once, in document order. A set holds no
// node nested in another until a descendant segment, and only a nested set can come out
// unsorted.
func (d *document) evaluate(segments []segment) []uint32 {
	nodes, nested := []uint32{0}, false
	for i := range segments {
		seg := &segments[i]
		var out []uint32
		if seg.descendant {
			covered := uint32(0)
			for _, id := range nodes {
				if id >= covered {
					out = d.descend(id, &seg.selector, out)
					covered = d.nodes[id].end
				}
			}
			nested = true
		} else {
			for _, id := range nodes {
				out = d.children(id, &seg.selector, out)
			}
			if nested {
				slices.Sort(out)
			}
		}
		nodes = out
	}
	return nodes
}

func (d *document) print(id uint32, out []byte) []byte {
	n := &d.nodes[id]
	if n.kind == scalar {
		return append(out, d.text[n.start:n.start+n.length]...)
	}
	open, close := byte('['), byte(']')
	if n.kind == object {
		open, close = '{', '}'
	}
	out = append(out, open)
	for j, c := range d.kidsOf(n) {
		if j > 0 {
			out = append(out, ',')
		}
		if n.kind == object {
			out = append(append(out, d.name(c)...), ':')
		}
		out = d.print(c, out)
	}
	return append(out, close)
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	first, rest, _ := bytes.Cut(input, []byte("\n"))
	count, err := strconv.Atoi(string(first))
	if err != nil {
		log.Fatal(err)
	}
	queries := make([][]byte, count)
	for i := range queries {
		queries[i], rest, _ = bytes.Cut(rest, []byte("\n"))
	}

	var out []byte
	doc, ok := parseDocument(rest)
	if !ok {
		out = append(out, "invalid document\n"...)
	} else {
		for _, query := range queries {
			segments, ok := parseQuery(query)
			if !ok {
				out = append(out, "invalid query\n"...)
				continue
			}
			nodes := doc.evaluate(segments)
			out = append(strconv.AppendInt(out, int64(len(nodes)), 10), '\n')
			for _, id := range nodes {
				out = append(doc.print(id, out), '\n')
			}
		}
	}
	if _, err := os.Stdout.Write(out); err != nil {
		log.Fatal(err)
	}
}
