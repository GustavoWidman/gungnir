import re
import sys
from collections import Counter

WORD = re.compile(rb"[a-z]+")


def main() -> None:
    data = sys.stdin.buffer.read()
    first, _, text = data.partition(b"\n")
    k = int(first)
    counts = Counter(WORD.findall(text.lower()))
    ranked = sorted(counts.items(), key=lambda item: (-item[1], item[0]))
    sys.stdout.buffer.write(b"".join(b"%s %d\n" % (word, count) for word, count in ranked[:k]))


if __name__ == "__main__":
    main()
