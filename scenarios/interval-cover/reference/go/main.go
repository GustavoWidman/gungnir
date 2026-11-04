package main

import (
	"bufio"
	"cmp"
	"io"
	"log"
	"os"
	"slices"
	"sort"
	"strconv"
)

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<16), 1<<16)
	in.Split(bufio.ScanWords)
	next := func() int64 {
		if !in.Scan() {
			log.Fatal(cmp.Or(in.Err(), io.ErrUnexpectedEOF))
		}
		x, err := strconv.ParseInt(string(in.Bytes()), 10, 64)
		if err != nil {
			log.Fatal(err)
		}
		return x
	}

	n, q := next(), next()
	starts := make([]int64, n)
	ends := make([]int64, n)
	for i := range n {
		starts[i], ends[i] = next(), next()
	}
	slices.Sort(starts)
	slices.Sort(ends)

	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	for range q {
		x := next()
		// Intervals containing x: those starting at or before x, minus those ending before x.
		count := sort.Search(len(starts), func(i int) bool { return starts[i] > x }) -
			sort.Search(len(ends), func(i int) bool { return ends[i] >= x })
		line := strconv.AppendInt(out.AvailableBuffer(), int64(count), 10)
		out.Write(append(line, '\n'))
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
