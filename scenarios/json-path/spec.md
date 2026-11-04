# JSON path queries

Read a JSON document and a list of path queries, and print the values each query selects.

## Input

The input is read from standard input as bytes.

- The first line holds one integer `Q`, the number of queries, written in decimal with no sign and no leading zeros, followed by a line feed (byte 10).
- Each of the next `Q` lines holds one query, followed by a line feed. A query line holds only the printable ASCII bytes 32 through 126, and it may be empty.
- Every byte after the line feed that ends the last query is the document. When `Q` is 0, the document starts right after the first line. The document may hold any bytes, and it need not end with a line feed.

## The document

### Grammar

The document is valid when it is exactly one value, with optional whitespace before and after it. Whitespace is any run of the bytes space (32), tab (9), line feed (10) and carriage return (13). It may appear between any two tokens, but never inside one. The tokens are `{`, `}`, `[`, `]`, `,`, `:`, strings, integers, and the literals `true`, `false` and `null`.

A value is one of these:

- An object: `{`, then zero or more members separated by `,`, then `}`. A member is a string, which is its name, then `:`, then a value.
- An array: `[`, then zero or more values separated by `,`, then `]`. These values are its elements.
- A string.
- An integer.
- One of the literals `true`, `false` and `null`, in lowercase.

### Integers

An integer is an optional `-`, then either the single digit `0` or a digit from `1` to `9` followed by any number of digits. Its value must lie between -9223372036854775808 and 9223372036854775807, inclusive. There are no fractions, exponents, `+` signs or leading zeros. `-0` is a valid integer whose value is 0.

### Strings

A string is `"`, then any number of the following items, then `"`.

- A byte from 32 to 126 other than `"` (34) and `\` (92), or the byte 127. It stands for the ASCII character with that code.
- A character from U+0080 to U+10FFFF, other than the surrogates U+D800 to U+DFFF, encoded in UTF-8. The only valid encodings are the byte sequences in this table, where `80..BF` means any byte from 0x80 to 0xBF.

  | Characters | Byte 1 | Byte 2 | Byte 3 | Byte 4 |
  | --- | --- | --- | --- | --- |
  | U+0080 to U+07FF | C2..DF | 80..BF | | |
  | U+0800 to U+0FFF | E0 | A0..BF | 80..BF | |
  | U+1000 to U+CFFF | E1..EC | 80..BF | 80..BF | |
  | U+D000 to U+D7FF | ED | 80..9F | 80..BF | |
  | U+E000 to U+FFFF | EE..EF | 80..BF | 80..BF | |
  | U+10000 to U+3FFFF | F0 | 90..BF | 80..BF | 80..BF |
  | U+40000 to U+FFFFF | F1..F3 | 80..BF | 80..BF | 80..BF |
  | U+100000 to U+10FFFF | F4 | 80..8F | 80..BF | 80..BF |

- An escape: `\"`, `\\` and `\/` stand for `"`, `\` and `/`. `\b`, `\f`, `\n`, `\r` and `\t` stand for U+0008, U+000C, U+000A, U+000D and U+0009. `\u` followed by exactly four hexadecimal digits (`0` to `9`, `a` to `f`, `A` to `F`) stands for the 16-bit code unit with that value.

A byte from 0 to 31 is not an item, so it can appear in a string only as an escape. Any byte sequence in a string that is not one of the items above makes the document invalid, and so does any byte from 128 to 255 outside a string.

The value of a string is a sequence of 16-bit code units, as in UTF-16. A character from U+0000 to U+FFFF gives one code unit equal to its code point. A character `c` above U+FFFF gives two, the surrogate pair `0xD800 + ((c - 0x10000) >> 10)` and `0xDC00 + ((c - 0x10000) & 0x3FF)`. A `\u` escape gives exactly the code unit it names, whatever its value. So the escapes `\ud83d\ude00` give the same value as the character U+1F600 written in UTF-8 (the bytes F0 9F 98 80), and a string may hold a surrogate code unit that is not part of a pair, such as `"\udc00"` or `"\ud800x"`. Two strings are equal when their code unit sequences are equal, so `"\u00e9"`, `"\u00E9"` and the character é written in UTF-8 are the same string, and `"A"` and `"\u0041"` are the same string.

### Invalid documents

Anything that does not match the grammar makes the document invalid. Some examples: an empty document, a document of only whitespace, a second value after the first, a fraction such as `1.5`, an exponent such as `1e3`, a leading zero such as `01`, a `+` sign, an integer outside the range above, `NaN`, `Infinity`, comments, single-quoted strings, a name without quotes, a trailing or doubled comma, a missing `:`, an unclosed string, array or object, an unknown escape such as `\x` or `\U0041`, a `\u` with fewer than four hexadecimal digits, an unescaped byte from 0 to 31 in a string, a UTF-8 byte order mark (EF BB BF) before the value, and a byte sequence that is not valid UTF-8.

### Nodes and document order

Every value in the document is a node: the top-level value, called the root, each member value of each object, and each element of each array. Member names are not nodes. The descendants of a node are the values nested inside it at any depth: its elements or member values, their elements or member values, and so on. A node is not its own descendant.

Nodes are ordered by where they start in the document: a node comes before another when its first byte comes earlier. This is document order. A node comes before its descendants, and the members of an object and the elements of an array come in the order they are written.

An object may hold several members with the same name. All of them are kept, in the order they are written. Each is a node, a name selector picks every one of them, and printing the object prints every one of them.

## Queries

### Syntax

A query is `$` followed by zero or more segments, with nothing before, between or after them. There is no whitespace anywhere in a query except inside a string literal. Each segment has one of these six forms.

| Segment | Kind | Selector |
| --- | --- | --- |
| `.name` | child | the name `name` |
| `.*` | child | wildcard |
| `[selector]` | child | as written between the brackets |
| `..name` | descendant | the name `name` |
| `..*` | descendant | wildcard |
| `..[selector]` | descendant | as written between the brackets |

In the `.name` and `..name` forms, `name` is an ASCII letter (`A` to `Z`, `a` to `z`) or `_`, followed by any number of ASCII letters, digits and `_`. It is a name selector for the string made of those characters, so `.true` selects the member named `true`.

Between brackets, a selector is one of these:

- A string literal, which is a name selector. It has exactly the syntax of a string in the document: double quotes, and the same escapes. A query holds only printable ASCII, so other characters must be written as `\u` escapes. Examples: `["a b"]`, `[""]`, `["\""]`, `["\u00e9"]`.
- `*`, the wildcard.
- An integer, which is an index selector. It is written like an integer in the document, with the same range, so `-0` is allowed and means 0.
- A slice selector: `start:end` or `start:end:step`. Each of `start`, `end` and `step` is written like an integer in the document, with the same range, and each may be omitted, so `[:]`, `[2:]`, `[:-1]`, `[::2]` and `[1:5:]` are all slices. A `step`, when present, must be at least 1.

Every other line is an invalid query. Some examples: an empty line, a line that does not start with `$`, a space anywhere outside a string literal, a single-quoted string such as `['a']`, a `.` followed by `[` as in `$.[0]`, a lone `.` or `..` at the end, `...`, an empty bracket `[]`, an unclosed bracket, a leading zero such as `[01]`, a `+` sign, a fraction, a name that starts with a digit such as `.1a`, a name with any character other than letters, digits and `_`, a string literal with an invalid escape, an integer outside the range of document integers, a step of 0, a negative step, and more than two `:` in a slice.

### Meaning

The result of a query is a set of nodes. It starts as the set that holds only the root. Each segment in turn replaces the set with a new one:

- A child segment applies its selector to each node in the set. The new set holds every node those applications pick.
- A descendant segment applies its selector to each node in the set and to every descendant of each node in the set. The new set holds every node those applications pick.

The set after the last segment is the result. A set holds each node at most once, however many ways the node was picked. Two different nodes with equal values are still two nodes.

Applying a selector to a node `N` picks these nodes:

- A name selector for the string `s`: when `N` is an object, the value of every member of `N` whose name equals `s`. Otherwise nothing.
- The wildcard: when `N` is an object, the values of all its members. When `N` is an array, all its elements. Otherwise nothing.
- An index selector `i`, when `N` is an array of length `n`: the element at position `i` when `0 <= i < n`, or the element at position `n + i` when `-n <= i < 0`. Positions count from 0. Otherwise, or when `N` is not an array, nothing.
- A slice selector, when `N` is an array of length `n`: a missing `step` is 1. A missing `start` is 0 and a missing `end` is `n`. A negative `start` or `end` has `n` added to it, and then a bound below 0 becomes 0 and a bound above `n` becomes `n`. The slice picks the elements at positions `start`, `start + step`, `start + 2 * step`, and so on, as long as the position is less than `end`. It picks nothing when `start >= end`, or when `N` is not an array.

So `$..*` selects every node except the root, `$..a` selects every value of a member named `a` anywhere in the document, the root's own members included, and `$..[0]` selects the first element of every array.

Indexes, slice bounds and steps span the full signed 64-bit range, so a computation such as `start + step` or `n + i` can overflow a 64-bit integer.

## Output

When the document is invalid, print the line `invalid document` and a line feed, and nothing else. The queries are not examined.

Otherwise, print one block per query, in input order:

- For an invalid query, the line `invalid query`.
- For a valid query, a line holding the number of nodes in its result, `R`, in decimal. Then `R` lines, one per node of the result, in document order. Each line holds the node's value printed as below. A query with an empty result prints the single line `0`.

Every line ends with a line feed.

### Printing a value

A value is printed as compact JSON with no whitespace at all:

- `true`, `false` and `null` print as themselves.
- An integer prints in decimal, with a `-` only when it is negative and no leading zeros, so `-0` prints as `0`.
- A string prints as `"`, then each of its code units in order, then `"`. A code unit prints as follows.

  | Code unit | Printed as |
  | --- | --- |
  | 0x22 (`"`) | `\"` |
  | 0x5C (`\`) | `\\` |
  | 0x08 | `\b` |
  | 0x0C | `\f` |
  | 0x0A | `\n` |
  | 0x0D | `\r` |
  | 0x09 | `\t` |
  | any other unit from 0x20 to 0x7E | the ASCII character itself |
  | any other unit | `\u` and its value as four lowercase hexadecimal digits |

  So `/` prints as `/`, the byte 127 as `\u007f`, é as `\u00e9`, U+1F600 as `\ud83d\ude00`, and a lone surrogate as `\udc00`. The output is pure ASCII.

- An array prints as `[`, its elements separated by `,`, then `]`. An empty array prints as `[]`.
- An object prints as `{`, its members in the order they are written separated by `,`, then `}`, with every member of a repeated name included. A member prints as its name printed as a string, then `:`, then its value. An empty object prints as `{}`.

## Constraints

- `0 <= Q <= 100000`
- A query line is at most 1000 bytes long.
- The document is at most 100000000 bytes long.
- Arrays and objects nest at most 1000 deep. In every document, valid or not, at most 1000 of the `[` and `{` that come before any point and lie outside strings are still unclosed at that point.

Some documents are tens of megabytes long and come with thousands of queries.

## Examples

Input:

```text
8
$.store.book[*].title
$..price
$.store.book[-1]
$.store.book[:2].author
$["store"]["bicycle"]
$.store.book[5]
$.store.book[0]..*
$.store..[0]
{"store": {"book": [{"title": "Sayings", "price": 8, "author": "Rees"},
                    {"title": "Moby Dick", "price": 9, "author": "Melville"},
                    {"title": "Lord", "price": -0, "author": "Tolkien"}],
           "bicycle": {"color": "red", "price": 19}},
 "price": 1}
```

Output:

```text
3
"Sayings"
"Moby Dick"
"Lord"
5
8
9
0
19
1
1
{"title":"Lord","price":0,"author":"Tolkien"}
2
"Rees"
"Melville"
1
{"color":"red","price":19}
0
3
"Sayings"
8
"Rees"
1
{"title":"Sayings","price":8,"author":"Rees"}
```

The root's own `price` comes last in the document, so it is printed last. `$.store.book[5]` selects nothing and prints `0`.

Input:

```text
6
$..a
$..a..a
$.k
$["caf\u00e9"]
$.s
$.t
{"x": {"a": 1}, "a": {"a": {"a": 2}},
 "k": "first", "k": "second",
 "café": [], "s": "tab\there \/ \u00E9\ud83d\ude00\udc00", "t": "\u007f"}
```

Output:

```text
4
1
{"a":{"a":2}}
{"a":2}
2
2
{"a":2}
2
2
"first"
"second"
1
[]
1
"tab\there / \u00e9\ud83d\ude00\udc00"
1
"\u007f"
```

`$..a` lists `1` first because it starts earliest in the document, even though it lies deeper than the root's own member `a`. In `$..a..a`, the node `2` is a descendant of both `{"a":{"a":2}}` and `{"a":2}`, but a result holds each node once. The name `café` is written in UTF-8 in the document and as `\u00e9` in the query, and the two are equal.
