import gc
import json
import re
import sys
from json.encoder import encode_basestring_ascii

LIMIT = 1 << 63
INT = r"-?(?:0|[1-9][0-9]*)"
NAME = re.compile(r"[A-Za-z_][A-Za-z0-9_]*")
STRING = re.compile(r'"(?:[^"\\]|\\.)*"(?=\])')
INDEX = re.compile(rf"({INT})(?=\])")
SLICE = re.compile(rf"({INT})?:({INT})?(?::({INT})?)?(?=\])")
WILD = ("wild",)


class Obj(tuple):
    """An object: its (name, value) members in input order, duplicate names included."""

    __slots__ = ()


class Invalid(Exception):
    pass


def reject(_text):
    raise Invalid


def checked_int(text):
    value = int(text)
    if not -LIMIT <= value < LIMIT:
        raise Invalid
    return value


DECODER = json.JSONDecoder(
    object_pairs_hook=Obj, parse_float=reject, parse_int=checked_int, parse_constant=reject
)


def parse_document(data):
    """Returns (True, root), or (False, None) for an invalid document."""
    try:
        return True, DECODER.decode(data.decode("utf-8"))
    except (Invalid, ValueError):
        return False, None


def parse_bracket(text, i):
    """Parses the selector of the bracket that opens at text[i]; returns (selector, end) or None."""
    i += 1
    if text.startswith("*]", i):
        return WILD, i + 2
    if m := STRING.match(text, i):
        try:
            name = json.loads(m.group())
        except ValueError:
            return None
        return ("name", name), m.end() + 1
    if m := INDEX.match(text, i):
        value = int(m.group(1))
        return (("index", value), m.end() + 1) if -LIMIT <= value < LIMIT else None
    if m := SLICE.match(text, i):
        bounds = [None if g is None else int(g) for g in m.groups()]
        if any(b is not None and not -LIMIT <= b < LIMIT for b in bounds):
            return None
        start, end, step = bounds
        if step is not None and step < 1:
            return None
        return ("slice", start, end, step or 1), m.end() + 1
    return None


def parse_query(text):
    """Returns the query's segments as (descendant, selector) pairs, or None if it is invalid."""
    if not text.startswith("$"):
        return None
    segments = []
    i = 1
    while i < len(text):
        if text.startswith("..", i):
            descendant, dotted, i = True, True, i + 2
        elif text[i] == ".":
            descendant, dotted, i = False, True, i + 1
        else:
            descendant, dotted = False, False
        if text.startswith("[", i) and (descendant or not dotted):
            parsed = parse_bracket(text, i)
            if parsed is None:
                return None
            selector, i = parsed
        elif dotted and text.startswith("*", i):
            selector, i = WILD, i + 1
        elif dotted and (m := NAME.match(text, i)):
            selector, i = ("name", m.group()), m.end()
        else:
            return None
        segments.append((descendant, selector))
    return segments


def positions(selector, length):
    """The array positions an index or slice selector picks, as a range."""
    if selector[0] == "index":
        i = selector[1] + length if selector[1] < 0 else selector[1]
        return range(i, i + 1) if 0 <= i < length else range(0)
    _, start, end, step = selector
    start = 0 if start is None else max(0, start + length) if start < 0 else min(start, length)
    end = length if end is None else max(0, end + length) if end < 0 else min(end, length)
    return range(start, end, step)


def children(value, path, selector, out):
    """Appends the children of value that selector picks, with their paths, in document order."""
    kind = selector[0]
    if type(value) is Obj:
        if kind == "name":
            name = selector[1]
            for j, (k, v) in enumerate(value):
                if k == name:
                    out.append((v, path + (j,)))
        elif kind == "wild":
            for j, (_, v) in enumerate(value):
                out.append((v, path + (j,)))
    elif type(value) is list:
        if kind == "wild":
            for j, v in enumerate(value):
                out.append((v, path + (j,)))
        elif kind != "name":
            for j in positions(selector, len(value)):
                out.append((value[j], path + (j,)))


def descend_name(value, path, name, out):
    if type(value) is Obj:
        for j, (k, v) in enumerate(value):
            if k == name:
                out.append((v, path + (j,)))
            if type(v) is Obj or type(v) is list:
                descend_name(v, path + (j,), name, out)
    else:
        for j, v in enumerate(value):
            if type(v) is Obj or type(v) is list:
                descend_name(v, path + (j,), name, out)


def descend_all(value, path, out):
    items = enumerate(value) if type(value) is list else ((j, v) for j, (_, v) in enumerate(value))
    for j, v in items:
        out.append((v, path + (j,)))
        if type(v) is Obj or type(v) is list:
            descend_all(v, path + (j,), out)


def descend_positions(value, path, selector, out):
    if type(value) is Obj:
        items, picked = ((j, v) for j, (_, v) in enumerate(value)), range(0)
    else:
        items, picked = enumerate(value), positions(selector, len(value))
    for j, v in items:
        if j in picked:
            out.append((v, path + (j,)))
        if type(v) is Obj or type(v) is list:
            descend_positions(v, path + (j,), selector, out)


def descend(value, path, selector, out):
    """Appends what selector picks from value and from each of its descendants, in document order."""
    if type(value) is not Obj and type(value) is not list:
        return
    if selector[0] == "name":
        descend_name(value, path, selector[1], out)
    elif selector[0] == "wild":
        descend_all(value, path, out)
    else:
        descend_positions(value, path, selector, out)


def outermost(nodes):
    """Drops each node that lies inside an earlier one; nodes are in document order."""
    kept = []
    for node in nodes:
        if not kept or node[1][: len(kept[-1][1])] != kept[-1][1]:
            kept.append(node)
    return kept


def evaluate(root, segments):
    """The selected nodes as (value, path) pairs, each once, in document order.

    A path lists child positions from the root, so paths sort in document order. A set stays
    free of nested nodes until a descendant segment, and only a nested set needs sorting.
    """
    nodes, nested = [(root, ())], False
    for descendant, selector in segments:
        out = []
        if descendant:
            for value, path in outermost(nodes) if nested else nodes:
                descend(value, path, selector, out)
            nested = True
        else:
            for value, path in nodes:
                children(value, path, selector, out)
            if nested:
                out.sort(key=lambda node: node[1])
        nodes = out
    return nodes


def dump(value, out):
    t = type(value)
    if t is str:
        out.append(encode_basestring_ascii(value))
    elif t is int:
        out.append(str(value))
    elif t is list:
        out.append("[")
        for j, v in enumerate(value):
            if j:
                out.append(",")
            dump(v, out)
        out.append("]")
    elif t is Obj:
        out.append("{")
        for j, (k, v) in enumerate(value):
            out.append("," + encode_basestring_ascii(k) + ":" if j else encode_basestring_ascii(k) + ":")
            dump(v, out)
        out.append("}")
    else:
        out.append("null" if value is None else "true" if value else "false")


def main() -> None:
    # The parse allocates millions of containers and never makes a cycle, so cyclic garbage
    # collection would only rescan them, which more than doubles the parse time.
    gc.disable()
    sys.setrecursionlimit(20_000)
    data = sys.stdin.buffer.read()
    count, _, rest = data.partition(b"\n")
    *queries, document = rest.split(b"\n", int(count))
    valid, root = parse_document(document)
    if not valid:
        sys.stdout.write("invalid document\n")
        return
    out = []
    for query in queries:
        segments = parse_query(query.decode("ascii"))
        if segments is None:
            out.append("invalid query\n")
            continue
        nodes = evaluate(root, segments)
        out.append(f"{len(nodes)}\n")
        for value, _ in nodes:
            dump(value, out)
            out.append("\n")
    sys.stdout.write("".join(out))


if __name__ == "__main__":
    main()
