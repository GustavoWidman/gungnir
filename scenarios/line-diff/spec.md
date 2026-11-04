# Line diff

Compare pairs of text documents line by line. For each pair, find the fewest line deletions and insertions that turn the first document into the second, and print one particular shortest edit script, fixed by the rule below, as a list of hunks.

## Input

The input is read from standard input as bytes.

- The first line holds one integer `T`, the number of document pairs, followed by a line feed (byte 10).
- Then come the `T` pairs, one after another. A pair starts with a line that holds two integers `n` and `m`, separated by one space and followed by a line feed. The `n` bytes right after that line feed are the first document, and the `m` bytes right after those are the second document. The next pair's `n m` line starts right after the second document's last byte.

Integers are written in decimal with no sign and no leading zeros. Nothing separates a document from what follows it. A document that ends with a line feed counts that line feed in its length, and a document that does not end with one is followed directly by the next document or by the next pair's `n m` line. The input ends right after the last pair's second document.

Documents contain only these bytes: tab (9), line feed (10), carriage return (13), and the printable ASCII characters 32 through 126.

## Lines

A document is a sequence of lines. Every line ends right after a line feed, except the document's last line, which ends at the end of the document and so has no line feed when the document does not end with one. A document of 0 bytes has no lines, and a final line feed does not start another, empty line.

Some documents and their lines, with `\n` standing for a line feed and `\r` for a carriage return:

- `a\nb\n` has two lines, `a\n` and `b\n`.
- `a\nb` has two lines, `a\n` and `b`.
- `\n\n` has two lines, each a lone line feed.
- `a\r\nb\rc` has two lines, `a\r\n` and `b\rc`. A carriage return is an ordinary byte, never a line break.

Two lines are equal when they consist of the same bytes, the line feed included. So `b` differs from `b\n`, `a\r\n` differs from `a\n`, and `a \n` differs from `a\n`.

Below, `A` is the sequence of the first document's lines and `N` their number, and `B` and `M` are the same for the second document.

## Edit scripts

An edit script turns `A` into `B` by walking through both from the start. It is a sequence of operations, each one of these three:

- keep: the next line of `A` and the next line of `B` are equal, and both are used up;
- delete: the next line of `A` is used up;
- insert: the next line of `B` is used up.

A script is complete when it has used up every line of both. Its cost is its number of deletes and inserts, and a shortest script is a complete script of the least cost for that pair. Let `L` be the length of a longest sequence of lines that appears in both `A` and `B` in the same order, not necessarily contiguously. Every shortest script keeps `L` lines, deletes `N - L` and inserts `M - L`.

## The canonical script

A pair usually has many shortest scripts. Print the canonical one: the shortest script that comes first when shortest scripts are compared operation by operation from the start, with keep before delete before insert. At the first position where two shortest scripts differ, the one with a keep there comes first, and when neither has a keep there, the one with a delete there comes first. All shortest scripts of a pair have the same number of operations, `N + M - L`, so exactly one comes first.

The canonical script can also be built one operation at a time from the start, choosing each operation like this:

1. When both documents have a next line and those two lines are equal, keep. Keeping two equal lines never raises the least cost of finishing.
2. Otherwise, delete when some shortest script begins with the operations chosen so far followed by a delete.
3. Otherwise, insert.

## Output

A hunk is a maximal run of consecutive deletes and inserts in the canonical script: a keep or an end of the script comes right before it and right after it. In the canonical script every hunk holds all its deletes before all its inserts.

For each pair, in input order, write:

- one line `d i h`, where `d = N - L` is the number of deleted lines, `i = M - L` is the number of inserted lines, and `h` is the number of hunks;
- then `h` lines, one per hunk in script order, each `x a y b`, where `x` is the number of lines of `A` before the hunk, `a` is the number of lines the hunk deletes, `y` is the number of lines of `B` before the hunk, and `b` is the number of lines it inserts.

Counting lines from 1, a hunk `x a y b` deletes lines `x + 1` through `x + a` of `A` and inserts lines `y + 1` through `y + b` of `B`. Either `a` or `b` can be 0, but not both.

Integers are written in decimal, the integers on a line are separated by one space, and every line ends with a line feed. Two documents with the same lines give the single line `0 0 0`. When `T` is 0, write nothing.

## Constraints

- `0 <= T <= 10000`
- The input is at most 50000000 bytes long.
- For every pair, `d + i <= 5000`.

Some inputs hold documents of tens of thousands of lines that differ in a few thousand places. A solution that compares every line of the first document with every line of the second is far too slow for them and needs far too much memory.

## Examples

Input:

```text
2
8 10
a
b
c
d
a
x
c
d
e
4 6
a
b
a
a
b
```

Output:

```text
1 2 2
1 1 1 1
4 0 4 1
0 1 1
1 0 1 1
```

The first pair turns `a b c d` into `a x c d e`. Its first hunk, `1 1 1 1`, deletes line 2 of the first document and inserts line 2 of the second. Its second hunk, `4 0 4 1`, inserts line 5 of the second document after all four lines of the first. The second pair turns `a b` into `a a b`, which takes one insert. Both "insert, keep, keep" and "keep, insert, keep" are shortest scripts, and the canonical one keeps first, so the hunk is `1 0 1 1` and not `0 0 0 1`.

Input:

```text
3
7 8
one
twoone
two
4 4
x
a
a
x
0 0
```

Output:

```text
1 1 1
1 1 1 1
1 1 2
0 1 0 0
2 0 1 1
0 0 0
```

The first document of the first pair is the 7 bytes `one\ntwo`, and the second is the 8 bytes `one\ntwo\n`. Their last lines `two` and `two\n` differ, so line 2 is deleted and line 2 inserted. The second pair turns `x a` into `a x`. Its shortest scripts "delete, keep, insert" and "insert, keep, delete" first differ at their first operation, where a delete comes before an insert, so the first one is canonical. Its hunks delete `x` before the kept `a` and insert `x` after it. The third pair has two empty documents.
