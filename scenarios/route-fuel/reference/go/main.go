package main

import (
	"bufio"
	"cmp"
	"io"
	"log"
	"os"
	"strconv"
)

type road struct {
	to, fuel int
	time     int64
}

type state struct {
	cost, time  int64
	place, fuel int
}

func (a state) before(b state) bool {
	return a.cost < b.cost || a.cost == b.cost && a.time < b.time
}

// queue is a binary min-heap of states ordered by (cost, time).
type queue []state

func (q *queue) push(s state) {
	h := append(*q, s)
	i := len(h) - 1
	for i > 0 && s.before(h[(i-1)/2]) {
		h[i] = h[(i-1)/2]
		i = (i - 1) / 2
	}
	h[i] = s
	*q = h
}

func (q *queue) pop() state {
	h := *q
	top, last := h[0], h[len(h)-1]
	h = h[:len(h)-1]
	i := 0
	for {
		child := 2*i + 1
		if child >= len(h) {
			break
		}
		if child+1 < len(h) && h[child+1].before(h[child]) {
			child++
		}
		if !h[child].before(last) {
			break
		}
		h[i] = h[child]
		i = child
	}
	if len(h) > 0 {
		h[i] = last
	}
	*q = h
	return top
}

type search struct {
	capacity int
	price    []int64 // -1 where there is no station
	roads    [][]road
	most     []int // the most fuel each place was expanded with during trip trip[place]
	trip     []int
	queue    queue
}

// bestTrip runs Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit
// per step. A state is dominated once its place was expanded with at least as much fuel, since
// that earlier state was no costlier and no slower.
func (s *search) bestTrip(id, start, fuel, target int) (state, bool) {
	expanded := func(place int) int {
		if s.trip[place] == id {
			return s.most[place]
		}
		return -1
	}
	s.queue = append(s.queue[:0], state{0, 0, start, fuel})
	for len(s.queue) > 0 {
		cur := s.queue.pop()
		if expanded(cur.place) >= cur.fuel {
			continue
		}
		if cur.place == target {
			return cur, true
		}
		s.trip[cur.place], s.most[cur.place] = id, cur.fuel
		if p := s.price[cur.place]; p >= 0 && cur.fuel < s.capacity {
			s.queue.push(state{cur.cost + p, cur.time, cur.place, cur.fuel + 1})
		}
		for _, r := range s.roads[cur.place] {
			if left := cur.fuel - r.fuel; left >= 0 && expanded(r.to) < left {
				s.queue.push(state{cur.cost, cur.time + r.time, r.to, left})
			}
		}
	}
	return state{}, false
}

func main() {
	in := bufio.NewScanner(os.Stdin)
	in.Buffer(make([]byte, 1<<16), 1<<16)
	in.Split(bufio.ScanWords)
	next := func() int {
		if !in.Scan() {
			log.Fatal(cmp.Or(in.Err(), io.ErrUnexpectedEOF))
		}
		x, err := strconv.Atoi(string(in.Bytes()))
		if err != nil {
			log.Fatal(err)
		}
		return x
	}

	n, capacity, stations, m, q := next(), next(), next(), next(), next()
	s := &search{
		capacity: capacity,
		price:    make([]int64, n+1),
		roads:    make([][]road, n+1),
		most:     make([]int, n+1),
		trip:     make([]int, n+1),
	}
	for i := range s.price {
		s.price[i], s.trip[i] = -1, -1
	}
	for range stations {
		place := next()
		s.price[place] = int64(next())
	}
	for range m {
		from, to, fuel, time := next(), next(), next(), next()
		if fuel <= capacity {
			s.roads[from] = append(s.roads[from], road{to, fuel, int64(time)})
		}
	}

	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	for id := range q {
		start, fuel, target := next(), next(), next()
		line := out.AvailableBuffer()
		if best, ok := s.bestTrip(id, start, fuel, target); ok {
			line = strconv.AppendInt(line, best.cost, 10)
			line = append(line, ' ')
			line = strconv.AppendInt(line, best.time, 10)
		} else {
			line = append(line, "unreachable"...)
		}
		out.Write(append(line, '\n'))
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
