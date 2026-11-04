#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

enum {
    STACK_LIMIT = 1000000,
    CALL_LIMIT = 100000,
    LOCALS = 8,
    MEMORY = 1 << 20,
};

typedef enum {
    PUSH, DROP, DUP, SWAP, OVER, ADD, SUB, MUL, DIV, MOD, EQ, NE, LT, LE, GT, GE,
    JMP, JZ, JNZ, CALL, RET, GET, SET, LOAD, STORE, READ, PRINT, HALT,
} Op;

typedef enum { NONE, INTEGER, LOCAL, LABEL } Operand;

static const struct {
    const char *name;
    Op op;
    Operand operand;
} MNEMONICS[] = {
    {"push", PUSH, INTEGER}, {"drop", DROP, NONE}, {"dup", DUP, NONE}, {"swap", SWAP, NONE},
    {"over", OVER, NONE}, {"add", ADD, NONE}, {"sub", SUB, NONE}, {"mul", MUL, NONE},
    {"div", DIV, NONE}, {"mod", MOD, NONE}, {"eq", EQ, NONE}, {"ne", NE, NONE},
    {"lt", LT, NONE}, {"le", LE, NONE}, {"gt", GT, NONE}, {"ge", GE, NONE},
    {"jmp", JMP, LABEL}, {"jz", JZ, LABEL}, {"jnz", JNZ, LABEL}, {"call", CALL, LABEL},
    {"ret", RET, NONE}, {"get", GET, LOCAL}, {"set", SET, LOCAL}, {"load", LOAD, NONE},
    {"store", STORE, NONE}, {"read", READ, NONE}, {"print", PRINT, NONE}, {"halt", HALT, NONE},
};

/* arg is the literal of push, the local of get and set, or the target index of a jump or call,
 * which is -1 when the program never defines the label. */
typedef struct {
    Op op;
    uint32_t line;
    int64_t arg;
} Instruction;

typedef struct {
    const char *start;
    size_t length;
} Slice;

typedef struct {
    const char *message;
    uint32_t line;
} Error;

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

static char *read_all(FILE *stream, size_t *length_out)
{
    size_t capacity = 1 << 16;
    size_t length = 0;
    char *data = checked(malloc(capacity));
    while ((length += fread(data + length, 1, capacity - length, stream)) == capacity) {
        capacity *= 2;
        data = checked(realloc(data, capacity));
    }
    if (ferror(stream)) {
        perror("reading standard input");
        exit(EXIT_FAILURE);
    }
    *length_out = length;
    return data;
}

/* Open addressing from label names to instruction indices. */
typedef struct {
    Slice *names;
    int64_t *values;
    size_t mask;
} Labels;

static uint64_t hash(Slice s)
{
    uint64_t h = 14695981039346656037u;
    for (size_t i = 0; i < s.length; i++)
        h = (h ^ (unsigned char)s.start[i]) * 1099511628211u;
    return h;
}

static size_t slot(const Labels *labels, Slice name)
{
    size_t i = hash(name) & labels->mask;
    while (labels->names[i].start != nullptr &&
           (labels->names[i].length != name.length ||
            memcmp(labels->names[i].start, name.start, name.length) != 0))
        i = (i + 1) & labels->mask;
    return i;
}

static bool is_label(Slice s)
{
    if (s.length == 0)
        return false;
    for (size_t i = 0; i < s.length; i++) {
        char c = s.start[i];
        bool letter = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_';
        if (!letter && !(i > 0 && c >= '0' && c <= '9'))
            return false;
    }
    return true;
}

/* Parses -?[0-9]+ into a signed 64-bit value, failing on any other form or on overflow. */
static bool parse_integer(Slice s, int64_t *value)
{
    size_t i = s.length > 0 && s.start[0] == '-';
    bool negative = i == 1;
    if (i == s.length)
        return false;
    uint64_t limit = negative ? (uint64_t)1 << 63 : ((uint64_t)1 << 63) - 1;
    uint64_t magnitude = 0;
    for (; i < s.length; i++) {
        char c = s.start[i];
        if (c < '0' || c > '9')
            return false;
        uint64_t digit = (uint64_t)(c - '0');
        if (magnitude > (limit - digit) / 10)
            return false;
        magnitude = magnitude * 10 + digit;
    }
    *value = negative ? (int64_t)(0 - magnitude) : (int64_t)magnitude;
    return true;
}

static bool lookup(Slice s, size_t *index)
{
    char lower[8];
    if (s.length >= sizeof lower)
        return false;
    for (size_t i = 0; i < s.length; i++) {
        char c = s.start[i];
        lower[i] = (c >= 'A' && c <= 'Z') ? (char)(c - 'A' + 'a') : c;
    }
    lower[s.length] = '\0';
    for (size_t i = 0; i < sizeof MNEMONICS / sizeof MNEMONICS[0]; i++) {
        if (strcmp(lower, MNEMONICS[i].name) == 0) {
            *index = i;
            return true;
        }
    }
    return false;
}

/* The next run of non-blank bytes in [*cursor, end), or an empty slice at end. */
static Slice next_token(const char **cursor, const char *end)
{
    const char *p = *cursor;
    while (p < end && (*p == ' ' || *p == '\t'))
        p++;
    const char *start = p;
    while (p < end && *p != ' ' && *p != '\t')
        p++;
    *cursor = p;
    return (Slice){start, (size_t)(p - start)};
}

typedef struct {
    Instruction *code;
    size_t length;
} Program;

/* Assembles the source lines in [source, end). Label operands resolve after the last line,
 * so a jump may name a label defined later. */
static bool assemble(const char *source, const char *end, size_t line_count, Program *program,
                     Error *error)
{
    Instruction *code = checked(malloc((line_count + 1) * sizeof *code));
    Slice *targets = checked(malloc((line_count + 1) * sizeof *targets));
    size_t capacity = 16;
    while (capacity < 2 * line_count)
        capacity *= 2;
    Labels labels = {checked(calloc(capacity, sizeof(Slice))),
                     checked(malloc(capacity * sizeof(int64_t))), capacity - 1};
    size_t count = 0;
    uint32_t number = 0;
    bool ok = true;
    const char *line = source;
    while (ok && line < end) {
        const char *newline = memchr(line, '\n', (size_t)(end - line));
        const char *line_end = newline != nullptr ? newline : end;
        const char *comment = memchr(line, ';', (size_t)(line_end - line));
        const char *stop = comment != nullptr ? comment : line_end;
        number++;
        const char *cursor = line;
        line = newline != nullptr ? newline + 1 : end;

        Slice token = next_token(&cursor, stop);
        while (token.length > 0 && token.start[token.length - 1] == ':') {
            Slice name = {token.start, token.length - 1};
            size_t i = slot(&labels, name);
            if (!is_label(name) || labels.names[i].start != nullptr) {
                *error = (Error){is_label(name) ? "duplicate label" : "bad label", number};
                ok = false;
                break;
            }
            labels.names[i] = name;
            labels.values[i] = (int64_t)count;
            token = next_token(&cursor, stop);
        }
        if (!ok || token.length == 0)
            continue;
        size_t m;
        if (!lookup(token, &m)) {
            *error = (Error){"unknown instruction", number};
            ok = false;
            continue;
        }
        Slice operand = next_token(&cursor, stop);
        bool extra = next_token(&cursor, stop).length > 0;
        Operand kind = MNEMONICS[m].operand;
        Instruction in = {MNEMONICS[m].op, number, 0};
        targets[count] = kind == LABEL ? operand : (Slice){};
        bool valid = !extra && (operand.length > 0) == (kind != NONE);
        if (valid && kind == INTEGER)
            valid = parse_integer(operand, &in.arg);
        else if (valid && kind == LOCAL)
            valid = parse_integer(operand, &in.arg) && in.arg >= 0 && in.arg < LOCALS;
        else if (valid && kind == LABEL)
            valid = is_label(operand);
        if (!valid) {
            *error = (Error){"bad operand", number};
            ok = false;
            continue;
        }
        code[count++] = in;
    }
    for (size_t i = 0; ok && i < count; i++) {
        if (targets[i].start != nullptr) {
            size_t s = slot(&labels, targets[i]);
            code[i].arg = labels.names[s].start != nullptr ? labels.values[s] : -1;
        }
    }
    /* Running off the end halts, and a label after the last instruction points here. */
    code[count] = (Instruction){HALT, 0, 0};
    free(labels.values);
    free(labels.names);
    free(targets);
    if (!ok) {
        free(code);
        return false;
    }
    *program = (Program){code, count};
    return true;
}

typedef struct {
    size_t length;
    char data[1 << 16];
} Output;

static void flush(Output *out)
{
    fwrite(out->data, 1, out->length, stdout);
    out->length = 0;
}

static void write_bytes(Output *out, const char *bytes, size_t length)
{
    if (sizeof out->data - out->length < length)
        flush(out);
    memcpy(out->data + out->length, bytes, length);
    out->length += length;
}

static void write_int(Output *out, int64_t value)
{
    char digits[24];
    size_t at = sizeof digits;
    digits[--at] = '\n';
    uint64_t magnitude = value < 0 ? 0 - (uint64_t)value : (uint64_t)value;
    do {
        digits[--at] = (char)('0' + magnitude % 10);
        magnitude /= 10;
    } while (magnitude != 0);
    if (value < 0)
        digits[--at] = '-';
    write_bytes(out, digits + at, sizeof digits - at);
}

static int64_t stack[STACK_LIMIT];
static int64_t frames[LOCALS * (CALL_LIMIT + 1)];
static int64_t memory[MEMORY];
static size_t calls[CALL_LIMIT];

#define FAIL(text)                                                                                 \
    do {                                                                                           \
        message = (text);                                                                          \
        goto failed;                                                                               \
    } while (0)

/* Runs the program, printing to out. Returns the error that stopped it, or a null message
 * when it halted. */
static Error execute(const Instruction *code, const int64_t *data, size_t data_length,
                     Output *out)
{
    size_t pc = 0;
    size_t sp = 0;
    size_t depth = 0;
    size_t cursor = 0;
    int64_t *locals = frames;
    const char *message;
    for (;;) {
        const Instruction *in = &code[pc++];
        switch (in->op) {
        case PUSH:
            if (sp == STACK_LIMIT)
                FAIL("stack overflow");
            stack[sp++] = in->arg;
            break;
        case DROP:
            if (sp < 1)
                FAIL("stack underflow");
            sp--;
            break;
        case DUP:
            if (sp < 1)
                FAIL("stack underflow");
            if (sp == STACK_LIMIT)
                FAIL("stack overflow");
            stack[sp] = stack[sp - 1];
            sp++;
            break;
        case SWAP: {
            if (sp < 2)
                FAIL("stack underflow");
            int64_t top = stack[sp - 1];
            stack[sp - 1] = stack[sp - 2];
            stack[sp - 2] = top;
            break;
        }
        case OVER:
            if (sp < 2)
                FAIL("stack underflow");
            if (sp == STACK_LIMIT)
                FAIL("stack overflow");
            stack[sp] = stack[sp - 2];
            sp++;
            break;
        case ADD:
        case SUB:
        case MUL:
        case DIV:
        case MOD:
        case EQ:
        case NE:
        case LT:
        case LE:
        case GT:
        case GE: {
            if (sp < 2)
                FAIL("stack underflow");
            int64_t a = stack[sp - 2];
            int64_t b = stack[sp - 1];
            int64_t r;
            switch (in->op) {
            case ADD: r = (int64_t)((uint64_t)a + (uint64_t)b); break;
            case SUB: r = (int64_t)((uint64_t)a - (uint64_t)b); break;
            case MUL: r = (int64_t)((uint64_t)a * (uint64_t)b); break;
            case DIV:
            case MOD:
                if (b == 0)
                    FAIL("division by zero");
                /* INT64_MIN / -1 overflows in C, and the wrapped results are -a and 0. */
                if (b == -1)
                    r = in->op == DIV ? (int64_t)(0 - (uint64_t)a) : 0;
                else
                    r = in->op == DIV ? a / b : a % b;
                break;
            case EQ: r = a == b; break;
            case NE: r = a != b; break;
            case LT: r = a < b; break;
            case LE: r = a <= b; break;
            case GT: r = a > b; break;
            default: r = a >= b; break;
            }
            stack[sp - 2] = r;
            sp--;
            break;
        }
        case JMP:
            if (in->arg < 0)
                FAIL("undefined label");
            pc = (size_t)in->arg;
            break;
        case JZ:
        case JNZ:
            if (sp < 1)
                FAIL("stack underflow");
            if ((stack[--sp] == 0) == (in->op == JZ)) {
                if (in->arg < 0)
                    FAIL("undefined label");
                pc = (size_t)in->arg;
            }
            break;
        case CALL:
            if (in->arg < 0)
                FAIL("undefined label");
            if (depth == CALL_LIMIT)
                FAIL("call stack overflow");
            calls[depth++] = pc;
            locals += LOCALS;
            memset(locals, 0, LOCALS * sizeof *locals);
            pc = (size_t)in->arg;
            break;
        case RET:
            if (depth == 0)
                FAIL("return without call");
            pc = calls[--depth];
            locals -= LOCALS;
            break;
        case GET:
            if (sp == STACK_LIMIT)
                FAIL("stack overflow");
            stack[sp++] = locals[in->arg];
            break;
        case SET:
            if (sp < 1)
                FAIL("stack underflow");
            locals[in->arg] = stack[--sp];
            break;
        case LOAD:
            if (sp < 1)
                FAIL("stack underflow");
            if (stack[sp - 1] < 0 || stack[sp - 1] >= MEMORY)
                FAIL("bad address");
            stack[sp - 1] = memory[stack[sp - 1]];
            break;
        case STORE:
            if (sp < 2)
                FAIL("stack underflow");
            if (stack[sp - 1] < 0 || stack[sp - 1] >= MEMORY)
                FAIL("bad address");
            memory[stack[sp - 1]] = stack[sp - 2];
            sp -= 2;
            break;
        case READ:
            if (cursor == data_length)
                FAIL("end of input");
            if (sp == STACK_LIMIT)
                FAIL("stack overflow");
            stack[sp++] = data[cursor++];
            break;
        case PRINT:
            if (sp < 1)
                FAIL("stack underflow");
            write_int(out, stack[--sp]);
            break;
        case HALT:
            return (Error){nullptr, 0};
        }
    }
failed:
    return (Error){message, code[pc - 1].line};
}

/* Parses the whitespace-separated integers of the data section. */
static int64_t *parse_data(const char *p, const char *end, size_t *count)
{
    size_t capacity = 1024;
    size_t length = 0;
    int64_t *values = checked(malloc(capacity * sizeof *values));
    for (;;) {
        while (p < end && (*p == ' ' || *p == '\t' || *p == '\n'))
            p++;
        if (p == end)
            break;
        const char *start = p;
        while (p < end && *p != ' ' && *p != '\t' && *p != '\n')
            p++;
        if (length == capacity) {
            capacity *= 2;
            values = checked(realloc(values, capacity * sizeof *values));
        }
        parse_integer((Slice){start, (size_t)(p - start)}, &values[length++]);
    }
    *count = length;
    return values;
}

int main(void)
{
    size_t length;
    char *input = read_all(stdin, &length);
    const char *end = input + length;

    /* The source ends at the first line that is exactly ".input", and the data follows it. */
    const char *source_end = end;
    const char *data = end;
    size_t line_count = 0;
    for (const char *line = input; line < end;) {
        const char *newline = memchr(line, '\n', (size_t)(end - line));
        const char *line_end = newline != nullptr ? newline : end;
        if (line_end - line == 6 && memcmp(line, ".input", 6) == 0) {
            source_end = line;
            data = line_end < end ? line_end + 1 : end;
            break;
        }
        line_count++;
        line = newline != nullptr ? newline + 1 : end;
    }

    Output out = {};
    Program program;
    Error error;
    if (assemble(input, source_end, line_count, &program, &error)) {
        size_t data_length;
        int64_t *values = parse_data(data, end, &data_length);
        error = execute(program.code, values, data_length, &out);
        free(values);
        free(program.code);
    }
    if (error.message != nullptr) {
        char line[64];
        int n = snprintf(line, sizeof line, " at line %u\n", error.line);
        write_bytes(&out, "error: ", 7);
        write_bytes(&out, error.message, strlen(error.message));
        write_bytes(&out, line, (size_t)n);
    }
    flush(&out);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
