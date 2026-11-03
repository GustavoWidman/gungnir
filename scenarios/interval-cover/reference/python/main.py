import sys
from bisect import bisect_left, bisect_right


def main() -> None:
    data = sys.stdin.buffer.read().split()
    n, q = int(data[0]), int(data[1])
    bounds = data[2 : 2 + 2 * n]
    starts = sorted(map(int, bounds[0::2]))
    ends = sorted(map(int, bounds[1::2]))
    queries = map(int, data[2 + 2 * n : 2 + 2 * n + q])
    # Intervals containing x: those starting at or before x, minus those ending before x.
    counts = [bisect_right(starts, x) - bisect_left(ends, x) for x in queries]
    if counts:
        sys.stdout.write("\n".join(map(str, counts)))
        sys.stdout.write("\n")


if __name__ == "__main__":
    main()
