import sys
from math import factorial

type Shape = tuple[tuple[int, int], ...]


def normalize(cells: list[tuple[int, int]]) -> Shape:
    top = min(r for r, _ in cells)
    left = min(c for _, c in cells)
    return tuple(sorted((r - top, c - left) for r, c in cells))


def orientations(cells: list[tuple[int, int]]) -> list[Shape]:
    """The distinct shapes the 4 rotations and their mirror images give."""
    found = set()
    for _ in range(2):
        for _ in range(4):
            cells = [(c, -r) for r, c in cells]
            found.add(normalize(cells))
        cells = [(r, -c) for r, c in cells]
    return sorted(found)


class Dlx:
    """Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
    covered once all its copies are placed, so copies of a shape are never told apart."""

    def __init__(self, cells: int, copies: list[int], rows: list[list[int]]):
        columns = cells + len(copies)
        self.cells = cells
        self.copies = copies
        self.root = root = columns
        self.left = left = list(range(columns + 1))
        self.right = right = list(range(columns + 1))
        self.up = up = list(range(columns + 1))
        self.down = down = list(range(columns + 1))
        self.column = column = list(range(columns + 1))
        self.size = size = [0] * (columns + 1)
        ring = [*range(cells), root]
        for a, b in zip(ring, ring[1:] + ring[:1]):
            right[a], left[b] = b, a
        for row in rows:
            first = len(left)
            for col in row:
                node = len(left)
                column.append(col)
                up.append(up[col])
                down.append(col)
                down[up[col]] = node
                up[col] = node
                size[col] += 1
                left.append(node - 1)
                right.append(node + 1)
            left[first] = len(left) - 1
            right[-1] = first

    def cover(self, c: int) -> None:
        left, right, up, down, column, size = self.left, self.right, self.up, self.down, self.column, self.size
        left[right[c]] = left[c]
        right[left[c]] = right[c]
        i = down[c]
        while i != c:
            j = right[i]
            while j != i:
                up[down[j]] = up[j]
                down[up[j]] = down[j]
                size[column[j]] -= 1
                j = right[j]
            i = down[i]

    def uncover(self, c: int) -> None:
        left, right, up, down, column, size = self.left, self.right, self.up, self.down, self.column, self.size
        i = up[c]
        while i != c:
            j = left[i]
            while j != i:
                size[column[j]] += 1
                up[down[j]] = j
                down[up[j]] = j
                j = left[j]
            i = up[i]
        left[right[c]] = c
        right[left[c]] = c

    def place(self, row: int) -> None:
        j = self.right[row]
        while j != row:
            col = self.column[j]
            if col < self.cells:
                self.cover(col)
            else:
                self.copies[col - self.cells] -= 1
                if self.copies[col - self.cells] == 0:
                    self.cover(col)
            j = self.right[j]

    def unplace(self, row: int) -> None:
        j = self.left[row]
        while j != row:
            col = self.column[j]
            if col < self.cells:
                self.uncover(col)
            else:
                if self.copies[col - self.cells] == 0:
                    self.uncover(col)
                self.copies[col - self.cells] += 1
            j = self.left[j]

    def count(self) -> int:
        """The number of exact covers, choosing at each step the cell with the fewest options."""
        right, size, down, root = self.right, self.size, self.down, self.root
        best = right[root]
        if best == root:
            return 1
        c = right[best]
        while c != root and size[best] > 0:
            if size[c] < size[best]:
                best = c
            c = right[c]
        if size[best] == 0:
            return 0
        self.cover(best)
        total = 0
        row = down[best]
        while row != best:
            self.place(row)
            total += self.count()
            self.unplace(row)
            row = down[row]
        self.uncover(best)
        return total


def tilings(board: list[bytes], pieces: list[list[tuple[int, int]]]) -> int:
    index = {}
    for r, line in enumerate(board):
        for c, ch in enumerate(line):
            if ch == ord("."):
                index[r, c] = len(index)
    if sum(map(len, pieces)) != len(index):
        return 0

    copies: dict[Shape, int] = {}
    for piece in pieces:
        shape = orientations(piece)[0]
        copies[shape] = copies.get(shape, 0) + 1

    rows = []
    height, width = len(board), len(board[0])
    for kind, shape in enumerate(copies):
        for cells in orientations(list(shape)):
            for top in range(height):
                for left in range(width):
                    ids = [index.get((top + r, left + c)) for r, c in cells]
                    if None not in ids:
                        rows.append([*ids, len(index) + kind])

    found = Dlx(len(index), list(copies.values()), rows).count()
    if found == 0:
        return 0
    for k in copies.values():
        found *= factorial(k)
    return found


def main() -> None:
    sys.setrecursionlimit(10_000)
    lines = sys.stdin.buffer.read().split(b"\n")
    height, width = map(int, lines[0].split())
    board = lines[1 : 1 + height]
    at = 1 + height
    pieces = []
    for _ in range(int(lines[at])):
        rows, _ = map(int, lines[at + 1].split())
        grid = lines[at + 2 : at + 2 + rows]
        pieces.append([(r, c) for r, line in enumerate(grid) for c, ch in enumerate(line) if ch == ord("#")])
        at += 1 + rows
    sys.stdout.write(f"{tilings(board, pieces)}\n")


if __name__ == "__main__":
    main()
