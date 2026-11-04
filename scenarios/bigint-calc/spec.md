# Big integer calculator

Read integer expressions, one per line, evaluate each exactly with integers of unbounded size, and print each result in decimal.

## Input

The input is ASCII text read from standard input. It holds zero or more lines, and every line, including the last, ends with a line feed (byte 10). A line contains only tabs (byte 9) and the printable ASCII characters 32 through 126. Each line is one expression, evaluated on its own.

## Grammar

A line is made of these tokens:

- A number: one or more decimal digits `0` to `9`. Leading zeros are allowed, so `007` is 7. A number has no sign and no other characters: no `+`, `_`, `,`, `.`, exponent or base prefix.
- An operator: `+`, `-`, `*`, `/`, `%` or `^`.
- A parenthesis: `(` or `)`.

Spaces and tabs may appear before and after any token, any number of times, and are never required. They cannot appear inside a number, so `1 000` is two numbers in a row, which is a syntax error.

A line must match `line` in this grammar, where whitespace is left out:

```text
line    = expr
expr    = term { ("+" | "-") term }
term    = unary { ("*" | "/" | "%") unary }
unary   = "-" unary | power
power   = primary [ "^" unary ]
primary = number | "(" expr ")"
```

`{ ... }` repeats zero or more times and `[ ... ]` is optional. The grammar fixes precedence and grouping:

- Binary `+` and `-` bind loosest, and `*`, `/` and `%` bind tighter. Both levels group from the left, so `7 - 3 - 2` is `(7 - 3) - 2` and `100 / 10 / 5` is `(100 / 10) / 5`.
- A `-` where an operand is expected is unary minus. It binds tighter than `*`, `/` and `%` but looser than `^`. So `-7 / 2` is `(-7) / 2`, `2 * -3` is `2 * (-3)`, and `-2^2` is `-(2^2)`, which is -4.
- `^` binds tightest and groups from the right, so `2^3^2` is `2^(3^2)`, which is 512. The operand to the right of `^` may start with unary minus: `2^-1` is `2^(-1)`, and `2^-3^2` is `2^(-(3^2))`.
- Unary minus may repeat: `--5` is 5 and `1 - -2` is 3.
- A `-` is never part of a number. It is always an operator, so `-5^2` is -25, not 25.

There is no unary `+`, no implicit multiplication (`2(3)` and `(2)(3)` are syntax errors), and no operator spelled with two characters (`**` and `//` are syntax errors). A line that is empty or holds only whitespace does not match the grammar.

## Values

Every value is an integer of unbounded size. The operators compute:

- `a + b`, `a - b` and `a * b`: the exact sum, difference and product.
- `-a`: the negation of `a`.
- `a / b`: the quotient rounded toward negative infinity, the largest integer `q` with `q * b <= a` when `b > 0`, or with `q * b >= a` when `b < 0`.
- `a % b`: the remainder `a - b * (a / b)`. It is 0 or has the sign of `b`, and its absolute value is less than the absolute value of `b`.
- `a ^ b`: `a` raised to the power `b`, for `b >= 0`. `a ^ 0` is 1 for every `a`, including `0 ^ 0`.

Rounding toward negative infinity matters when exactly one operand is negative:

| `a` | `b` | `a / b` | `a % b` |
| ---: | ---: | ---: | ---: |
| 7 | 2 | 3 | 1 |
| -7 | 2 | -4 | 1 |
| 7 | -2 | -4 | -1 |
| -7 | -2 | 3 | -1 |
| -6 | 3 | -2 | 0 |
| 1 | -5 | -1 | -4 |

## Errors

A line fails in one of three ways, each printed as a fixed message:

- `syntax error`: the line does not match the grammar. This covers characters outside the tokens above, unbalanced parentheses, a missing or extra operand, two numbers in a row, and empty or blank lines.
- `division by zero`: the right operand of `/` or `%` is 0.
- `negative exponent`: the right operand of `^` is negative, whatever the left operand is. `0 ^ -1`, `1 ^ -1` and `(-1) ^ -1` are all `negative exponent`.

The grammar is checked first. A line with a syntax error prints `syntax error` and is not evaluated at all, even in part, so `1 / 0 +` is a syntax error, and so is `9^9^9^9 )` though its power would have hundreds of millions of digits.

A line without a syntax error is evaluated in this order. For a binary operator, its left operand is evaluated completely, then its right operand, and then the operator is applied. For unary minus, its operand is evaluated and then negated. Evaluation stops at the first operator that fails, the line prints that operator's error, and nothing after it is evaluated. So `1/0 + 2^-1` prints `division by zero`, `2^-1 + 1/0` prints `negative exponent`, and `1/0 + 9^9^9^9` prints `division by zero` without computing the power.

## Output

Write exactly one line per input line, in input order, each followed by a line feed. For a line that evaluates without error, write its value in decimal: a `-` before the digits when it is negative, no leading zeros, and `0` for zero, never `-0`. For a line that fails, write its message. When the input is empty, write nothing.

## Constraints

- The input is at most 20,000,000 bytes long, and a line holds at most 1,000,000 bytes before its line feed.
- Every number in the input is below 10^200000, so it has at most 200,000 digits once its leading zeros are dropped. Leading zeros can make it longer.
- In a line without a syntax error, every value that evaluation computes before it stops has an absolute value below 10^200000. This bounds powers: when the left operand of `^` is 0, 1 or -1, its right operand can be any value up to that bound, and otherwise the right operand is at most 664385.
- Summed over the whole input, the values that evaluation computes, numbers read from the input included, have at most 100,000,000 digits.
- A syntax error or the first failing operator ends a line's evaluation, so the bounds on values say nothing about what a line would compute past that point.

Parentheses, unary minus signs and `^` operators can nest as deeply as the line length allows, hundreds of thousands of levels, so a parser or evaluator that recurses once per level can run out of stack. Some lines hold numbers with hundreds of thousands of digits, and some inputs hold hundreds of thousands of short lines.

## Examples

Input:

```text
1 + 2 * 3
-2^2
(-2)^2
2^3^2
-7 / 2
-7 % 2
7 % -2
--5 - -5
```

Output:

```text
7
-4
4
512
-4
1
-1
10
```

Unary minus binds looser than `^`, so `-2^2` negates 4. It binds tighter than `/` and `%`, so `-7 / 2` divides -7, and the quotient -3.5 rounds down to -4.

Input:

```text
2^100
123456789012345678901234567890 * -987654321
1 / (3 - 3)
2 ^ -1
1 2
1/0 +
0^0
  0007	- 7
```

Output:

```text
1267650600228229401496703205376
-121932631124828532112482853211126352690
division by zero
negative exponent
syntax error
syntax error
1
0
```

`1 2` holds two numbers in a row. `1/0 +` lacks its last operand, and the syntax error wins over the division by zero, since the grammar is checked before anything is evaluated. The last line has spaces and a tab around its tokens.
