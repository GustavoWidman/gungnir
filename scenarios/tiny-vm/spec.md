# Tiny VM

Assemble and run a program for a small stack machine. The program's source text and the input it reads arrive together on standard input. Print what the program prints, and report the first error that stops it.

## Input

The input is read from standard input as bytes. It contains only tab (9), line feed (10) and the printable ASCII characters 32 through 126.

The input is a sequence of lines separated by line feeds. The last line need not end with a line feed, and a line feed at the very end does not start another line. Lines are numbered from 1 at the start of the input.

The source is every line before the first line that is exactly the six bytes `.input`, with nothing before or after them on the line. The data is everything after that line. When no line is exactly `.input`, every line is source and the data is empty.

The data is a sequence of integers separated by spaces, tabs and line feeds, possibly with more of them before the first integer and after the last. Each integer is written in decimal with an optional leading `-` and no leading zeros, and lies from -9223372036854775808 to 9223372036854775807. The data may hold no integers at all.

## Source syntax

Each source line is read as follows.

1. A `;` starts a comment. The `;` and everything after it on the line are ignored.
2. What remains is split into tokens. Tokens are separated by one or more spaces or tabs. Spaces and tabs at the start or end of a line are ignored. A line with no tokens is blank and is skipped.
3. Each leading token that ends with `:` defines a label. Its name is the token without that final `:`. A line may define any number of labels this way.
4. The first token after the label definitions, if there is one, is a mnemonic, and every token after the mnemonic is an operand. Each line holds at most one instruction.

Mnemonics are case-insensitive, so `push`, `PUSH` and `pUsH` are the same instruction. Label names are case-sensitive, so `loop` and `Loop` are two different labels.

A label name is a letter (`A` to `Z` or `a` to `z`) or underscore, followed by any number of letters, digits (`0` to `9`) and underscores. Any such name may be a label, including one spelled like a mnemonic.

A label marks the next instruction in source order, whether it is on the same line or a later one. A label followed by no instruction marks the end of the program.

The operand of an instruction is one of three kinds.

- An integer: an optional `-` followed by one or more digits, and nothing else. Leading zeros are allowed, so `007`, `-0` and `0000` are valid. Its value must lie from -9223372036854775808 to 9223372036854775807. A `+` sign, a decimal point, an underscore or any other character makes it invalid.
- A local: an integer as above whose value is from 0 to 7.
- A label: a label name. The label does not have to be defined anywhere in the program. A jump or call to a label the program never defines fails only when it runs.

## The machine

Every value is a signed 64-bit integer, from -9223372036854775808 to 9223372036854775807.

- The data stack holds at most 1000000 values. It starts empty.
- The memory is 1048576 cells with addresses 0 to 1048575. Every cell starts at 0.
- A frame holds 8 locals, numbered 0 to 7. The main program runs in its own frame. Each `call` gives the callee a new frame whose 8 locals are all 0, and the matching `ret` discards that frame and returns to the caller's frame, whose locals are as the caller left them. The instructions `get` and `set` reach only the current frame.
- The call stack holds the return address of every call that has not returned yet. At most 100000 calls can be active at once.
- The program reads the integers of the data in order, one per `read`.

Execution starts at the first instruction and runs the instructions in source order unless one transfers control. It stops normally at `halt`, or when control moves past the last instruction, which includes a jump, call or return to a label or address at the end of the program. Stopping this way ends the whole program, even while calls are active.

Arithmetic wraps. A result of `add`, `sub`, `mul` or `div` that falls outside the 64-bit range is reduced modulo 2^64 into it, as two's complement hardware does. So 9223372036854775807 plus 1 is -9223372036854775808, and -9223372036854775808 divided by -1 is -9223372036854775808.

Division truncates toward zero. The remainder `a mod b` is `a - b * (a div b)`, so it is 0 or has the sign of `a`. For example, 7 div 2 is 3, -7 div 2 is -3, 7 div -2 is -3, -7 mod 2 is -1, 7 mod -2 is 1, and -9223372036854775808 mod -1 is 0.

## Instructions

Below, `b` is the value on top of the stack and `a` is the value just below it. An instruction that removes values removes them from the top. Comparisons push 1 when they hold and 0 when they do not, and compare as signed integers.

| Instruction | Effect | Can fail with, checked in this order |
| --- | --- | --- |
| `push N` | Pushes the integer `N`. | stack overflow |
| `drop` | Removes the top value. | stack underflow |
| `dup` | Pushes a copy of the top value. | stack underflow, stack overflow |
| `swap` | Exchanges the top two values. | stack underflow |
| `over` | Pushes a copy of `a`, the value below the top. | stack underflow, stack overflow |
| `add` | Removes `b` and `a`, pushes `a + b`. | stack underflow |
| `sub` | Removes `b` and `a`, pushes `a - b`. | stack underflow |
| `mul` | Removes `b` and `a`, pushes `a * b`. | stack underflow |
| `div` | Removes `b` and `a`, pushes `a div b`. | stack underflow, division by zero |
| `mod` | Removes `b` and `a`, pushes `a mod b`. | stack underflow, division by zero |
| `eq` | Removes `b` and `a`, pushes whether `a` equals `b`. | stack underflow |
| `ne` | Removes `b` and `a`, pushes whether `a` differs from `b`. | stack underflow |
| `lt` | Removes `b` and `a`, pushes whether `a < b`. | stack underflow |
| `le` | Removes `b` and `a`, pushes whether `a <= b`. | stack underflow |
| `gt` | Removes `b` and `a`, pushes whether `a > b`. | stack underflow |
| `ge` | Removes `b` and `a`, pushes whether `a >= b`. | stack underflow |
| `jmp L` | Continues at label `L`. | undefined label |
| `jz L` | Removes the top value. If it is 0, continues at label `L`. | stack underflow, undefined label |
| `jnz L` | Removes the top value. If it is not 0, continues at label `L`. | stack underflow, undefined label |
| `call L` | Saves the address of the instruction after the `call`, enters a new frame, and continues at label `L`. | undefined label, call stack overflow |
| `ret` | Leaves the current frame and continues at the address the matching `call` saved. | return without call |
| `get K` | Pushes local `K` of the current frame. | stack overflow |
| `set K` | Removes the top value and stores it in local `K` of the current frame. | stack underflow |
| `load` | Removes `b`, an address, and pushes the memory cell at address `b`. | stack underflow, bad address |
| `store` | Removes `b`, an address, and `a`, and stores `a` into the memory cell at address `b`. | stack underflow, bad address |
| `read` | Pushes the next integer of the data. | end of input, stack overflow |
| `print` | Removes the top value and prints it. | stack underflow |
| `halt` | Stops the program normally. | nothing |

`push`, `get` and `set` take exactly one integer, local and local operand respectively, and `jmp`, `jz`, `jnz` and `call` take exactly one label operand. Every other instruction takes no operand.

## Errors

The program is assembled completely before any instruction runs. Assembly reads the source lines in order and stops at the first problem it finds. On one line it checks the label definitions from left to right, then the mnemonic, then the operands. The problems are these.

- `bad label`: a label definition whose name is not a valid label name, such as `1st:`, `a-b:`, `x::` or a lone `:`.
- `duplicate label`: a label definition whose name an earlier definition, on the same line or an earlier one, already defined.
- `unknown instruction`: the mnemonic is not one of the 28 in the table.
- `bad operand`: the instruction has the wrong number of operands, or an operand is not of the kind the instruction takes.

When assembly finds a problem, nothing runs and nothing else is printed.

At run time, an instruction fails in these cases, each checked before the instruction changes anything.

- `stack underflow`: the stack holds fewer values than the instruction removes or copies. That is one value for `drop`, `dup`, `jz`, `jnz`, `set`, `load` and `print`, and two values for `swap`, `over`, `store` and the arithmetic and comparison instructions.
- `stack overflow`: the instruction would leave more than 1000000 values on the stack.
- `division by zero`: `b` is 0 for `div` or `mod`.
- `bad address`: the address `b` of `load` or `store` is below 0 or above 1048575.
- `undefined label`: the program never defines the label of a `jmp` or `call`, or of a `jz` or `jnz` whose condition holds. A `jz` or `jnz` whose condition does not hold continues with the next instruction without looking at its label.
- `call stack overflow`: a `call` runs while 100000 calls are active.
- `return without call`: a `ret` runs while no call is active.
- `end of input`: a `read` runs after every integer of the data has been read.

When an instruction can fail in more than one way, the table lists the checks in the order they are made, and the first that fails is reported.

## Output

Each `print` writes the value in decimal, with a leading `-` when it is negative and no leading zeros, followed by a line feed.

When assembly or an instruction fails, the program writes one more line after everything printed so far and stops:

```text
error: MESSAGE at line N
```

`MESSAGE` is one of the messages above, and `N` is the number of the line where assembly found the problem, or the number of the line that holds the instruction that failed. The line ends with a line feed.

A program that stops normally writes only what it printed, which may be nothing. Exit with status 0 in every case, including after an error.

## Constraints

- The input is at most 10000000 bytes long, and the source is at most 100000 lines.
- The data holds at most 1000000 integers.
- Every test program stops on its own, normally or with an error. Some execute tens of millions of instructions before they stop.

## Examples

Input:

```text
; Sums the data up to the first 0, then prints the sum and how many it added.
        push 0
        set 0           ; local 0 is the sum
next:   read
        dup
        jz done
        get 0
        add
        set 0
        get 1           ; local 1 is the count
        push 1
        add
        set 1
        jmp next
done:   get 0
        print
        get 1
        print
.input
5 -3
10 0 99
```

Output:

```text
12
3
```

The `jz done` that reads the 0 removes it and jumps, so the trailing 99 is never read.

Input:

```text
push 9223372036854775807
push 1
add
print
push -7
push 2
div
print
PUSH -7
Push 2
mod
print
drop
```

Output:

```text
-9223372036854775808
-3
-1
error: stack underflow at line 13
```

The sum wraps to the smallest value, division truncates toward zero, and the remainder takes the sign of `-7`. The `drop` on line 13 finds the stack empty, so the program stops after the three values it printed.
