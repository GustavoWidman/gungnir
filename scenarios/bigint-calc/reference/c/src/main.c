#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef uint32_t Limb;

/* Limbs are base 10^9, so reading and printing decimal take linear time. */
#define BASE 1000000000u
#define BASE_DIGITS 9
/* Below this many limbs in the shorter operand, schoolbook multiplication beats Karatsuba. */
#define KARATSUBA_CUTOFF 40

/* A signed integer: little-endian limbs with no high zero limb, so zero has no limbs and is
   never negative. */
typedef struct {
    Limb *limbs;
    size_t len;
    bool negative;
} Int;

typedef enum { NUMBER, ADD, SUB, MUL, DIV, MOD, NEG, POW, OPEN } Kind;

typedef struct {
    Kind kind;
    const char *text; /* the digits of a NUMBER */
    size_t len;
} Item;

typedef struct {
    char *data;
    size_t len, cap;
} Buffer;

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

static Limb *new_limbs(size_t n)
{
    return checked(malloc((n > 0 ? n : 1) * sizeof(Limb)));
}

static void *grow(void *data, size_t *cap, size_t need, size_t size)
{
    if (need <= *cap)
        return data;
    while (*cap < need)
        *cap = *cap ? *cap * 2 : 64;
    return checked(realloc(data, *cap * size));
}

static size_t trim(const Limb *a, size_t n)
{
    while (n > 0 && a[n - 1] == 0)
        n--;
    return n;
}

static int compare(const Limb *a, size_t an, const Limb *b, size_t bn)
{
    if (an != bn)
        return an < bn ? -1 : 1;
    for (size_t i = an; i-- > 0;)
        if (a[i] != b[i])
            return a[i] < b[i] ? -1 : 1;
    return 0;
}

/* r[0, rn) += a[0, an), for an <= rn and a sum that fits in rn limbs. */
static void add_into(Limb *r, size_t rn, const Limb *a, size_t an)
{
    Limb carry = 0;
    size_t i = 0;
    for (; i < an; i++) {
        Limb s = r[i] + a[i] + carry;
        carry = s >= BASE;
        r[i] = carry ? s - BASE : s;
    }
    for (; carry && i < rn; i++) {
        carry = r[i] == BASE - 1;
        r[i] = carry ? 0 : r[i] + 1;
    }
}

/* r[0, rn) -= a[0, an), for an <= rn and r >= a. */
static void sub_into(Limb *r, size_t rn, const Limb *a, size_t an)
{
    Limb borrow = 0;
    size_t i = 0;
    for (; i < an; i++) {
        Limb s = a[i] + borrow;
        borrow = r[i] < s;
        r[i] = borrow ? r[i] + BASE - s : r[i] - s;
    }
    for (; borrow && i < rn; i++) {
        borrow = r[i] == 0;
        r[i] = borrow ? BASE - 1 : r[i] - 1;
    }
}

static void mul_schoolbook(Limb *r, const Limb *a, size_t an, const Limb *b, size_t bn)
{
    memset(r, 0, (an + bn) * sizeof *r);
    for (size_t i = 0; i < an; i++) {
        uint64_t carry = 0;
        for (size_t j = 0; j < bn; j++) {
            uint64_t t = r[i + j] + (uint64_t)a[i] * b[j] + carry;
            r[i + j] = (Limb)(t % BASE);
            carry = t / BASE;
        }
        r[i + bn] = (Limb)carry;
    }
}

/* r[0, an + bn) = a * b. The operands may have high zero limbs and must not overlap r. */
static void mul_into(Limb *r, const Limb *a, size_t an, const Limb *b, size_t bn)
{
    if (an < bn) {
        const Limb *t = a;
        a = b;
        b = t;
        size_t tn = an;
        an = bn;
        bn = tn;
    }
    if (bn < KARATSUBA_CUTOFF) {
        mul_schoolbook(r, a, an, b, bn);
        return;
    }
    if (an >= 2 * bn) {
        /* Unbalanced: multiply b by each bn-limb slice of a and add the products. */
        memset(r, 0, (an + bn) * sizeof *r);
        Limb *t = new_limbs(2 * bn);
        for (size_t i = 0; i < an; i += bn) {
            size_t n = an - i < bn ? an - i : bn;
            mul_into(t, a + i, n, b, bn);
            add_into(r + i, an + bn - i, t, n + bn);
        }
        free(t);
        return;
    }
    /* Karatsuba with a = a1 B^m + a0 and b = b1 B^m + b0, where m < bn <= an < 2 bn. */
    size_t m = an / 2, a1n = an - m, b1n = bn - m;
    size_t san = a1n + 1, sbn = (m > b1n ? m : b1n) + 1, zn = san + sbn;
    Limb *sa = new_limbs(san + sbn + zn);
    Limb *sb = sa + san, *z = sb + sbn;
    memcpy(sa, a + m, a1n * sizeof *sa);
    sa[a1n] = 0;
    add_into(sa, san, a, m);
    memset(sb, 0, sbn * sizeof *sb);
    memcpy(sb, b, m * sizeof *sb);
    add_into(sb, sbn, b + m, b1n);
    mul_into(r, a, m, b, m);
    mul_into(r + 2 * m, a + m, a1n, b + m, b1n);
    mul_into(z, sa, san, sb, sbn);
    sub_into(z, zn, r, 2 * m);
    sub_into(z, zn, r + 2 * m, a1n + b1n);
    add_into(r + m, an + bn - m, z, trim(z, zn));
    free(sa);
}

/* r[0, an + 1) = a * k for a single limb k. */
static void mul_limb(Limb *r, const Limb *a, size_t an, Limb k)
{
    uint64_t carry = 0;
    for (size_t i = 0; i < an; i++) {
        uint64_t t = (uint64_t)a[i] * k + carry;
        r[i] = (Limb)(t % BASE);
        carry = t / BASE;
    }
    r[an] = (Limb)carry;
}

/* q[0, un - vn + 1) = u / v and r[0, vn) = u % v, for un >= vn >= 1 and v without high zero
   limbs: Knuth's Algorithm D (TAOCP 4.3.1). */
static void divmod_into(const Limb *u, size_t un, const Limb *v, size_t vn, Limb *q, Limb *r)
{
    if (vn == 1) {
        uint64_t rem = 0;
        for (size_t i = un; i-- > 0;) {
            uint64_t cur = rem * BASE + u[i];
            q[i] = (Limb)(cur / v[0]);
            rem = cur % v[0];
        }
        r[0] = (Limb)rem;
        return;
    }
    /* Scale both so the divisor's top limb is at least BASE / 2, which keeps each estimated
       quotient limb at most one too large after the two-limb test. */
    Limb d = (Limb)(BASE / ((uint64_t)v[vn - 1] + 1));
    Limb *w = new_limbs(un + 1 + vn + 1);
    Limb *y = w + un + 1;
    mul_limb(w, u, un, d);
    mul_limb(y, v, vn, d);
    uint64_t top = y[vn - 1], next = y[vn - 2];
    for (size_t j = un - vn + 1; j-- > 0;) {
        uint64_t num = (uint64_t)w[j + vn] * BASE + w[j + vn - 1];
        uint64_t qhat = num / top, rhat = num % top;
        while (qhat >= BASE || qhat * next > rhat * BASE + w[j + vn - 2]) {
            qhat--;
            rhat += top;
            if (rhat >= BASE)
                break;
        }
        uint64_t carry = 0;
        int64_t borrow = 0;
        for (size_t i = 0; i < vn; i++) {
            uint64_t p = qhat * y[i] + carry;
            carry = p / BASE;
            int64_t t = (int64_t)w[i + j] - (int64_t)(p % BASE) - borrow;
            borrow = t < 0;
            w[i + j] = (Limb)(t < 0 ? t + BASE : t);
        }
        int64_t t = (int64_t)w[j + vn] - (int64_t)carry - borrow;
        if (t < 0) {
            /* The estimate was one too large, which is rare: add the divisor back. */
            qhat--;
            Limb c = 0;
            for (size_t i = 0; i < vn; i++) {
                Limb s = w[i + j] + y[i] + c;
                c = s >= BASE;
                w[i + j] = c ? s - BASE : s;
            }
            t += c;
        }
        w[j + vn] = (Limb)t;
        q[j] = (Limb)qhat;
    }
    uint64_t rem = 0;
    for (size_t i = vn; i-- > 0;) {
        uint64_t cur = rem * BASE + w[i];
        r[i] = (Limb)(cur / d);
        rem = cur % d;
    }
    free(w);
}

static Int make(Limb *limbs, size_t n, bool negative)
{
    n = trim(limbs, n);
    return (Int){.limbs = limbs, .len = n, .negative = n > 0 && negative};
}

static Int small(Limb value, bool negative)
{
    Limb *limbs = new_limbs(1);
    limbs[0] = value;
    return make(limbs, 1, negative);
}

static Int parse(const char *digits, size_t n)
{
    while (n > 0 && *digits == '0') {
        digits++;
        n--;
    }
    size_t len = (n + BASE_DIGITS - 1) / BASE_DIGITS;
    Limb *limbs = new_limbs(len);
    for (size_t i = 0; i < len; i++) {
        size_t end = n - i * BASE_DIGITS;
        size_t start = end > BASE_DIGITS ? end - BASE_DIGITS : 0;
        Limb limb = 0;
        for (size_t k = start; k < end; k++)
            limb = limb * 10 + (Limb)(digits[k] - '0');
        limbs[i] = limb;
    }
    return make(limbs, len, false);
}

/* a + b, or a - b when subtract is set. */
static Int add(Int a, Int b, bool subtract)
{
    bool bneg = b.negative != subtract;
    if (a.negative == bneg) {
        if (a.len < b.len) {
            Int t = a;
            a = b;
            b = t;
        }
        Limb *r = new_limbs(a.len + 1);
        memcpy(r, a.limbs, a.len * sizeof *r);
        r[a.len] = 0;
        add_into(r, a.len + 1, b.limbs, b.len);
        return make(r, a.len + 1, bneg);
    }
    /* Opposite signs: subtract the smaller magnitude from the larger. */
    bool negative = a.negative;
    if (compare(a.limbs, a.len, b.limbs, b.len) < 0) {
        Int t = a;
        a = b;
        b = t;
        negative = bneg;
    }
    Limb *r = new_limbs(a.len);
    memcpy(r, a.limbs, a.len * sizeof *r);
    sub_into(r, a.len, b.limbs, b.len);
    return make(r, a.len, negative);
}

static Int mul(Int a, Int b)
{
    if (a.len == 0 || b.len == 0)
        return small(0, false);
    Limb *r = new_limbs(a.len + b.len);
    mul_into(r, a.limbs, a.len, b.limbs, b.len);
    return make(r, a.len + b.len, a.negative != b.negative);
}

/* The quotient rounded toward negative infinity, or the remainder that goes with it. b is
   nonzero. */
static Int divide(Int a, Int b, bool remainder)
{
    size_t qn = a.len >= b.len ? a.len - b.len + 1 : 1;
    Limb *q = new_limbs(qn);
    Limb *r = new_limbs(b.len);
    if (a.len < b.len) {
        q[0] = 0;
        memset(r, 0, b.len * sizeof *r);
        memcpy(r, a.limbs, a.len * sizeof *r);
    } else {
        divmod_into(a.limbs, a.len, b.limbs, b.len, q, r);
    }
    size_t rn = trim(r, b.len);
    if (rn > 0 && a.negative != b.negative) {
        /* Truncation rounded toward zero, so step the quotient down by one and move the
           remainder to the divisor's sign: |r| becomes |b| - |r|. */
        Limb one = 1;
        Limb *q1 = new_limbs(qn + 1);
        memcpy(q1, q, qn * sizeof *q1);
        q1[qn] = 0;
        add_into(q1, qn + 1, &one, 1);
        free(q);
        q = q1;
        qn++;
        Limb *r1 = new_limbs(b.len);
        memcpy(r1, b.limbs, b.len * sizeof *r1);
        sub_into(r1, b.len, r, rn);
        free(r);
        r = r1;
    }
    if (remainder) {
        free(q);
        return make(r, b.len, b.negative);
    }
    free(r);
    return make(q, qn, a.negative != b.negative);
}

/* a ^ e for e >= 0. */
static Int power(Int a, Int e)
{
    if (a.len == 0)
        return small(e.len == 0 ? 1 : 0, false);
    if (a.len == 1 && a.limbs[0] == 1)
        return small(1, a.negative && e.len > 0 && (e.limbs[0] & 1));
    /* Any other base keeps the exponent below 664386, so it fits in one limb. */
    Limb n = e.len == 0 ? 0 : e.limbs[0];
    Limb bit = 1;
    while (bit <= n / 2)
        bit <<= 1;
    Int result = small(1, false);
    for (; bit > 0; bit >>= 1) {
        Int squared = mul(result, result);
        free(result.limbs);
        result = squared;
        if (n & bit) {
            Int product = mul(result, a);
            free(result.limbs);
            result = product;
        }
    }
    return result;
}

static void put(Buffer *out, const char *text, size_t n)
{
    out->data = grow(out->data, &out->cap, out->len + n, 1);
    memcpy(out->data + out->len, text, n);
    out->len += n;
}

static void put_int(Buffer *out, Int x)
{
    if (x.len == 0) {
        put(out, "0\n", 2);
        return;
    }
    out->data = grow(out->data, &out->cap, out->len + x.len * BASE_DIGITS + 2, 1);
    char *p = out->data + out->len;
    if (x.negative)
        *p++ = '-';
    char top[BASE_DIGITS];
    int n = 0;
    for (Limb v = x.limbs[x.len - 1]; v > 0; v /= 10)
        top[n++] = (char)('0' + v % 10);
    while (n > 0)
        *p++ = top[--n];
    for (size_t i = x.len - 1; i-- > 0;) {
        Limb v = x.limbs[i];
        for (int k = BASE_DIGITS - 1; k >= 0; k--) {
            p[k] = (char)('0' + v % 10);
            v /= 10;
        }
        p += BASE_DIGITS;
    }
    *p++ = '\n';
    out->len = (size_t)(p - out->data);
}

static int rank(Kind kind)
{
    switch (kind) {
    case ADD:
    case SUB:
        return 1;
    case MUL:
    case DIV:
    case MOD:
        return 2;
    case NEG:
        return 3;
    case POW:
        return 4;
    default:
        return 0;
    }
}

typedef struct {
    Item *items, *ops;
    size_t items_cap, ops_cap;
    Int *values;
    size_t values_cap;
} Scratch;

/* Converts a line to postfix order with the shunting-yard algorithm. Returns the item count,
   or -1 when the line does not match the grammar. */
static ptrdiff_t postfix(const char *line, size_t len, Scratch *s)
{
    /* Each item and each pending operator is a different byte of the line. */
    s->items = grow(s->items, &s->items_cap, len, sizeof(Item));
    s->ops = grow(s->ops, &s->ops_cap, len, sizeof(Item));
    size_t n = 0, depth = 0;
    bool want_operand = true;
    for (size_t i = 0; i < len;) {
        char c = line[i];
        if (c == ' ' || c == '\t') {
            i++;
            continue;
        }
        if (c >= '0' && c <= '9') {
            if (!want_operand)
                return -1;
            size_t start = i;
            while (i < len && line[i] >= '0' && line[i] <= '9')
                i++;
            s->items[n++] = (Item){.kind = NUMBER, .text = line + start, .len = i - start};
            want_operand = false;
            continue;
        }
        i++;
        if (want_operand) {
            if (c == '(')
                s->ops[depth++] = (Item){.kind = OPEN};
            else if (c == '-')
                s->ops[depth++] = (Item){.kind = NEG};
            else
                return -1;
            continue;
        }
        if (c == ')') {
            while (depth > 0 && s->ops[depth - 1].kind != OPEN)
                s->items[n++] = s->ops[--depth];
            if (depth == 0)
                return -1;
            depth--;
            continue;
        }
        Kind kind;
        switch (c) {
        case '+': kind = ADD; break;
        case '-': kind = SUB; break;
        case '*': kind = MUL; break;
        case '/': kind = DIV; break;
        case '%': kind = MOD; break;
        case '^': kind = POW; break;
        default: return -1;
        }
        int r = rank(kind);
        while (depth > 0 && s->ops[depth - 1].kind != OPEN) {
            int top = rank(s->ops[depth - 1].kind);
            if (top < r || (top == r && kind == POW))
                break;
            s->items[n++] = s->ops[--depth];
        }
        s->ops[depth++] = (Item){.kind = kind};
        want_operand = true;
    }
    if (want_operand)
        return -1;
    while (depth > 0) {
        if (s->ops[depth - 1].kind == OPEN)
            return -1;
        s->items[n++] = s->ops[--depth];
    }
    return (ptrdiff_t)n;
}

static void evaluate(const char *line, size_t len, Scratch *s, Buffer *out)
{
    ptrdiff_t count = postfix(line, len, s);
    if (count < 0) {
        put(out, "syntax error\n", 13);
        return;
    }
    s->values = grow(s->values, &s->values_cap, (size_t)count, sizeof(Int));
    size_t depth = 0;
    const char *error = nullptr;
    for (ptrdiff_t i = 0; i < count && error == nullptr; i++) {
        Item item = s->items[i];
        if (item.kind == NUMBER) {
            s->values[depth++] = parse(item.text, item.len);
            continue;
        }
        if (item.kind == NEG) {
            Int *x = &s->values[depth - 1];
            x->negative = x->len > 0 && !x->negative;
            continue;
        }
        Int a = s->values[depth - 2], b = s->values[depth - 1], r;
        switch (item.kind) {
        case ADD: r = add(a, b, false); break;
        case SUB: r = add(a, b, true); break;
        case MUL: r = mul(a, b); break;
        case POW:
            if (b.negative) {
                error = "negative exponent\n";
                continue;
            }
            r = power(a, b);
            break;
        default:
            if (b.len == 0) {
                error = "division by zero\n";
                continue;
            }
            r = divide(a, b, item.kind == MOD);
        }
        free(a.limbs);
        free(b.limbs);
        depth--;
        s->values[depth - 1] = r;
    }
    if (error != nullptr)
        put(out, error, strlen(error));
    else
        put_int(out, s->values[0]);
    while (depth > 0)
        free(s->values[--depth].limbs);
}

static char *read_all(FILE *stream, size_t *length)
{
    size_t cap = 1 << 16, len = 0;
    char *data = checked(malloc(cap));
    while ((len += fread(data + len, 1, cap - len, stream)) == cap) {
        cap *= 2;
        data = checked(realloc(data, cap));
    }
    if (ferror(stream)) {
        perror("reading standard input");
        exit(EXIT_FAILURE);
    }
    *length = len;
    return data;
}

int main(void)
{
    size_t length;
    char *input = read_all(stdin, &length);
    Scratch scratch = {};
    Buffer out = {};
    for (size_t start = 0; start < length;) {
        const char *newline = memchr(input + start, '\n', length - start);
        size_t end = newline != nullptr ? (size_t)(newline - input) : length;
        evaluate(input + start, end - start, &scratch, &out);
        start = end + 1;
    }
    if (out.len > 0 && fwrite(out.data, 1, out.len, stdout) != out.len) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    free(out.data);
    free(scratch.items);
    free(scratch.ops);
    free(scratch.values);
    free(input);
    if (fflush(stdout) != 0) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
