# Interval cover

You are given N closed intervals on the integer line and Q query points. For each query point, report how many of the intervals contain it.

An interval `[L, R]` contains a point `X` when `L <= X <= R`. Both endpoints belong to the interval.

## Input

The input is ASCII text read from standard input.

- The first line holds two integers, `N` and `Q`, separated by one space.
- Each of the next `N` lines holds one interval as two integers, `L` and `R`, separated by one space.
- Each of the next `Q` lines holds one query point, a single integer `X`.

Every line, including the last, ends with a line feed (byte 10). Integers are written in decimal with an optional leading `-` and no leading `+` or leading zeros.

## Output

Write exactly `Q` lines to standard output, one per query, in input order. Line `i` holds the number of intervals that contain the `i`-th query point, written in decimal, followed by a line feed. When `Q` is 0, write nothing.

## Constraints

- `0 <= N <= 1000000`
- `0 <= Q <= 1000000`
- `-9223372036854775808 <= L <= R <= 9223372036854775807` for every interval. Intervals may repeat, nest, overlap or touch.
- `-9223372036854775808 <= X <= 9223372036854775807` for every query. Query points may repeat and come in any order.

The coordinates span the full signed 64-bit range, so a computation such as `R - L` or `R + 1` can overflow a 64-bit integer.

Some inputs have hundreds of thousands of intervals and queries. A solution that compares every query with every interval is far too slow for them.

## Examples

Input:

```text
3 4
1 5
3 8
5 5
0
3
5
9
```

Output:

```text
0
2
3
0
```

The point 5 lies in all three intervals. It is the right end of `[1, 5]`, lies inside `[3, 8]`, and is the only point of `[5, 5]`.

Input:

```text
2 3
-9223372036854775808 0
0 9223372036854775807
-9223372036854775808
0
9223372036854775807
```

Output:

```text
1
2
1
```
