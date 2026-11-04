# Exact cover

You are given a rectangular board, some of whose cells are blocked, and a list of pieces. Count the ways to place every piece on the board so that the pieces cover every free cell exactly once.

## Input

The input is ASCII text read from standard input. Every line, including the last, ends with a line feed (byte 10). Integers are written in decimal with no sign and no leading zeros.

- The first line holds two integers, `H` and `W`, separated by one space: the number of rows and columns of the board.
- Each of the next `H` lines holds exactly `W` characters, one per cell from left to right. `.` is a free cell and `#` is a blocked cell.
- The next line holds one integer `P`, the number of pieces.
- Then come the `P` pieces, one after another. Each piece starts with a line holding two integers, `h` and `w`, separated by one space, followed by `h` lines of exactly `w` characters each. `#` is a cell of the piece and `.` is not.

Apart from the single spaces between two integers, no line holds any other character, such as a carriage return.

## Pieces

A piece is the set of its `#` cells. Its drawing may have rows or columns that hold no `#` at all. Those empty rows and columns are not part of the piece and do not limit where it goes: only the `#` cells must lie on the board.

The cells of every piece are connected: from any of its cells, any other can be reached through cells of the piece by steps to the cell above, below, left or right.

A piece may be rotated by any multiple of 90 degrees and may be turned over (reflected), before it is moved to any position on the board, with each of its cells landing exactly on a cell of the board. A **placement** of a piece is the set of board cells it then covers. Different rotations and reflections that cover the same cells are the same placement, so a symmetric piece has fewer distinct placements than a piece without symmetry.

## Tilings

A **tiling** gives every one of the `P` pieces one placement such that

- every cell each placement covers is free, and
- every free cell is covered by exactly one piece.

Every piece is used exactly once, so there is no tiling when the pieces' cells together outnumber the free cells or fall short of them.

Two tilings are different when at least one piece has a different placement in them. The pieces are told apart by their position in the input, even when two of them have the same shape: exchanging the placements of two pieces of the same shape gives a different tiling. The board is fixed, so rotating or reflecting a whole tiling gives a different tiling unless every piece keeps its placement.

When there are no free cells and no pieces, the empty tiling is the only one, and the answer is 1.

## Output

Write one line to standard output: the number of tilings, written in decimal, followed by a line feed.

## Constraints

- `1 <= H <= 20` and `1 <= W <= 20`
- `0 <= P <= 100`
- `1 <= h <= 8` and `1 <= w <= 8` for every piece, and every piece has at least one `#` cell.
- The number of tilings is less than 2<sup>63</sup>, so it fits in a signed 64-bit integer.

Some answers are far larger than 2<sup>53</sup>, and some inputs repeat a shape many times and have more than 10<sup>15</sup> tilings. Other inputs have only a few tilings but an enormous number of ways to place some of the pieces before a dead end shows.

## Examples

Input:

```text
2 3
...
...
2
2 2
##
#.
2 2
#.
##
```

Output:

```text
4
```

The two pieces are the same L shape, drawn in two orientations. The 2×3 board has two ways to split into two L shapes, `AAB/ABB` and `ABB/AAB`, and in each the first piece can take either half, so there are 4 tilings.

Input:

```text
2 4
....
#..#
4
1 3
###
1 1
#
1 1
#
1 1
#
```

Output:

```text
12
```

The board has six free cells. The straight piece fits only in the top row, at columns 1 to 3 or 2 to 4; turned upright it would need three rows. Its rotations by 180 degrees cover the same cells, so each position is one placement. The three single cells then fill the three remaining free cells in 3 × 2 × 1 = 6 ways, for 2 × 6 = 12 tilings.
