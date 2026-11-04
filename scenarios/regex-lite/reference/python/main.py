import sys

ESCAPABLE = frozenset(b"\\.*+?|()[]^$-")
CHAR, EPS, BEGIN, END, MATCH = range(5)
ALL = (1 << 128) - 1
# A text line has at most 100000 characters, so one line adds at most about 200000 DFA states.
STATE_LIMIT = 10000


class Invalid(Exception):
    pass


class Parser:
    """Builds a Thompson NFA while parsing. Every fragment is an (entry, exit) pair whose exit is
    an EPS node that later gets the edges to whatever follows."""

    def __init__(self, pattern: bytes):
        self.pattern = pattern
        self.pos = 0
        self.kind: list[int] = []
        self.chars: list[int] = []
        self.out: list[list[int]] = []

    def node(self, kind: int, chars: int = 0) -> int:
        self.kind.append(kind)
        self.chars.append(chars)
        self.out.append([])
        return len(self.kind) - 1

    def peek(self) -> int:
        return self.pattern[self.pos] if self.pos < len(self.pattern) else -1

    def take(self) -> int:
        c = self.peek()
        if c < 0:
            raise Invalid
        self.pos += 1
        return c

    def parse(self) -> int:
        """Parses the whole pattern and returns the start node. The accepting node comes last."""
        start, exit = self.alternation()
        if self.pos != len(self.pattern):
            raise Invalid
        self.out[exit].append(self.node(MATCH))
        return start

    def alternation(self) -> tuple[int, int]:
        entry, exit = self.node(EPS), self.node(EPS)
        while True:
            first, last = self.sequence()
            self.out[entry].append(first)
            self.out[last].append(exit)
            if self.peek() != ord("|"):
                return entry, exit
            self.pos += 1

    def sequence(self) -> tuple[int, int]:
        entry = last = self.node(EPS)
        while self.peek() not in (-1, ord("|"), ord(")")):
            first, end = self.item()
            self.out[last].append(first)
            last = end
        return entry, last

    def item(self) -> tuple[int, int]:
        first, last = self.atom()
        q = self.peek()
        if q == ord("*"):
            self.pos += 1
            loop = self.node(EPS)
            self.out[loop].append(first)
            self.out[last].append(loop)
            return loop, loop
        if q == ord("+"):
            self.pos += 1
            self.out[last].append(first)
            return first, last
        if q == ord("?"):
            self.pos += 1
            skip = self.node(EPS)
            self.out[skip] += [first, last]
            return skip, last
        return first, last

    def atom(self) -> tuple[int, int]:
        c = self.take()
        if c == ord("("):
            fragment = self.alternation()
            if self.take() != ord(")"):
                raise Invalid
            return fragment
        if c in b"*+?":
            raise Invalid
        if c == ord("^"):
            return self.wrap(self.node(BEGIN))
        if c == ord("$"):
            return self.wrap(self.node(END))
        if c == ord("."):
            chars = ALL
        elif c == ord("["):
            chars = self.bracket()
        elif c == ord("\\"):
            chars = 1 << self.escaped()
        else:
            chars = 1 << c
        return self.wrap(self.node(CHAR, chars))

    def wrap(self, node: int) -> tuple[int, int]:
        exit = self.node(EPS)
        self.out[node].append(exit)
        return node, exit

    def escaped(self) -> int:
        c = self.take()
        if c not in ESCAPABLE:
            raise Invalid
        return c

    def bracket(self) -> int:
        negated = self.peek() == ord("^")
        if negated:
            self.pos += 1
        # Each body character as (byte, is an unescaped dash).
        body: list[tuple[int, bool]] = []
        while (c := self.take()) != ord("]"):
            if c == ord("\\"):
                body.append((self.escaped(), False))
            else:
                body.append((c, c == ord("-")))
        if not body:
            raise Invalid
        chars, start, stop = 0, 0, len(body)
        if body[0][1]:
            chars |= 1 << ord("-")
            start = 1
        if stop > start and body[-1][1]:
            chars |= 1 << ord("-")
            stop -= 1
        i = start
        while i < stop:
            low, dash = body[i]
            if dash:
                raise Invalid
            if i + 1 < stop and body[i + 1][1]:
                if i + 2 >= stop or body[i + 2][1]:
                    raise Invalid
                high = body[i + 2][0]
                if low > high:
                    raise Invalid
                chars |= ((1 << (high + 1)) - 1) ^ ((1 << low) - 1)
                i += 3
            else:
                chars |= 1 << low
                i += 1
        return chars ^ ALL if negated else chars


class Matcher:
    """Counts leftmost-longest matches in linear time per line.

    The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
    node that consumes a character. A reverse pass records at each position which kernel states
    can still reach acceptance from there. Forward scans start only where a match starts and keep
    only states that can still accept, so each scan stops right after its longest match.
    """

    def __init__(self, parser: Parser, start: int):
        kind, out = parser.kind, parser.out
        kernel = {len(kind) - 1: 0}
        for node, k in enumerate(kind):
            if k == CHAR:
                kernel[node] = len(kernel)

        def closure(roots: list[int], begin: bool, end: bool) -> int:
            bits, seen, stack = 0, set(roots), list(roots)
            while stack:
                node = stack.pop()
                k = kind[node]
                if k == CHAR or k == MATCH:
                    bits |= 1 << kernel[node]
                    continue
                if (k == BEGIN and not begin) or (k == END and not end):
                    continue
                for nxt in out[node]:
                    if nxt not in seen:
                        seen.add(nxt)
                        stack.append(nxt)
            return bits

        self.follow = [0] * len(kernel)
        follow_end = [0] * len(kernel)
        self.consumers: list[list[int]] = [[] for _ in range(128)]
        for node, k in kernel.items():
            if k == 0:
                continue
            self.follow[k] = closure(out[node], False, False)
            follow_end[k] = closure(out[node], False, True)
            chars = parser.chars[node]
            for c in range(32, 127):
                if chars >> c & 1:
                    self.consumers[c].append(k)
        self.start = {(b, e): closure([start], b, e) for b in (False, True) for e in (False, True)}
        self.last = [1 | sum(1 << k for k in self.consumers[c] if follow_end[k] & 1) for c in range(128)]
        self.reset()

    def reset(self):
        self.ids: dict[int, int] = {}
        self.bits: list[int] = []
        self.rows: list[list[int]] = []
        self.starts: list[bool] = []
        self.union: dict[int, int] = {}
        self.accept_only = self.intern(1)
        self.last_ids = [self.intern(bits) for bits in self.last]

    def intern(self, bits: int) -> int:
        state = self.ids.get(bits)
        if state is None:
            state = self.ids[bits] = len(self.bits)
            self.bits.append(bits)
            self.rows.append([-1] * 128)
            self.starts.append(bool(self.start[False, False] & bits))
        return state

    def step_back(self, state: int, c: int) -> int:
        """The kernel states that can accept from a position holding c, given those that can
        accept from the next position."""
        ahead = self.bits[state]
        follow = self.follow
        bits = 1
        for k in self.consumers[c]:
            if follow[k] & ahead:
                bits |= 1 << k
        nxt = self.rows[state][c] = self.intern(bits)
        return nxt

    def advance(self, bits: int) -> int:
        """Every kernel state that follows one in bits after it consumes a character. The
        character is implied: every state in a pruned set accepts the character at its position."""
        nxt = 0
        rest = bits >> 1
        k = 1
        follow = self.follow
        while rest:
            if rest & 1:
                nxt |= follow[k]
            rest >>= 1
            k += 1
        self.union[bits] = nxt
        return nxt

    def count(self, line: bytes) -> int:
        if len(self.bits) + len(self.union) > STATE_LIMIT:
            self.reset()
        n = len(line)
        alive = [self.accept_only] * (n + 1)
        if n:
            state = alive[n - 1] = self.last_ids[line[n - 1]]
            rows = self.rows
            for i in range(n - 2, -1, -1):
                c = line[i]
                nxt = rows[state][c]
                if nxt < 0:
                    nxt = self.step_back(state, c)
                alive[i] = state = nxt

        bits, starts, union = self.bits, self.starts, self.union
        count = p = 0
        while p <= n:
            s = p
            while 0 < s < n and not starts[alive[s]]:
                s += 1
            current = self.start[s == 0, s == n] & bits[alive[s]]
            if not current:
                if s >= n:
                    break
                p = s + 1
                continue
            end = s if current & 1 else -1
            i = s
            while current > 1:
                i += 1
                if i == n:
                    current = 1
                else:
                    nxt = union.get(current)
                    if nxt is None:
                        nxt = self.advance(current)
                    current = nxt & bits[alive[i]]
                if current & 1:
                    end = i
            count += 1
            p = end if end > s else s + 1
        return count


def main() -> None:
    sys.setrecursionlimit(10000)
    data = sys.stdin.buffer.read()
    pattern, _, text = data.partition(b"\n")
    lines = text.split(b"\n")[:-1]
    parser = Parser(pattern)
    try:
        start = parser.parse()
    except Invalid:
        sys.stdout.buffer.write(b"invalid\n")
        return
    matcher = Matcher(parser, start)
    sys.stdout.buffer.write(b"".join(b"%d\n" % matcher.count(line) for line in lines))


if __name__ == "__main__":
    main()
