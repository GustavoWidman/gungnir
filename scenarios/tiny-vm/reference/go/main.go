package main

import (
	"bufio"
	"io"
	"log"
	"os"
	"strconv"
	"strings"
)

const (
	stackLimit = 1_000_000
	callLimit  = 100_000
	locals     = 8
	memorySize = 1 << 20
)

type opcode uint8

const (
	opPush opcode = iota
	opDrop
	opDup
	opSwap
	opOver
	opAdd
	opSub
	opMul
	opDiv
	opMod
	opEq
	opNe
	opLt
	opLe
	opGt
	opGe
	opJmp
	opJz
	opJnz
	opCall
	opRet
	opGet
	opSet
	opLoad
	opStore
	opRead
	opPrint
	opHalt
)

type operand uint8

const (
	noOperand operand = iota
	integerOperand
	localOperand
	labelOperand
)

var mnemonics = map[string]struct {
	op   opcode
	kind operand
}{
	"push": {opPush, integerOperand}, "drop": {opDrop, noOperand}, "dup": {opDup, noOperand},
	"swap": {opSwap, noOperand}, "over": {opOver, noOperand}, "add": {opAdd, noOperand},
	"sub": {opSub, noOperand}, "mul": {opMul, noOperand}, "div": {opDiv, noOperand},
	"mod": {opMod, noOperand}, "eq": {opEq, noOperand}, "ne": {opNe, noOperand},
	"lt": {opLt, noOperand}, "le": {opLe, noOperand}, "gt": {opGt, noOperand},
	"ge": {opGe, noOperand}, "jmp": {opJmp, labelOperand}, "jz": {opJz, labelOperand},
	"jnz": {opJnz, labelOperand}, "call": {opCall, labelOperand}, "ret": {opRet, noOperand},
	"get": {opGet, localOperand}, "set": {opSet, localOperand}, "load": {opLoad, noOperand},
	"store": {opStore, noOperand}, "read": {opRead, noOperand}, "print": {opPrint, noOperand},
	"halt": {opHalt, noOperand},
}

// arg is the literal of push, the local of get and set, or the target index of a jump or
// call, which is -1 when the program never defines the label.
type instruction struct {
	op   opcode
	line int
	arg  int64
}

type vmError struct {
	message string
	line    int
}

func isLabel(name string) bool {
	if name == "" {
		return false
	}
	for i := 0; i < len(name); i++ {
		c := name[i]
		letter := 'a' <= c && c <= 'z' || 'A' <= c && c <= 'Z' || c == '_'
		if !letter && !(i > 0 && '0' <= c && c <= '9') {
			return false
		}
	}
	return true
}

// parseInteger parses -?[0-9]+ into a signed 64-bit value. The form is checked first
// because strconv also accepts a leading +.
func parseInteger(token string) (int64, bool) {
	digits := strings.TrimPrefix(token, "-")
	if digits == "" || strings.Trim(digits, "0123456789") != "" {
		return 0, false
	}
	value, err := strconv.ParseInt(token, 10, 64)
	return value, err == nil
}

func isBlank(r rune) bool {
	return r == ' ' || r == '\t'
}

// assemble turns source lines into instructions. Label operands resolve after the last line,
// so a jump may name a label defined later.
func assemble(source []string) ([]instruction, *vmError) {
	code := make([]instruction, 0, len(source)+1)
	type target struct {
		index int
		name  string
	}
	var targets []target
	labels := make(map[string]int64)
	for i, text := range source {
		line := i + 1
		text, _, _ = strings.Cut(text, ";")
		tokens := strings.FieldsFunc(text, isBlank)
		for len(tokens) > 0 && strings.HasSuffix(tokens[0], ":") {
			name := strings.TrimSuffix(tokens[0], ":")
			tokens = tokens[1:]
			if !isLabel(name) {
				return nil, &vmError{"bad label", line}
			}
			if _, seen := labels[name]; seen {
				return nil, &vmError{"duplicate label", line}
			}
			labels[name] = int64(len(code))
		}
		if len(tokens) == 0 {
			continue
		}
		m, found := mnemonics[strings.ToLower(tokens[0])]
		if !found {
			return nil, &vmError{"unknown instruction", line}
		}
		operands := tokens[1:]
		in := instruction{op: m.op, line: line}
		valid := len(operands) == 0 && m.kind == noOperand
		if len(operands) == 1 {
			switch m.kind {
			case integerOperand:
				in.arg, valid = parseInteger(operands[0])
			case localOperand:
				in.arg, valid = parseInteger(operands[0])
				valid = valid && 0 <= in.arg && in.arg < locals
			case labelOperand:
				valid = isLabel(operands[0])
				targets = append(targets, target{len(code), operands[0]})
			}
		}
		if !valid {
			return nil, &vmError{"bad operand", line}
		}
		code = append(code, in)
	}
	for _, t := range targets {
		index, defined := labels[t.name]
		if !defined {
			index = -1
		}
		code[t.index].arg = index
	}
	// Running off the end halts, and a label after the last instruction points here.
	return append(code, instruction{op: opHalt}), nil
}

// execute runs the program, printing to out, until it halts or an instruction fails.
func execute(code []instruction, data []int64, out *bufio.Writer) *vmError {
	stack := make([]int64, stackLimit)
	frames := make([]int64, locals*(callLimit+1))
	memory := make([]int64, memorySize)
	calls := make([]int, 0, callLimit)
	pc, sp, base, cursor := 0, 0, 0, 0
	for {
		in := &code[pc]
		pc++
		switch in.op {
		case opPush:
			if sp == stackLimit {
				return &vmError{"stack overflow", in.line}
			}
			stack[sp] = in.arg
			sp++
		case opDrop:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			sp--
		case opDup:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			if sp == stackLimit {
				return &vmError{"stack overflow", in.line}
			}
			stack[sp] = stack[sp-1]
			sp++
		case opSwap:
			if sp < 2 {
				return &vmError{"stack underflow", in.line}
			}
			stack[sp-1], stack[sp-2] = stack[sp-2], stack[sp-1]
		case opOver:
			if sp < 2 {
				return &vmError{"stack underflow", in.line}
			}
			if sp == stackLimit {
				return &vmError{"stack overflow", in.line}
			}
			stack[sp] = stack[sp-2]
			sp++
		case opAdd, opSub, opMul, opDiv, opMod, opEq, opNe, opLt, opLe, opGt, opGe:
			if sp < 2 {
				return &vmError{"stack underflow", in.line}
			}
			a, b := stack[sp-2], stack[sp-1]
			var r int64
			// Go's integer arithmetic wraps, and its MinInt64 / -1 is MinInt64 with remainder 0.
			switch in.op {
			case opAdd:
				r = a + b
			case opSub:
				r = a - b
			case opMul:
				r = a * b
			case opDiv, opMod:
				if b == 0 {
					return &vmError{"division by zero", in.line}
				}
				if in.op == opDiv {
					r = a / b
				} else {
					r = a % b
				}
			case opEq:
				r = boolInt(a == b)
			case opNe:
				r = boolInt(a != b)
			case opLt:
				r = boolInt(a < b)
			case opLe:
				r = boolInt(a <= b)
			case opGt:
				r = boolInt(a > b)
			default:
				r = boolInt(a >= b)
			}
			stack[sp-2] = r
			sp--
		case opJmp:
			if in.arg < 0 {
				return &vmError{"undefined label", in.line}
			}
			pc = int(in.arg)
		case opJz, opJnz:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			sp--
			if (stack[sp] == 0) == (in.op == opJz) {
				if in.arg < 0 {
					return &vmError{"undefined label", in.line}
				}
				pc = int(in.arg)
			}
		case opCall:
			if in.arg < 0 {
				return &vmError{"undefined label", in.line}
			}
			if len(calls) == callLimit {
				return &vmError{"call stack overflow", in.line}
			}
			calls = append(calls, pc)
			base += locals
			clear(frames[base : base+locals])
			pc = int(in.arg)
		case opRet:
			if len(calls) == 0 {
				return &vmError{"return without call", in.line}
			}
			pc = calls[len(calls)-1]
			calls = calls[:len(calls)-1]
			base -= locals
		case opGet:
			if sp == stackLimit {
				return &vmError{"stack overflow", in.line}
			}
			stack[sp] = frames[base+int(in.arg)]
			sp++
		case opSet:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			sp--
			frames[base+int(in.arg)] = stack[sp]
		case opLoad:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			address := stack[sp-1]
			if address < 0 || address >= memorySize {
				return &vmError{"bad address", in.line}
			}
			stack[sp-1] = memory[address]
		case opStore:
			if sp < 2 {
				return &vmError{"stack underflow", in.line}
			}
			address := stack[sp-1]
			if address < 0 || address >= memorySize {
				return &vmError{"bad address", in.line}
			}
			memory[address] = stack[sp-2]
			sp -= 2
		case opRead:
			if cursor == len(data) {
				return &vmError{"end of input", in.line}
			}
			if sp == stackLimit {
				return &vmError{"stack overflow", in.line}
			}
			stack[sp] = data[cursor]
			cursor++
			sp++
		case opPrint:
			if sp < 1 {
				return &vmError{"stack underflow", in.line}
			}
			sp--
			line := strconv.AppendInt(out.AvailableBuffer(), stack[sp], 10)
			out.Write(append(line, '\n'))
		case opHalt:
			return nil
		}
	}
}

func boolInt(b bool) int64 {
	if b {
		return 1
	}
	return 0
}

func main() {
	input, err := io.ReadAll(os.Stdin)
	if err != nil {
		log.Fatal(err)
	}
	lines := strings.Split(string(input), "\n")
	if lines[len(lines)-1] == "" {
		lines = lines[:len(lines)-1]
	}
	// The source ends at the first line that is exactly ".input", and the data follows it.
	source, rest := lines, []string(nil)
	for i, line := range lines {
		if line == ".input" {
			source, rest = lines[:i], lines[i+1:]
			break
		}
	}
	var data []int64
	for _, token := range strings.Fields(strings.Join(rest, "\n")) {
		value, err := strconv.ParseInt(token, 10, 64)
		if err != nil {
			log.Fatal(err)
		}
		data = append(data, value)
	}

	out := bufio.NewWriterSize(os.Stdout, 1<<16)
	code, failure := assemble(source)
	if failure == nil {
		failure = execute(code, data, out)
	}
	if failure != nil {
		out.WriteString("error: " + failure.message + " at line " + strconv.Itoa(failure.line) + "\n")
	}
	if err := out.Flush(); err != nil {
		log.Fatal(err)
	}
}
