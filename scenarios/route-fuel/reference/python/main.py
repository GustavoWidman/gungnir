import sys
from heapq import heappop, heappush


def main() -> None:
    data = sys.stdin.buffer.read().split()
    n, capacity, s, m, q = map(int, data[:5])
    pos = 5
    price = [-1] * (n + 1)
    for _ in range(s):
        price[int(data[pos])] = int(data[pos + 1])
        pos += 2
    roads = [[] for _ in range(n + 1)]
    for _ in range(m):
        u, v, fuel, time = map(int, data[pos : pos + 4])
        pos += 4
        if fuel <= capacity:
            roads[u].append((v, fuel, time))
    answers = []
    for _ in range(q):
        start, fuel, target = map(int, data[pos : pos + 3])
        pos += 3
        answers.append(best_trip(roads, price, capacity, start, fuel, target))
    if answers:
        sys.stdout.write("\n".join(answers))
        sys.stdout.write("\n")


def best_trip(roads, price, capacity, start, fuel, target) -> str:
    """Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step."""
    # A state is dominated once its place was expanded with at least as much fuel, since that
    # earlier state was no costlier and no slower. most[v] is the most fuel v was expanded with.
    most = {}
    heap = [(0, 0, start, fuel)]
    while heap:
        cost, time, v, f = heappop(heap)
        if most.get(v, -1) >= f:
            continue
        if v == target:
            return f"{cost} {time}"
        most[v] = f
        if price[v] >= 0 and f < capacity:
            heappush(heap, (cost + price[v], time, v, f + 1))
        for w, need, duration in roads[v]:
            left = f - need
            if left >= 0 and most.get(w, -1) < left:
                heappush(heap, (cost, time + duration, w, left))
    return "unreachable"


if __name__ == "__main__":
    main()
