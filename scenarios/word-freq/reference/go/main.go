package main

import (
	"bufio"
	"cmp"
	"io"
	"log"
	"os"
	"slices"
	"strconv"
	"strings"
)

type entry struct {
	word  string
	count int
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	first, text, _ := strings.Cut(string(input), "\n")
	k, err := strconv.Atoi(first)
	if err != nil {
		log.Fatal(err)
	}
	text = strings.ToLower(text)

	counts := make(map[string]int)
	for i := 0; i < len(text); {
		for i < len(text) && !isLetter(text[i]) {
			i++
		}
		start := i
		for i < len(text) && isLetter(text[i]) {
			i++
		}
		if i > start {
			counts[text[start:i]]++
		}
	}

	ranked := make([]entry, 0, len(counts))
	for word, count := range counts {
		ranked = append(ranked, entry{word, count})
	}
	slices.SortFunc(ranked, func(a, b entry) int {
		if a.count != b.count {
			return cmp.Compare(b.count, a.count)
		}
		return strings.Compare(a.word, b.word)
	})

	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	for _, e := range ranked[:min(k, len(ranked))] {
		line := append(out.AvailableBuffer(), e.word...)
		line = append(line, ' ')
		line = strconv.AppendInt(line, int64(e.count), 10)
		out.Write(append(line, '\n'))
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}

func isLetter(c byte) bool {
	return 'a' <= c && c <= 'z'
}
