/** A binary min-heap of (cost, time, place, fuel) states ordered by (cost, time), in parallel arrays. */
class Heap {
  length = 0;
  cost = new Float64Array(1024);
  time = new Float64Array(1024);
  place = new Int32Array(1024);
  fuel = new Int32Array(1024);

  push(cost: number, time: number, place: number, fuel: number): void {
    if (this.length === this.cost.length) this.grow();
    let i = this.length++;
    while (i > 0) {
      const parent = (i - 1) >> 1;
      const c = this.cost[parent]!;
      if (c < cost || (c === cost && this.time[parent]! <= time)) break;
      this.move(i, parent);
      i = parent;
    }
    this.set(i, cost, time, place, fuel);
  }

  /** Removes the state in slot 0, the first in order. */
  pop(): void {
    const last = --this.length;
    const cost = this.cost[last]!;
    const time = this.time[last]!;
    let i = 0;
    for (;;) {
      let child = 2 * i + 1;
      if (child >= last) break;
      if (child + 1 < last && this.before(child + 1, child)) child++;
      const c = this.cost[child]!;
      if (cost < c || (cost === c && time <= this.time[child]!)) break;
      this.move(i, child);
      i = child;
    }
    this.set(i, cost, time, this.place[last]!, this.fuel[last]!);
  }

  private grow(): void {
    const size = 2 * this.length;
    const cost = new Float64Array(size);
    const time = new Float64Array(size);
    const place = new Int32Array(size);
    const fuel = new Int32Array(size);
    cost.set(this.cost);
    time.set(this.time);
    place.set(this.place);
    fuel.set(this.fuel);
    Object.assign(this, { cost, time, place, fuel });
  }

  private before(i: number, j: number): boolean {
    const a = this.cost[i]!;
    const b = this.cost[j]!;
    return a !== b ? a < b : this.time[i]! < this.time[j]!;
  }

  private move(to: number, from: number): void {
    this.set(to, this.cost[from]!, this.time[from]!, this.place[from]!, this.fuel[from]!);
  }

  private set(i: number, cost: number, time: number, place: number, fuel: number): void {
    this.cost[i] = cost;
    this.time[i] = time;
    this.place[i] = place;
    this.fuel[i] = fuel;
  }
}

interface RoadMap {
  capacity: number;
  /** The price per unit at each place, or -1 where there is no station. */
  price: Float64Array;
  /** Roads first[v] to first[v + 1] - 1 leave place v. */
  first: Int32Array;
  to: Int32Array;
  need: Int32Array;
  duration: Float64Array;
}

interface Search {
  /** The most fuel each place was expanded with during the trip numbered trip[place]. */
  most: Int32Array;
  trip: Int32Array;
  heap: Heap;
}

/**
 * Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step. A state
 * is dominated once its place was expanded with at least as much fuel, since that earlier state
 * was no costlier and no slower.
 */
function bestTrip(map: RoadMap, search: Search, id: number, start: number, fuel: number, target: number): string {
  const { capacity, price, first, to, need, duration } = map;
  const { most, trip, heap } = search;
  heap.length = 0;
  heap.push(0, 0, start, fuel);
  while (heap.length > 0) {
    const cost = heap.cost[0]!;
    const time = heap.time[0]!;
    const place = heap.place[0]!;
    const f = heap.fuel[0]!;
    heap.pop();
    if (trip[place] === id && most[place]! >= f) continue;
    if (place === target) return `${cost} ${time}`;
    trip[place] = id;
    most[place] = f;
    const p = price[place]!;
    if (p >= 0 && f < capacity) heap.push(cost + p, time, place, f + 1);
    for (let r = first[place]!; r < first[place + 1]!; r++) {
      const left = f - need[r]!;
      const w = to[r]!;
      if (left >= 0 && (trip[w] !== id || most[w]! < left)) heap.push(cost, time + duration[r]!, w, left);
    }
  }
  return "unreachable";
}

async function main(): Promise<void> {
  const tokens = (await Bun.stdin.text()).split(/\s+/);
  let pos = 0;
  const next = (): number => Number(tokens[pos++]);
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
  // are grouped by the place they leave.
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
    first[u + 1]!++;
    kept++;
  }
  for (let v = 1; v <= n + 1; v++) first[v]! += first[v - 1]!;
  const fill = first.slice(0, n + 1);
  const to = new Int32Array(kept);
  const need = new Int32Array(kept);
  const duration = new Float64Array(kept);
  for (let i = 0; i < kept; i++) {
    const r = fill[from[i]!]!++;
    to[r] = roadTo[i]!;
    need[r] = roadNeed[i]!;
    duration[r] = roadTime[i]!;
  }
  const map: RoadMap = { capacity, price, first, to, need, duration };

  const search: Search = { most: new Int32Array(n + 1), trip: new Int32Array(n + 1).fill(-1), heap: new Heap() };
  const answers = new Array<string>(q);
  for (let id = 0; id < q; id++) {
    const start = next();
    const fuel = next();
    const target = next();
    answers[id] = bestTrip(map, search, id, start, fuel, target);
  }
  if (q > 0) await Bun.write(Bun.stdout, answers.join("\n") + "\n");
}

await main();
