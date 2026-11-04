package main

import (
	"bytes"
	"errors"
	"io"
	"log"
	"math/big"
	"os"
)

type kind byte

const (
	number kind = iota
	add
	sub
	mul
	div
	mod
	neg
	pow
	open
)

// rank is each operator's binding strength. `^` and unary minus group from the right, the
// others from the left.
var rank = [...]int{add: 1, sub: 1, mul: 2, div: 2, mod: 2, neg: 3, pow: 4}

var binary = map[byte]kind{'+': add, '-': sub, '*': mul, '/': div, '%': mod, '^': pow}

type item struct {
	kind   kind
	digits []byte
}

var (
	errDivisionByZero   = errors.New("division by zero")
	errNegativeExponent = errors.New("negative exponent")
	one                 = big.NewInt(1)
)

// postfix converts a line to postfix order with the shunting-yard algorithm, and reports false
// when the line does not match the grammar.
func postfix(line []byte) ([]item, bool) {
	var items, ops []item
	wantOperand := true
	for i := 0; i < len(line); {
		c := line[i]
		switch {
		case c == ' ' || c == '\t':
			i++
		case '0' <= c && c <= '9':
			if !wantOperand {
				return items, false
			}
			start := i
			for i < len(line) && '0' <= line[i] && line[i] <= '9' {
				i++
			}
			items = append(items, item{kind: number, digits: line[start:i]})
			wantOperand = false
		case wantOperand:
			i++
			switch c {
			case '(':
				ops = append(ops, item{kind: open})
			case '-':
				ops = append(ops, item{kind: neg})
			default:
				return items, false
			}
		case c == ')':
			i++
			for len(ops) > 0 && ops[len(ops)-1].kind != open {
				items = append(items, ops[len(ops)-1])
				ops = ops[:len(ops)-1]
			}
			if len(ops) == 0 {
				return items, false
			}
			ops = ops[:len(ops)-1]
		default:
			i++
			k, ok := binary[c]
			if !ok {
				return items, false
			}
			for len(ops) > 0 && ops[len(ops)-1].kind != open {
				top := rank[ops[len(ops)-1].kind]
				if top < rank[k] || (top == rank[k] && k == pow) {
					break
				}
				items = append(items, ops[len(ops)-1])
				ops = ops[:len(ops)-1]
			}
			ops = append(ops, item{kind: k})
			wantOperand = true
		}
	}
	if wantOperand {
		return items, false
	}
	for len(ops) > 0 {
		if ops[len(ops)-1].kind == open {
			return items, false
		}
		items = append(items, ops[len(ops)-1])
		ops = ops[:len(ops)-1]
	}
	return items, true
}

// big.Int.SetString takes quadratic time in the number of digits, so long numbers are split in
// halves at powers of ten from 10^chunk, 10^(2 chunk), 10^(4 chunk), and so on.
const chunk = 256

var tens []*big.Int

func tenToChunk(i int) *big.Int {
	for len(tens) <= i {
		if len(tens) == 0 {
			tens = append(tens, new(big.Int).Exp(big.NewInt(10), big.NewInt(chunk), nil))
		} else {
			last := tens[len(tens)-1]
			tens = append(tens, new(big.Int).Mul(last, last))
		}
	}
	return tens[i]
}

func parse(digits []byte) *big.Int {
	if len(digits) == 0 {
		return new(big.Int)
	}
	if len(digits) <= chunk {
		z, _ := new(big.Int).SetString(string(digits), 10)
		return z
	}
	i := 0
	for chunk<<(i+1) < len(digits) {
		i++
	}
	split := len(digits) - chunk<<i
	z := parse(digits[:split])
	z.Mul(z, tenToChunk(i))
	return z.Add(z, parse(digits[split:]))
}

// divide returns the quotient rounded toward negative infinity and its remainder. QuoRem
// truncates toward zero, so a nonzero remainder whose sign differs from b's moves one step.
func divide(a, b *big.Int) (*big.Int, *big.Int, error) {
	if b.Sign() == 0 {
		return nil, nil, errDivisionByZero
	}
	q, r := new(big.Int).QuoRem(a, b, new(big.Int))
	if r.Sign() != 0 && (r.Sign() < 0) != (b.Sign() < 0) {
		q.Sub(q, one)
		r.Add(r, b)
	}
	return q, r, nil
}

func power(base, exponent *big.Int) (*big.Int, error) {
	if exponent.Sign() < 0 {
		return nil, errNegativeExponent
	}
	// The exponent is unbounded only for these bases, whose powers need no multiplication.
	switch {
	case base.Sign() == 0 && exponent.Sign() == 0:
		return big.NewInt(1), nil
	case base.Sign() == 0:
		return big.NewInt(0), nil
	case base.CmpAbs(one) == 0 && base.Sign() < 0 && exponent.Bit(0) == 1:
		return big.NewInt(-1), nil
	case base.CmpAbs(one) == 0:
		return big.NewInt(1), nil
	}
	return new(big.Int).Exp(base, exponent, nil), nil
}

func apply(k kind, a, b *big.Int) (*big.Int, error) {
	switch k {
	case add:
		return new(big.Int).Add(a, b), nil
	case sub:
		return new(big.Int).Sub(a, b), nil
	case mul:
		return new(big.Int).Mul(a, b), nil
	case div:
		q, _, err := divide(a, b)
		return q, err
	case mod:
		_, r, err := divide(a, b)
		return r, err
	}
	return power(a, b)
}

// evaluate appends the line's value or error message to out.
func evaluate(out []byte, items []item) []byte {
	var values []*big.Int
	for _, it := range items {
		switch it.kind {
		case number:
			values = append(values, parse(bytes.TrimLeft(it.digits, "0")))
		case neg:
			last := values[len(values)-1]
			last.Neg(last)
		default:
			a, b := values[len(values)-2], values[len(values)-1]
			r, err := apply(it.kind, a, b)
			if err != nil {
				return append(out, err.Error()...)
			}
			values = values[:len(values)-1]
			values[len(values)-1] = r
		}
	}
	return values[0].Append(out, 10)
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	var out []byte
	for len(input) > 0 {
		line, rest, _ := bytes.Cut(input, []byte{'\n'})
		input = rest
		if items, ok := postfix(line); ok {
			out = evaluate(out, items)
		} else {
			out = append(out, "syntax error"...)
		}
		out = append(out, '\n')
	}
	if _, err := os.Stdout.Write(out); err != nil {
		log.Fatal(err)
	}
}
