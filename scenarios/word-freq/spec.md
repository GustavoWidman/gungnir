# Word frequency

Read a text, split it into words, and print the `K` most frequent words with their counts.

## Input

The input is read from standard input as bytes.

- The first line holds one integer `K`, written in decimal with no sign and no leading zeros, followed by a line feed (byte 10).
- Every byte after that first line feed is the text. The text may be empty, and it need not end with a line feed.

The text contains only these bytes: tab (9), line feed (10), carriage return (13), and the printable ASCII characters 32 through 126.

## Words

A word is a maximal run of one or more ASCII letters, the bytes `A` to `Z` (65 to 90) and `a` to `z` (97 to 122). Every other byte, including digits, apostrophes, hyphens, underscores, spaces and line breaks, separates words. Words are case-folded, so each uppercase letter is replaced by its lowercase letter before words are counted.

For example, `Don't` gives the words `don` and `t`, `e-mail` gives `e` and `mail`, `abc123DEF` gives `abc` and `def`, and `HeLLo` and `hello` are the same word.

## Output

Let `D` be the number of distinct words in the text. Print `min(K, D)` lines, one per word, ranked as follows.

1. A word with a higher count comes first.
2. Among words with equal counts, the word that is smaller in byte order comes first. A word sorts before any longer word that starts with it, so `a` comes before `ab`.

Each line holds the word in lowercase, one space, its count in decimal, and a line feed. When the text has no words, print nothing.

## Constraints

- `1 <= K <= 1000000`
- The text is at most 50000000 bytes long.
- A single word can be as long as the whole text.

Some texts are tens of megabytes long.

## Examples

Input:

```text
3
The cat and the hat. THE END, and the cat's hat!
```

Output:

```text
the 4
and 2
cat 2
```

`cat's` gives the words `cat` and `s`. The words `and`, `cat` and `hat` all appear twice, so the tie goes to `and` and `cat`, the two that come first in byte order.

Input:

```text
10
x1y2z3 x-ray X
```

Output:

```text
x 3
ray 1
y 1
z 1
```

There are only four distinct words, so all four are printed even though `K` is 10.
