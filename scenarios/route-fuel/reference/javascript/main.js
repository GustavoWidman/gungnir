/** A binary min-heap of (cost, time, place, fuel) states ordered by (cost, time), in parallel arrays. */
class Heap {
  constructor() {
    this.length = 0;
    this.cost = new Float64Array(1024);
    this.time = new Float64Array(1024);
    this.place = new Int32Array(1024);
    this.fuel = new Int32Array(1024);
  }

  push(cost, time, place, fuel) {
    if (this.length === this.cost.length) {
      for (const key of ["cost", "time", "place", "fuel"]) {
        const grown = new this[key].constructor(2 * this.length);
        grown.set(this[key]);
        this[key] = grown;
      }
    }
    let i = this.length++;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      if (this.cost[parent] < cost || (this.cost[parent] === cost && this.time[parent] <= time)) break;
      this.move(i, parent);
      i = parent;
    }
    this.set(i, cost, time, place, fuel);
  }

  /** Removes the state in slot 0, the first in order. */
  pop() {
    const last = --this.length;
    const cost = this.cost[last];
    const time = this.time[last];
    let i = 0;
    for (;;) {
      let child = 2 * i + 1;
      if (child >= last) break;
      if (child + 1 < last && this.before(child + 1, child)) child++;
      if (cost < this.cost[child] || (cost === this.cost[child] && time <= this.time[child])) break;
      this.move(i, child);
      i = child;
    }
    this.set(i, cost, time, this.place[last], this.fuel[last]);
  }

  before(i, j) {
    return this.cost[i] !== this.cost[j] ? this.cost[i] < this.cost[j] : this.time[i] < this.time[j];
  }

  move(to, from) {
    this.set(to, this.cost[from], this.time[from], this.place[from], this.fuel[from]);
  }

  set(i, cost, time, place, fuel) {
    this.cost[i] = cost;
    this.time[i] = time;
    this.place[i] = place;
    this.fuel[i] = fuel;
  }
}

/**
 * Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step. A state
 * is dominated once its place was expanded with at least as much fuel, since that earlier state
 * was no costlier and no slower.
 */
function bestTrip(map, search, id, start, fuel, target) {
  const { capacity, price, first, to, need, duration } = map;
  const { most, trip, heap } = search;
  heap.length = 0;
  heap.push(0, 0, start, fuel);
  while (heap.length > 0) {
    const cost = heap.cost[0];
    const time = heap.time[0];
    const place = heap.place[0];
    const f = heap.fuel[0];
    heap.pop();
    if (trip[place] === id && most[place] >= f) continue;
    if (place === target) return `${cost} ${time}`;
    trip[place] = id;
    most[place] = f;
    if (price[place] >= 0 && f < capacity) heap.push(cost + price[place], time, place, f + 1);
    for (let r = first[place]; r < first[place + 1]; r++) {
      const left = f - need[r];
      const w = to[r];
      if (left >= 0 && (trip[w] !== id || most[w] < left)) heap.push(cost, time + duration[r], w, left);
    }
  }
  return "unreachable";
}

async function main() {
  const tokens = (await Bun.stdin.text()).split(/\s+/);
  let pos = 0;
  const next = () => Number(tokens[pos++]);
  const n = next();
  const capacity = next();
  const stations = next();
  const m = next();
  const q = next();

  const price = new Float64Array(n + 1).fill(-1);
  for (let i = 0; i < stations; i++) {
    const place = next();
    price[place] = next();
  }

  // Roads that need more than the capacity can never be driven, so they are dropped. The rest
  // are grouped by the place they leave: roads first[v] to first[v + 1] - 1 leave place v.
  const from = new Int32Array(m);
  const roadTo = new Int32Array(m);
  const roadNeed = new Int32Array(m);
  const roadTime = new Float64Array(m);
  const first = new Int32Array(n + 2);
  let kept = 0;
  for (let i = 0; i < m; i++) {
    const u = next();
    const v = next();
    const a = next();
    const b = next();
    if (a > capacity) continue;
    from[kept] = u;
    roadTo[kept] = v;
    roadNeed[kept] = a;
    roadTime[kept] = b;
    first[u + 1]++;
    kept++;
  }
  for (let v = 1; v <= n + 1; v++) first[v] += first[v - 1];
  const fill = first.slice(0, n + 1);
  const to = new Int32Array(kept);
  const need = new Int32Array(kept);
  const duration = new Float64Array(kept);
  for (let i = 0; i < kept; i++) {
    const r = fill[from[i]]++;
    to[r] = roadTo[i];
    need[r] = roadNeed[i];
    duration[r] = roadTime[i];
  }
  const map = { capacity, price, first, to, need, duration };

  const search = { most: new Int32Array(n + 1), trip: new Int32Array(n + 1).fill(-1), heap: new Heap() };
  const answers = new Array(q);
  for (let id = 0; id < q; id++) {
    const start = next();
    const fuel = next();
    const target = next();
    answers[id] = bestTrip(map, search, id, start, fuel, target);
  }
  if (q > 0) await Bun.write(Bun.stdout, answers.join("\n") + "\n");
}

await main();
