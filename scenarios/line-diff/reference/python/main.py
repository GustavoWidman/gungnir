import sys


def split_lines(doc: bytes) -> list[bytes]:
    """Splits a document into lines that keep their line feeds. Only the last may lack one."""
    lines = [line + b"\n" for line in doc.split(b"\n")]
    lines[-1] = lines[-1][:-1]
    if not lines[-1]:
        lines.pop()
    return lines


def search_back(a: list[int], b: list[int]) -> list[list[int]]:
    """Myers' search run from the end of both documents.

    rounds[e][i] is the smallest x on diagonal k = len(a) - len(b) - e + 2i such that
    (x, x - k) is at most e edits from the end, or len(a) + 1 when no point of that diagonal is.
    The last round is the first that reaches (0, 0), so its index is the shortest script's cost.
    """
    n, m = len(a), len(b)
    kend = n - m
    none = n + 1
    x, y = n, m
    while x and y and a[x - 1] == b[y - 1]:
        x -= 1
        y -= 1
    rounds = [[x]]
    e = 0
    while not ((e - kend) % 2 == 0 and abs(kend) <= e and rounds[e][(e - kend) // 2] == 0):
        e += 1
        prev = rounds[e - 1]
        cur = [none] * (e + 1)
        # Only the diagonals -m <= k <= n cross the grid.
        first = max(0, (-m - kend + e + 1) // 2)
        last = min(e, (n - kend + e) // 2)
        for i in range(first, last + 1):
            k = kend - e + 2 * i
            lo = k if k > 0 else 0
            best = none
            if i < e and prev[i] != none:
                # Back from diagonal k + 1 over a deletion.
                best = max(prev[i] - 1, lo)
            if i and prev[i - 1] != none:
                # Back from diagonal k - 1 over an insertion.
                best = min(best, max(prev[i - 1], lo))
            if best != none:
                x, y = best, best - k
                while x and y and a[x - 1] == b[y - 1]:
                    x -= 1
                    y -= 1
                cur[i] = x
        rounds.append(cur)
    return rounds


def diff(a: list[int], b: list[int]) -> list[tuple[int, int, int, int]]:
    """The hunks of the first shortest edit script in keep, delete, insert order."""
    n, m = len(a), len(b)
    kend = n - m
    rounds = search_back(a, b)
    e = len(rounds) - 1
    hunks = []
    x = y = 0
    while True:
        while x < n and y < m and a[x] == b[y]:
            x += 1
            y += 1
        if x == n and y == m:
            return hunks
        x0, y0 = x, y
        while True:
            # Delete when (x + 1, y) is still within e - 1 edits of the end.
            i = (x - y + e - kend) // 2
            if x < n and i < e and rounds[e - 1][i] <= x + 1:
                x += 1
            else:
                y += 1
            e -= 1
            if (x < n and y < m and a[x] == b[y]) or (x == n and y == m):
                break
        hunks.append((x0, x - x0, y0, y - y0))


def main() -> None:
    data = sys.stdin.buffer.read()
    pos = data.index(b"\n") + 1
    out = []
    for _ in range(int(data[:pos])):
        end = data.index(b"\n", pos)
        n, m = map(int, data[pos:end].split())
        first, second = data[end + 1 : end + 1 + n], data[end + 1 + n : end + 1 + n + m]
        pos = end + 1 + n + m
        ids: dict[bytes, int] = {}
        a = [ids.setdefault(line, len(ids)) for line in split_lines(first)]
        b = [ids.setdefault(line, len(ids)) for line in split_lines(second)]
        hunks = diff(a, b)
        deleted = sum(h[1] for h in hunks)
        inserted = sum(h[3] for h in hunks)
        out.append(b"%d %d %d\n" % (deleted, inserted, len(hunks)))
        out.extend(b"%d %d %d %d\n" % hunk for hunk in hunks)
    sys.stdout.buffer.write(b"".join(out))


if __name__ == "__main__":
    main()
