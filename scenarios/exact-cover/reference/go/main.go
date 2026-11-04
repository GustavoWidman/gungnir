package main

import (
	"bufio"
	"cmp"
	"fmt"
	"io"
	"log"
	"os"
	"slices"
	"strconv"
	"strings"
)

type cell struct{ r, c int }

type shape []cell

func compareCells(a, b cell) int {
	return cmp.Or(cmp.Compare(a.r, b.r), cmp.Compare(a.c, b.c))
}

// normalize shifts the shape to row and column 0 and sorts its cells.
func normalize(cells shape) shape {
	top, left := cells[0].r, cells[0].c
	for _, x := range cells {
		top, left = min(top, x.r), min(left, x.c)
	}
	out := make(shape, len(cells))
	for i, x := range cells {
		out[i] = cell{x.r - top, x.c - left}
	}
	slices.SortFunc(out, compareCells)
	return out
}

// orientations returns the distinct shapes the 4 rotations and their mirror images give,
// the smallest first.
func orientations(cells shape) []shape {
	cells = slices.Clone(cells)
	var found []shape
	for range 2 {
		for range 4 {
			for i, x := range cells {
				cells[i] = cell{x.c, -x.r}
			}
			found = append(found, normalize(cells))
		}
		for i := range cells {
			cells[i].c = -cells[i].c
		}
	}
	compare := func(a, b shape) int { return slices.CompareFunc(a, b, compareCells) }
	slices.SortFunc(found, compare)
	return slices.CompactFunc(found, func(a, b shape) bool { return compare(a, b) == 0 })
}

// dlx is Algorithm X with dancing links. Cell columns are primary; each shape has one more
// column, covered once all its copies are placed, so copies of a shape are never told apart.
type dlx struct {
	cells, root                         int
	copies                              []int
	left, right, up, down, column, size []int
}

func newDlx(cells int, copies []int) *dlx {
	root := cells + len(copies)
	x := &dlx{cells: cells, root: root, copies: copies, size: make([]int, root+1)}
	for c := 0; c <= root; c++ {
		x.left = append(x.left, c)
		x.right = append(x.right, c)
		x.up = append(x.up, c)
		x.down = append(x.down, c)
		x.column = append(x.column, c)
	}
	// Only the cell columns and the root form the header ring.
	previous := root
	for c := range cells {
		x.right[previous], x.left[c] = c, previous
		previous = c
	}
	x.right[previous], x.left[root] = root, previous
	return x
}

func (x *dlx) addRow(columns []int) {
	first, count := len(x.left), len(columns)
	for k, col := range columns {
		node := first + k
		x.column = append(x.column, col)
		x.up = append(x.up, x.up[col])
		x.down = append(x.down, col)
		x.down[x.up[col]] = node
		x.up[col] = node
		x.size[col]++
		l, r := node-1, node+1
		if k == 0 {
			l = first + count - 1
		}
		if k+1 == count {
			r = first
		}
		x.left = append(x.left, l)
		x.right = append(x.right, r)
	}
}

func (x *dlx) cover(c int) {
	left, right, up, down, column, size := x.left, x.right, x.up, x.down, x.column, x.size
	left[right[c]] = left[c]
	right[left[c]] = right[c]
	for i := down[c]; i != c; i = down[i] {
		for j := right[i]; j != i; j = right[j] {
			up[down[j]] = up[j]
			down[up[j]] = down[j]
			size[column[j]]--
		}
	}
}

func (x *dlx) uncover(c int) {
	left, right, up, down, column, size := x.left, x.right, x.up, x.down, x.column, x.size
	for i := up[c]; i != c; i = up[i] {
		for j := left[i]; j != i; j = left[j] {
			size[column[j]]++
			up[down[j]] = j
			down[up[j]] = j
		}
	}
	left[right[c]] = c
	right[left[c]] = c
}

func (x *dlx) place(row int) {
	for j := x.right[row]; j != row; j = x.right[j] {
		col := x.column[j]
		if col < x.cells {
			x.cover(col)
		} else if x.copies[col-x.cells]--; x.copies[col-x.cells] == 0 {
			x.cover(col)
		}
	}
}

func (x *dlx) unplace(row int) {
	for j := x.left[row]; j != row; j = x.left[j] {
		col := x.column[j]
		if col < x.cells {
			x.uncover(col)
		} else {
			if x.copies[col-x.cells] == 0 {
				x.uncover(col)
			}
			x.copies[col-x.cells]++
		}
	}
}

// count returns the number of exact covers, choosing at each step the cell with the fewest
// options.
func (x *dlx) count() uint64 {
	best := x.right[x.root]
	if best == x.root {
		return 1
	}
	for c := x.right[best]; c != x.root && x.size[best] > 0; c = x.right[c] {
		if x.size[c] < x.size[best] {
			best = c
		}
	}
	if x.size[best] == 0 {
		return 0
	}
	x.cover(best)
	var total uint64
	for row := x.down[best]; row != best; row = x.down[row] {
		x.place(row)
		total += x.count()
		x.unplace(row)
	}
	x.uncover(best)
	return total
}

func main() {
	data, err := io.ReadAll(bufio.NewReader(os.Stdin))
	if err != nil {
		log.Fatal(err)
	}
	tokens := strings.Fields(string(data))
	next := func() string {
		token := tokens[0]
		tokens = tokens[1:]
		return token
	}
	number := func() int {
		n, err := strconv.Atoi(next())
		if err != nil {
			log.Fatal(err)
		}
		return n
	}

	height, width := number(), number()
	index := make([]int, height*width)
	cells := 0
	for r := range height {
		line := next()
		for c := range width {
			index[r*width+c] = -1
			if line[c] == '.' {
				index[r*width+c] = cells
				cells++
			}
		}
	}

	pieces := number()
	var shapes []shape
	var copies []int
	area := 0
	for range pieces {
		rows := number()
		number()
		var piece shape
		for r := range rows {
			for c, ch := range []byte(next()) {
				if ch == '#' {
					piece = append(piece, cell{r, c})
				}
			}
		}
		area += len(piece)
		canonical := orientations(piece)[0]
		kind := slices.IndexFunc(shapes, func(s shape) bool { return slices.Equal(s, canonical) })
		if kind < 0 {
			kind = len(shapes)
			shapes = append(shapes, canonical)
			copies = append(copies, 0)
		}
		copies[kind]++
	}

	var found uint64
	if area == cells {
		x := newDlx(cells, slices.Clone(copies))
		row := make([]int, 0, 65)
		for kind, s := range shapes {
			for _, oriented := range orientations(s) {
				for top := range height {
					for left := range width {
						row = row[:0]
						for _, p := range oriented {
							r, c := top+p.r, left+p.c
							if r >= height || c >= width || index[r*width+c] < 0 {
								break
							}
							row = append(row, index[r*width+c])
						}
						if len(row) == len(oriented) {
							x.addRow(append(row, cells+kind))
						}
					}
				}
			}
		}
		found = x.count()
		for _, k := range copies {
			for f := 2; f <= k && found > 0; f++ {
				found *= uint64(f)
			}
		}
	}
	fmt.Println(found)
}
