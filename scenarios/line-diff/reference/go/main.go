package main

import (
	"bufio"
	"bytes"
	"io"
	"log"
	"math"
	"os"
	"strconv"
)

const none = math.MaxUint32

type hunk struct {
	x, a, y, b int
}

// intern numbers the lines of doc, giving equal lines equal numbers. A line keeps its line
// feed, so a last line without one differs from the same text with one.
func intern(doc []byte, ids map[string]uint32) []uint32 {
	var lines []uint32
	for len(doc) > 0 {
		length := bytes.IndexByte(doc, '\n') + 1
		if length == 0 {
			length = len(doc)
		}
		id, ok := ids[string(doc[:length])]
		if !ok {
			id = uint32(len(ids))
			ids[string(doc[:length])] = id
		}
		lines = append(lines, id)
		doc = doc[length:]
	}
	return lines
}

// offset is where round e starts in the flat slice of rounds: rounds 0 to e-1 hold 1 to e entries.
func offset(e int) int {
	return e * (e + 1) / 2
}

// searchBack is Myers' search run from the end of both documents. Entry i of round e is the
// smallest x on diagonal k = len(a) - len(b) - e + 2i such that (x, x-k) is at most e edits from
// the end, or none when no point of that diagonal is. It returns the rounds and the first round
// that reaches (0, 0), which is the shortest script's cost.
func searchBack(a, b []uint32, rounds []uint32) ([]uint32, int) {
	n, m := len(a), len(b)
	kend := n - m
	x, y := n, m
	for x > 0 && y > 0 && a[x-1] == b[y-1] {
		x--
		y--
	}
	rounds = append(rounds[:0], uint32(x))
	for e := 0; ; e++ {
		at := e - kend
		if at%2 == 0 && 0 <= at && at <= 2*e && rounds[offset(e)+at/2] == 0 {
			return rounds, e
		}
		next := e + 1
		prev, cur := offset(e), offset(next)
		for range next + 1 {
			rounds = append(rounds, none)
		}
		// Only the diagonals -m <= k <= n cross the grid.
		first := max(0, (next-n+1)/2)
		last := min(next, (m+next)/2)
		for i := first; i <= last; i++ {
			k := kend - next + 2*i
			lo := max(k, 0)
			best := math.MaxInt
			if i < next && rounds[prev+i] != none {
				// Back from diagonal k + 1 over a deletion.
				best = max(int(rounds[prev+i])-1, lo)
			}
			if i > 0 && rounds[prev+i-1] != none {
				// Back from diagonal k - 1 over an insertion.
				best = min(best, max(int(rounds[prev+i-1]), lo))
			}
			if best != math.MaxInt {
				sx, sy := best, best-k
				for sx > 0 && sy > 0 && a[sx-1] == b[sy-1] {
					sx--
					sy--
				}
				rounds[cur+i] = uint32(sx)
			}
		}
	}
}

// diff appends the hunks of the first shortest edit script in keep, delete, insert order.
func diff(a, b []uint32, rounds []uint32, hunks []hunk) ([]uint32, []hunk) {
	n, m := len(a), len(b)
	kend := n - m
	rounds, e := searchBack(a, b, rounds)
	keeps := func(x, y int) bool { return x < n && y < m && a[x] == b[y] }
	x, y := 0, 0
	for {
		for keeps(x, y) {
			x++
			y++
		}
		if x == n && y == m {
			return rounds, hunks
		}
		x0, y0 := x, y
		for {
			// Delete when (x + 1, y) is still within e - 1 edits of the end.
			i := (x - y + e - kend) / 2
			if x < n && i < e && int(rounds[offset(e-1)+i]) <= x+1 {
				x++
			} else {
				y++
			}
			e--
			if keeps(x, y) || (x == n && y == m) {
				break
			}
		}
		hunks = append(hunks, hunk{x0, x - x0, y0, y - y0})
	}
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	line, rest, _ := bytes.Cut(input, []byte{'\n'})
	pairs, err := strconv.Atoi(string(line))
	if err != nil {
		log.Fatal(err)
	}

	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	ids := make(map[string]uint32)
	var rounds []uint32
	var hunks []hunk
	for range pairs {
		line, rest, _ = bytes.Cut(rest, []byte{'\n'})
		first, second, _ := bytes.Cut(line, []byte{' '})
		n, err := strconv.Atoi(string(first))
		if err != nil {
			log.Fatal(err)
		}
		m, err := strconv.Atoi(string(second))
		if err != nil {
			log.Fatal(err)
		}
		clear(ids)
		a := intern(rest[:n], ids)
		b := intern(rest[n:n+m], ids)
		rest = rest[n+m:]

		rounds, hunks = diff(a, b, rounds, hunks[:0])
		deleted, inserted := 0, 0
		for _, h := range hunks {
			deleted += h.a
			inserted += h.b
		}
		buf := out.AvailableBuffer()
		buf = strconv.AppendInt(buf, int64(deleted), 10)
		buf = append(buf, ' ')
		buf = strconv.AppendInt(buf, int64(inserted), 10)
		buf = append(buf, ' ')
		buf = strconv.AppendInt(buf, int64(len(hunks)), 10)
		out.Write(append(buf, '\n'))
		for _, h := range hunks {
			buf := out.AvailableBuffer()
			for j, v := range [4]int{h.x, h.a, h.y, h.b} {
				if j > 0 {
					buf = append(buf, ' ')
				}
				buf = strconv.AppendInt(buf, int64(v), 10)
			}
			out.Write(append(buf, '\n'))
		}
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
