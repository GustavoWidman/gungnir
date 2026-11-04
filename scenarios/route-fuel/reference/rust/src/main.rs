use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::io::{self, Read, Write};

struct Road {
    to: usize,
    fuel: usize,
    time: u64,
}

struct Map {
    capacity: usize,
    /// The price per unit at each place, or `None` where there is no station.
    price: Vec<Option<u64>>,
    /// The drivable roads out of each place: those that need at most the capacity.
    roads: Vec<Vec<Road>>,
}

struct Search {
    /// The most fuel each place was expanded with during the trip numbered `trip[place]`.
    most: Vec<usize>,
    trip: Vec<usize>,
    heap: BinaryHeap<Reverse<(u64, u64, usize, usize)>>,
}

impl Search {
    /// Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step.
    fn best_trip(&mut self, map: &Map, id: usize, start: usize, fuel: usize, target: usize) -> Option<(u64, u64)> {
        // A state is dominated once its place was expanded with at least as much fuel, since
        // that earlier state was no costlier and no slower.
        let expanded = |search: &Self, place: usize| (search.trip[place] == id).then(|| search.most[place]);
        self.heap.clear();
        self.heap.push(Reverse((0, 0, start, fuel)));
        while let Some(Reverse((cost, time, place, fuel))) = self.heap.pop() {
            if expanded(self, place).is_some_and(|most| most >= fuel) {
                continue;
            }
            if place == target {
                return Some((cost, time));
            }
            self.trip[place] = id;
            self.most[place] = fuel;
            if let Some(price) = map.price[place]
                && fuel < map.capacity
            {
                self.heap.push(Reverse((cost + price, time, place, fuel + 1)));
            }
            for road in &map.roads[place] {
                if let Some(left) = fuel.checked_sub(road.fuel)
                    && expanded(self, road.to).is_none_or(|most| most < left)
                {
                    self.heap.push(Reverse((cost, time + road.time, road.to, left)));
                }
            }
        }
        None
    }
}

fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let mut tokens = input.split_ascii_whitespace().map(|t| t.parse::<usize>().unwrap());
    let mut next = || tokens.next().unwrap();
    let (n, capacity, s, m, q) = (next(), next(), next(), next(), next());

    let mut price = vec![None; n + 1];
    for _ in 0..s {
        let place = next();
        price[place] = Some(next() as u64);
    }
    let mut roads: Vec<Vec<Road>> = (0..=n).map(|_| Vec::new()).collect();
    for _ in 0..m {
        let (from, to, fuel, time) = (next(), next(), next(), next());
        if fuel <= capacity {
            roads[from].push(Road { to, fuel, time: time as u64 });
        }
    }
    let map = Map { capacity, price, roads };

    let mut search = Search { most: vec![0; n + 1], trip: vec![usize::MAX; n + 1], heap: BinaryHeap::new() };
    let mut out = io::BufWriter::with_capacity(1 << 16, io::stdout().lock());
    for id in 0..q {
        let (start, fuel, target) = (next(), next(), next());
        match search.best_trip(&map, id, start, fuel, target) {
            Some((cost, time)) => writeln!(out, "{cost} {time}").unwrap(),
            None => writeln!(out, "unreachable").unwrap(),
        }
    }
    out.flush().unwrap();
}
