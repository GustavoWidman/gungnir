# Regex lite

Read a pattern written in a small regular-expression language. Then, for each line of text, count the non-overlapping matches of the pattern that a left-to-right scan of the line finds.

## Input

The input is read from standard input as bytes. Every line, including the last, ends with a line feed (byte 10).

- The first line is the pattern: every byte before the first line feed. The pattern may be empty, and every byte of it counts, spaces included.
- Each following line is one text line. There may be no text lines, and a text line may be empty.

The pattern and the text lines contain only the printable ASCII characters, bytes 32 (space) through 126 (`~`).

## Pattern syntax

Outside a bracket expression, these eleven characters are metacharacters:

```text
\ . * + ? | ( ) [ ^ $
```

Every other character, including `]`, `{`, `}`, `-` and space, is a literal that matches itself. Matching is case-sensitive.

These thirteen characters can be escaped, inside a bracket expression and outside one:

```text
\ . * + ? | ( ) [ ] ^ $ -
```

A pattern is built from these pieces, where `X` and `Y` stand for smaller patterns:

- `c`, a literal character, matches the character `c`.
- `\c`, where `c` is one of the thirteen escapable characters, matches the character `c`.
- `.` matches any one character.
- `[set]` matches any one character in the set, and `[^set]` matches any one character not in it. See "Bracket expressions".
- `^` matches the start of the line, and `$` matches the end of the line. Neither consumes a character.
- `(X)` is a group. It matches whatever `X` matches.
- `X*` matches `X` zero or more times, `X+` one or more times, and `X?` zero times or once.
- `XY` matches `X` followed by `Y`.
- `X|Y` matches `X` or `Y`.

The grammar below defines every valid pattern. Quantifiers bind tightest, then concatenation, then alternation, so `ab*|c` means `(a(b*))|c`.

```text
pattern     := alternation
alternation := sequence ( "|" sequence )*
sequence    := item*
item        := atom quantifier?
quantifier  := "*" | "+" | "?"
atom        := literal | escape | "." | "^" | "$" | bracket | "(" alternation ")"
```

- A sequence may be empty, and an empty sequence matches the empty string. So the empty pattern, `()`, `a|` and `(|b)c` are valid.
- A quantifier applies to the single atom right before it. `^` and `$` are atoms, so `^*` and `$?` are valid.
- An atom takes at most one quantifier. `a**`, `a+?` and `(a)?*` are invalid, and there are no lazy or possessive quantifiers.
- Braces are literals, so `a{2}` matches the four characters `a{2}`. There are no counted repetitions, no backreferences, and no shorthand classes such as `\d` or `\w`.

## Bracket expressions

A bracket expression starts with `[`. If the next character is `^`, the expression is negated and that `^` is not part of the set. The body follows and runs up to the first `]` that is not escaped, which closes the expression.

In the body, a backslash starts an escape exactly as outside, with the same thirteen characters. An unescaped `-` has the special meaning described next. Every other character stands for itself, including `[`, `.`, `*`, `(`, `|`, `$`, space, and a `^` that does not come right after the opening `[`.

The body is a list of elements. An element is a single character or a range `x-y`: two characters joined by an unescaped `-`, which stands for every character whose byte value lies between those of `x` and `y`, both included. The grammar of a body is:

```text
body     := "-"? element* "-"?        (at least one character in total)
element  := endpoint ( "-" endpoint )?
endpoint := escape | any character except "\", "]" and "-"
```

So an unescaped `-` stands for itself only as the first or the last character of the body. Anywhere else it must join two endpoints. An unescaped `-` is never an endpoint, and an endpoint belongs to at most one range. To put a `-` elsewhere, write `\-`. To put a `]` in a set, write `\]`.

A plain bracket expression matches one character that is in its set. A negated one matches one character that is not in its set. Some examples:

- `[abc]` is the set of `a`, `b` and `c`.
- `[a-cx]` is the set of `a`, `b`, `c` and `x`.
- `[^0-9]` matches every character except the ten digits.
- `[-a]` and `[a-]` are both the set of `-` and `a`.
- `[a^]` is the set of `a` and `^`.
- `[\]\\]` is the set of `]` and `\`.
- `[ -~]` is the set of every printable character.

## Invalid patterns

A pattern is invalid exactly when it does not follow the grammars above. These are all the ways that can happen:

- A `(` has no matching `)`, or a `)` has no matching `(`.
- A quantifier comes first in the pattern, right after `(` or `|`, or right after another quantifier, as in `*a`, `(+a)`, `a|?` or `a*+`.
- A `\` is the last character of the pattern, or is followed by a character that is not one of the thirteen escapable characters, as in `\d`, `\n` or `\{`.
- A `[` has no closing `]`.
- A bracket expression has an empty body: `[]` or `[^]`. Since the first unescaped `]` closes the expression, `[]a]` is the invalid `[]` followed by `a]`.
- A range runs backwards, its first endpoint having a greater byte value than its second, as in `[z-a]`. A range of one character, such as `[a-a]`, is valid.
- An unescaped `-` in a body is neither its first nor its last character and does not join two endpoints, as in `[a-c-e]`, `[--a]` or `[a--]`.

## Matches

Number the positions of a text line of `n` characters from `0` to `n`. Position `i` lies just before the character with index `i`, counting from 0, and position `n` is the end of the line. A span `(s, e)` with `s <= e` covers the characters from index `s` through index `e - 1`. It is empty when `s = e`.

A pattern matches a set of spans of the line:

- A literal, an escape, `.` or a bracket expression matches `(i, i + 1)` whenever it matches the character at index `i`.
- `^` matches only the span `(0, 0)`. `$` matches only the span `(n, n)`.
- An empty sequence matches `(i, i)` for every position `i`.
- `XY` matches `(i, k)` when `X` matches `(i, j)` and `Y` matches `(j, k)` for some position `j`.
- `X|Y` matches every span that `X` matches and every span that `Y` matches. `(X)` matches the spans `X` matches.
- `X+` matches `(i0, ik)` when `X` matches every one of `(i0, i1)`, `(i1, i2)`, ..., `(ik-1, ik)` for some `k >= 1`.
- `X*` matches what `X+` matches and also `(i, i)` for every position `i`. `X?` matches what `X` matches and also `(i, i)` for every position `i`.

Spans are always measured on the whole line. In particular `^` matches only at the true start of the line, however far the scan below has advanced.

## Counting

Each text line is scanned from left to right with this procedure:

```text
count = 0
p = 0
while p <= n:
    if the pattern matches no span (s, e) with s >= p: stop
    s = the smallest start s >= p of a span the pattern matches
    e = the largest end of a span the pattern matches that starts at s
    count = count + 1
    if e > s: p = e
    else: p = s + 1
```

The scan takes the match that starts earliest and, among those, the longest. The order of the alternatives does not matter, so `a|ab` and `ab|a` give the same counts. A match may be empty. After an empty match the scan moves one position on, so no empty match is counted twice. An empty match that starts where the previous match ended is counted.

## Output

If the pattern is invalid, write exactly one line, `invalid`, whatever the text lines are.

Otherwise write one line per text line, in input order. Each line holds that text line's count in decimal. When there are no text lines, write nothing.

Every output line ends with a line feed. In every case, including an invalid pattern, the program exits with status 0.

## Constraints

- The pattern is at most 1000 characters long.
- There are at most 1000000 text lines, and each is at most 100000 characters long.
- The whole input is at most 50000000 bytes.

Some inputs are several megabytes long. In some patterns a part can match the same text in exponentially many ways, such as `(a|aa)*` in `(a|aa)*b` against a long run of `a`. A matcher that tries those ways one at a time is far too slow for them.

## Examples

Input:

```text
a[0-9]*|ab
ab a12 ba
xyz
a7a7
```

Output:

```text
3
0
2
```

In `ab a12 ba`, the first match starts at position 0, where `a[0-9]*` matches `a` and `ab` matches `ab`. The longer one wins, so the scan counts `ab` and moves on to position 2. It then counts `a12` and the final `a`. In `a7a7` the matches are `a7` and `a7`.

Input:

```text
b*
abba

bbb
```

Output:

```text
4
1
2
```

In `abba` the matches are the empty span at position 0, `bb`, the empty span at position 3, and the empty span at position 4. The empty line has one empty match. In `bbb` the scan counts `bbb`, moves on to position 3, and counts the empty match at the end of the line.
