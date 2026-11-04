#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* A text line has at most 100000 characters, so one line adds at most about 300000 DFA states. */
enum { STATE_LIMIT = 10000 };
static const char ESCAPABLE[] = "\\.*+?|()[]^$-";

typedef enum { CHAR, EPS, BEGIN, END, MATCH } Kind;

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

/* Grows an array of `size`-byte elements to hold at least `needed`, doubling its capacity. */
static void *reserve(void *array, size_t *capacity, size_t needed, size_t size)
{
    if (needed <= *capacity)
        return array;
    while (*capacity < needed)
        *capacity = *capacity ? 2 * *capacity : 64;
    return checked(realloc(array, *capacity * size));
}

static unsigned char *read_all(FILE *stream, size_t *length)
{
    size_t capacity = 1 << 16;
    size_t size = 0;
    unsigned char *data = checked(malloc(capacity));
    while ((size += fread(data + size, 1, capacity - size, stream)) == capacity) {
        capacity *= 2;
        data = checked(realloc(data, capacity));
    }
    if (ferror(stream)) {
        perror("reading standard input");
        exit(EXIT_FAILURE);
    }
    *length = size;
    return data;
}

/* A Thompson NFA. Each node's out-edges form a linked list through edge_next. */
typedef struct {
    Kind *kind;
    uint64_t (*chars)[2];
    int *head;
    size_t nodes, node_capacity;
    int *edge_to, *edge_next;
    size_t edges, edge_capacity;
} Nfa;

static int add_node(Nfa *nfa, Kind kind, const uint64_t chars[2])
{
    if (nfa->nodes == nfa->node_capacity) {
        nfa->node_capacity = nfa->node_capacity ? 2 * nfa->node_capacity : 64;
        nfa->kind = checked(realloc(nfa->kind, nfa->node_capacity * sizeof *nfa->kind));
        nfa->chars = checked(realloc(nfa->chars, nfa->node_capacity * sizeof *nfa->chars));
        nfa->head = checked(realloc(nfa->head, nfa->node_capacity * sizeof *nfa->head));
    }
    nfa->kind[nfa->nodes] = kind;
    nfa->chars[nfa->nodes][0] = chars ? chars[0] : 0;
    nfa->chars[nfa->nodes][1] = chars ? chars[1] : 0;
    nfa->head[nfa->nodes] = -1;
    return (int)nfa->nodes++;
}

static void add_edge(Nfa *nfa, int from, int to)
{
    if (nfa->edges == nfa->edge_capacity) {
        nfa->edge_capacity = nfa->edge_capacity ? 2 * nfa->edge_capacity : 64;
        nfa->edge_to = checked(realloc(nfa->edge_to, nfa->edge_capacity * sizeof *nfa->edge_to));
        nfa->edge_next = checked(realloc(nfa->edge_next, nfa->edge_capacity * sizeof *nfa->edge_next));
    }
    nfa->edge_to[nfa->edges] = to;
    nfa->edge_next[nfa->edges] = nfa->head[from];
    nfa->head[from] = (int)nfa->edges++;
}

/* Builds the NFA while parsing. Every fragment is an (entry, exit) pair whose exit is an EPS
   node that later gets the edges to whatever follows. Each function returns false when the
   pattern is invalid. */
typedef struct {
    const unsigned char *pattern;
    size_t length, pos;
    Nfa nfa;
} Parser;

typedef struct {
    int entry, exit;
} Fragment;

static int peek(const Parser *p)
{
    return p->pos < p->length ? p->pattern[p->pos] : -1;
}

static bool take(Parser *p, int *c)
{
    if (p->pos >= p->length)
        return false;
    *c = p->pattern[p->pos++];
    return true;
}

static bool escaped(Parser *p, int *c)
{
    return take(p, c) && strchr(ESCAPABLE, *c) != nullptr;
}

static void add_range(uint64_t chars[2], int low, int high)
{
    for (int c = low; c <= high; c++)
        chars[c >> 6] |= 1ull << (c & 63);
}

static bool bracket(Parser *p, uint64_t chars[2])
{
    bool negated = peek(p) == '^';
    if (negated)
        p->pos++;
    /* Each body character, and whether it is an unescaped dash. */
    int *body = checked(malloc((p->length + 1) * sizeof *body));
    bool *dash = checked(malloc(p->length + 1));
    size_t count = 0;
    bool ok;
    for (int c; (ok = take(p, &c)) && c != ']';) {
        bool unescaped_dash = c == '-';
        if (c == '\\' && !(ok = escaped(p, &c)))
            break;
        dash[count] = unescaped_dash;
        body[count++] = c;
    }
    ok = ok && count > 0;
    size_t start = 0, stop = count;
    if (ok && dash[0]) {
        add_range(chars, '-', '-');
        start = 1;
    }
    if (ok && stop > start && dash[stop - 1]) {
        add_range(chars, '-', '-');
        stop--;
    }
    for (size_t i = start; ok && i < stop;) {
        if (dash[i]) {
            ok = false;
        } else if (i + 1 < stop && dash[i + 1]) {
            ok = i + 2 < stop && !dash[i + 2] && body[i] <= body[i + 2];
            if (ok)
                add_range(chars, body[i], body[i + 2]);
            i += 3;
        } else {
            add_range(chars, body[i], body[i]);
            i++;
        }
    }
    free(body);
    free(dash);
    if (negated) {
        chars[0] = ~chars[0];
        chars[1] = ~chars[1];
    }
    return ok;
}

static bool alternation(Parser *p, Fragment *out);

static Fragment wrap(Parser *p, Kind kind, const uint64_t chars[2])
{
    int node = add_node(&p->nfa, kind, chars);
    int exit = add_node(&p->nfa, EPS, nullptr);
    add_edge(&p->nfa, node, exit);
    return (Fragment){node, exit};
}

static bool atom(Parser *p, Fragment *out)
{
    int c;
    if (!take(p, &c))
        return false;
    uint64_t chars[2] = {0, 0};
    switch (c) {
    case '(':
        return alternation(p, out) && take(p, &c) && c == ')';
    case '*':
    case '+':
    case '?':
        return false;
    case '^':
        *out = wrap(p, BEGIN, nullptr);
        return true;
    case '$':
        *out = wrap(p, END, nullptr);
        return true;
    case '.':
        chars[0] = chars[1] = UINT64_MAX;
        break;
    case '[':
        if (!bracket(p, chars))
            return false;
        break;
    case '\\':
        if (!escaped(p, &c))
            return false;
        add_range(chars, c, c);
        break;
    default:
        add_range(chars, c, c);
    }
    *out = wrap(p, CHAR, chars);
    return true;
}

static bool item(Parser *p, Fragment *out)
{
    Fragment f;
    if (!atom(p, &f))
        return false;
    switch (peek(p)) {
    case '*': {
        p->pos++;
        int loop = add_node(&p->nfa, EPS, nullptr);
        add_edge(&p->nfa, loop, f.entry);
        add_edge(&p->nfa, f.exit, loop);
        f = (Fragment){loop, loop};
        break;
    }
    case '+':
        p->pos++;
        add_edge(&p->nfa, f.exit, f.entry);
        break;
    case '?': {
        p->pos++;
        int skip = add_node(&p->nfa, EPS, nullptr);
        add_edge(&p->nfa, skip, f.entry);
        add_edge(&p->nfa, skip, f.exit);
        f.entry = skip;
        break;
    }
    }
    *out = f;
    return true;
}

static bool sequence(Parser *p, Fragment *out)
{
    int entry = add_node(&p->nfa, EPS, nullptr);
    int last = entry;
    for (int c = peek(p); c != -1 && c != '|' && c != ')'; c = peek(p)) {
        Fragment f;
        if (!item(p, &f))
            return false;
        add_edge(&p->nfa, last, f.entry);
        last = f.exit;
    }
    *out = (Fragment){entry, last};
    return true;
}

static bool alternation(Parser *p, Fragment *out)
{
    int entry = add_node(&p->nfa, EPS, nullptr);
    int exit = add_node(&p->nfa, EPS, nullptr);
    for (;;) {
        Fragment f;
        if (!sequence(p, &f))
            return false;
        add_edge(&p->nfa, entry, f.entry);
        add_edge(&p->nfa, f.exit, exit);
        if (peek(p) != '|')
            break;
        p->pos++;
    }
    *out = (Fragment){entry, exit};
    return true;
}

/* Parses the whole pattern into p->nfa and stores its start node. The accepting node comes
   last. */
static bool parse(Parser *p, int *start)
{
    Fragment f;
    if (!alternation(p, &f) || p->pos != p->length)
        return false;
    add_edge(&p->nfa, f.exit, add_node(&p->nfa, MATCH, nullptr));
    *start = f.entry;
    return true;
}

static void set_bit(uint64_t *bits, size_t k)
{
    bits[k / 64] |= 1ull << (k % 64);
}

static bool intersects(const uint64_t *a, const uint64_t *b, size_t words)
{
    for (size_t w = 0; w < words; w++)
        if (a[w] & b[w])
            return true;
    return false;
}

/* What the epsilon closures of the NFA need: the kernel numbering and a reusable DFS. */
typedef struct {
    const Nfa *nfa;
    const int *kernel;
    size_t words;
    uint32_t *mark;
    uint32_t stamp;
    int *stack;
    size_t top;
} Closure;

static void visit(Closure *c, int node)
{
    if (c->mark[node] != c->stamp) {
        c->mark[node] = c->stamp;
        c->stack[c->top++] = node;
    }
}

/* The kernel states reachable without consuming a character from `node`, or from the targets
   of its edges when `edges` is set, at a position that is or is not the line's start and end. */
static void closure(Closure *c, int node, bool edges, bool begin, bool end, uint64_t *bits)
{
    const Nfa *nfa = c->nfa;
    c->stamp++;
    c->top = 0;
    memset(bits, 0, c->words * sizeof *bits);
    if (edges)
        for (int e = nfa->head[node]; e >= 0; e = nfa->edge_next[e])
            visit(c, nfa->edge_to[e]);
    else
        visit(c, node);
    while (c->top > 0) {
        int v = c->stack[--c->top];
        Kind kind = nfa->kind[v];
        if (kind == CHAR || kind == MATCH) {
            set_bit(bits, (size_t)c->kernel[v]);
            continue;
        }
        if ((kind == BEGIN && !begin) || (kind == END && !end))
            continue;
        for (int e = nfa->head[v]; e >= 0; e = nfa->edge_next[e])
            visit(c, nfa->edge_to[e]);
    }
}

/* One interned DFA state: what is cached about a bitset over the kernel. */
typedef struct {
    int32_t back[128]; /* reverse transitions by character, -1 until computed */
    int32_t next;      /* the state after every member consumes a character, or -1 */
    int32_t entry;     /* this state intersected with the start closure at an inner position */
    bool accepts, consumes, starts;
} State;

/* Counts leftmost-longest matches in linear time per line.

   The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit
   a node that consumes a character. A reverse pass records at each position which kernel
   states can still reach acceptance from there. Forward scans start only where a match starts
   and keep only states that can still accept, so each scan stops right after its longest
   match. The forward DFA reads the reverse pass's state at each position as its input
   symbol. */
typedef struct {
    size_t words;
    uint64_t *follow; /* per kernel state: the kernel states after it consumes a character */
    int *consumers;   /* the kernel states accepting each character, grouped by character */
    size_t consumer_start[129];
    uint64_t *start[2][2]; /* the start closure, indexed by [at line start][at line end] */
    uint64_t *last;        /* per character: what can accept from the last position */
    uint64_t *scratch;

    uint64_t *bits; /* `words` per state */
    uint64_t *hashes;
    State *states;
    size_t count, capacity;
    int32_t *table; /* open addressing over state ids, -1 when empty */
    size_t table_capacity;

    uint64_t *forward_keys; /* (state << 32 | alive), UINT64_MAX when empty */
    int32_t *forward_values;
    size_t forward_count, forward_capacity;

    int32_t accept_only;
    int32_t last_ids[128];
} Matcher;

static uint64_t mix(uint64_t hash, uint64_t word)
{
    return ((hash << 5 | hash >> 59) ^ word) * 0x517cc1b727220a95;
}

static int32_t intern_scratch(Matcher *m)
{
    size_t words = m->words;
    uint64_t hash = 0;
    for (size_t w = 0; w < words; w++)
        hash = mix(hash, m->scratch[w]);
    size_t mask = m->table_capacity - 1;
    size_t slot = (hash ^ hash >> 29) & mask;
    for (; m->table[slot] >= 0; slot = (slot + 1) & mask) {
        int32_t id = m->table[slot];
        if (m->hashes[id] == hash && memcmp(m->bits + (size_t)id * words, m->scratch, words * 8) == 0)
            return id;
    }
    if (m->count == m->capacity) {
        m->capacity = m->capacity ? 2 * m->capacity : 256;
        m->bits = checked(realloc(m->bits, m->capacity * words * sizeof *m->bits));
        m->hashes = checked(realloc(m->hashes, m->capacity * sizeof *m->hashes));
        m->states = checked(realloc(m->states, m->capacity * sizeof *m->states));
    }
    int32_t id = (int32_t)m->count++;
    const uint64_t *bits = memcpy(m->bits + (size_t)id * words, m->scratch, words * 8);
    m->hashes[id] = hash;
    State *state = &m->states[id];
    memset(state->back, 0xff, sizeof state->back);
    state->next = state->entry = -1;
    state->accepts = bits[0] & 1;
    state->consumes = bits[0] > 1;
    for (size_t w = 1; w < words; w++)
        state->consumes |= bits[w] != 0;
    state->starts = intersects(m->start[0][0], bits, words);
    m->table[slot] = id;
    if (2 * m->count > m->table_capacity) {
        m->table_capacity *= 2;
        m->table = checked(realloc(m->table, m->table_capacity * sizeof *m->table));
        memset(m->table, 0xff, m->table_capacity * sizeof *m->table);
        mask = m->table_capacity - 1;
        for (size_t s = 0; s < m->count; s++) {
            for (slot = (m->hashes[s] ^ m->hashes[s] >> 29) & mask; m->table[slot] >= 0; slot = (slot + 1) & mask) {
            }
            m->table[slot] = (int32_t)s;
        }
    }
    return id;
}

static void reset(Matcher *m)
{
    m->count = 0;
    memset(m->table, 0xff, m->table_capacity * sizeof *m->table);
    m->forward_count = 0;
    memset(m->forward_keys, 0xff, m->forward_capacity * sizeof *m->forward_keys);
    memset(m->scratch, 0, m->words * sizeof *m->scratch);
    set_bit(m->scratch, 0);
    m->accept_only = intern_scratch(m);
    for (int c = 0; c < 128; c++) {
        memcpy(m->scratch, m->last + (size_t)c * m->words, m->words * sizeof *m->scratch);
        m->last_ids[c] = intern_scratch(m);
    }
}

/* The state holding the members of both `a` and state `b`. */
static int32_t meet(Matcher *m, const uint64_t *a, int32_t b)
{
    const uint64_t *bits = m->bits + (size_t)b * m->words;
    for (size_t w = 0; w < m->words; w++)
        m->scratch[w] = a[w] & bits[w];
    return intern_scratch(m);
}

/* The kernel states that can accept from a position holding c, given those that can accept
   from the next position. */
static int32_t step_back(Matcher *m, int32_t state, int c)
{
    size_t words = m->words;
    memset(m->scratch, 0, words * sizeof *m->scratch);
    set_bit(m->scratch, 0);
    const uint64_t *ahead = m->bits + (size_t)state * words;
    for (size_t i = m->consumer_start[c]; i < m->consumer_start[c + 1]; i++) {
        size_t k = (size_t)m->consumers[i];
        if (intersects(m->follow + k * words, ahead, words))
            set_bit(m->scratch, k);
    }
    int32_t next = intern_scratch(m);
    m->states[state].back[c] = next;
    return next;
}

/* The state after every member of `state` consumes a character. The character is implied:
   every state in a pruned set accepts the character at its position. */
static int32_t consume(Matcher *m, int32_t state)
{
    if (m->states[state].next >= 0)
        return m->states[state].next;
    size_t words = m->words;
    memset(m->scratch, 0, words * sizeof *m->scratch);
    for (size_t w = 0; w < words; w++) {
        uint64_t rest = m->bits[(size_t)state * words + w] & (w == 0 ? ~1ull : ~0ull);
        for (; rest != 0; rest &= rest - 1) {
            const uint64_t *follow = m->follow + (w * 64 + (size_t)__builtin_ctzll(rest)) * words;
            for (size_t v = 0; v < words; v++)
                m->scratch[v] |= follow[v];
        }
    }
    int32_t next = intern_scratch(m);
    m->states[state].next = next;
    return next;
}

/* The pruned forward state after `state`, at a position whose reverse state is `alive`. */
static int32_t step(Matcher *m, int32_t state, int32_t alive)
{
    uint64_t key = (uint64_t)state << 32 | (uint32_t)alive;
    size_t mask = m->forward_capacity - 1;
    size_t slot = (key * 0x9e3779b97f4a7c15) >> 32 & mask;
    for (; m->forward_keys[slot] != UINT64_MAX; slot = (slot + 1) & mask)
        if (m->forward_keys[slot] == key)
            return m->forward_values[slot];
    int32_t consumed = consume(m, state);
    int32_t next = meet(m, m->bits + (size_t)consumed * m->words, alive);
    m->forward_keys[slot] = key;
    m->forward_values[slot] = next;
    if (2 * ++m->forward_count > m->forward_capacity) {
        uint64_t *keys = m->forward_keys;
        int32_t *values = m->forward_values;
        size_t old = m->forward_capacity;
        m->forward_capacity *= 2;
        m->forward_keys = checked(malloc(m->forward_capacity * sizeof *m->forward_keys));
        m->forward_values = checked(malloc(m->forward_capacity * sizeof *m->forward_values));
        memset(m->forward_keys, 0xff, m->forward_capacity * sizeof *m->forward_keys);
        mask = m->forward_capacity - 1;
        for (size_t i = 0; i < old; i++) {
            if (keys[i] == UINT64_MAX)
                continue;
            for (slot = (keys[i] * 0x9e3779b97f4a7c15) >> 32 & mask; m->forward_keys[slot] != UINT64_MAX;
                 slot = (slot + 1) & mask) {
            }
            m->forward_keys[slot] = keys[i];
            m->forward_values[slot] = values[i];
        }
        free(keys);
        free(values);
    }
    return next;
}

/* The forward state at a match candidate s of a line of n characters. */
static int32_t entry(Matcher *m, size_t s, size_t n, int32_t alive)
{
    if (s == 0 || s == n)
        return meet(m, m->start[s == 0][s == n], alive);
    if (m->states[alive].entry < 0) {
        int32_t met = meet(m, m->start[0][0], alive);
        m->states[alive].entry = met;
    }
    return m->states[alive].entry;
}

static uint32_t count_matches(Matcher *m, const unsigned char *line, size_t n, int32_t *alive)
{
    if (m->count + m->forward_count > STATE_LIMIT)
        reset(m);
    alive[n] = m->accept_only;
    if (n > 0) {
        int32_t state = alive[n - 1] = m->last_ids[line[n - 1]];
        for (size_t i = n - 1; i-- > 0;) {
            int c = line[i];
            int32_t next = m->states[state].back[c];
            if (next < 0)
                next = step_back(m, state, c);
            alive[i] = state = next;
        }
    }

    uint32_t count = 0;
    for (size_t p = 0; p <= n;) {
        size_t s = p;
        while (0 < s && s < n && !m->states[alive[s]].starts)
            s++;
        int32_t current = entry(m, s, n, alive[s]);
        if (!m->states[current].accepts && !m->states[current].consumes) {
            if (s >= n)
                break;
            p = s + 1;
            continue;
        }
        size_t end = m->states[current].accepts ? s : SIZE_MAX;
        size_t i = s;
        while (m->states[current].consumes) {
            i++;
            current = i == n ? m->accept_only : step(m, current, alive[i]);
            if (m->states[current].accepts)
                end = i;
        }
        count++;
        p = end > s ? end : s + 1;
    }
    return count;
}

static Matcher build_matcher(const Nfa *nfa, int start_node)
{
    size_t nodes = nfa->nodes;
    int *kernel = checked(malloc(nodes * sizeof *kernel));
    size_t size = 1;
    for (size_t v = 0; v < nodes; v++)
        kernel[v] = nfa->kind[v] == CHAR ? (int)size++ : -1;
    kernel[nodes - 1] = 0;
    size_t words = (size + 63) / 64;
    Matcher m = {.words = words};
    Closure c = {
        .nfa = nfa,
        .kernel = kernel,
        .words = words,
        .mark = checked(calloc(nodes, sizeof(uint32_t))),
        .stack = checked(malloc(nodes * sizeof(int))),
    };

    m.follow = checked(calloc(size * words, sizeof *m.follow));
    uint64_t *follow_end = checked(calloc(size * words, sizeof *follow_end));
    m.consumers = checked(malloc(size * 128 * sizeof *m.consumers));
    for (size_t v = 0; v < nodes; v++) {
        if (kernel[v] > 0) {
            size_t k = (size_t)kernel[v];
            closure(&c, (int)v, true, false, false, m.follow + k * words);
            closure(&c, (int)v, true, false, true, follow_end + k * words);
        }
    }
    m.last = checked(calloc(128 * words, sizeof *m.last));
    size_t total = 0;
    for (int ch = 0; ch < 128; ch++) {
        m.consumer_start[ch] = total;
        set_bit(m.last + (size_t)ch * words, 0);
        for (size_t v = 0; v < nodes; v++) {
            if (kernel[v] <= 0 || ch < 32 || ch > 126 || !(nfa->chars[v][ch >> 6] >> (ch & 63) & 1))
                continue;
            size_t k = (size_t)kernel[v];
            m.consumers[total++] = (int)k;
            if (follow_end[k * words] & 1)
                set_bit(m.last + (size_t)ch * words, k);
        }
    }
    m.consumer_start[128] = total;
    for (int b = 0; b < 2; b++) {
        for (int e = 0; e < 2; e++) {
            m.start[b][e] = checked(malloc(words * sizeof *m.start[b][e]));
            closure(&c, start_node, false, b, e, m.start[b][e]);
        }
    }
    m.scratch = checked(malloc(words * sizeof *m.scratch));
    m.table_capacity = 1024;
    m.table = checked(malloc(m.table_capacity * sizeof *m.table));
    m.forward_capacity = 1024;
    m.forward_keys = checked(malloc(m.forward_capacity * sizeof *m.forward_keys));
    m.forward_values = checked(malloc(m.forward_capacity * sizeof *m.forward_values));
    reset(&m);

    free(follow_end);
    free(c.stack);
    free(c.mark);
    free(kernel);
    return m;
}

static void free_matcher(Matcher *m)
{
    for (int b = 0; b < 2; b++)
        for (int e = 0; e < 2; e++)
            free(m->start[b][e]);
    free(m->follow);
    free(m->consumers);
    free(m->last);
    free(m->scratch);
    free(m->bits);
    free(m->hashes);
    free(m->states);
    free(m->table);
    free(m->forward_keys);
    free(m->forward_values);
}

static void free_nfa(Nfa *nfa)
{
    free(nfa->kind);
    free(nfa->chars);
    free(nfa->head);
    free(nfa->edge_to);
    free(nfa->edge_next);
}

int main(void)
{
    size_t length;
    unsigned char *input = read_all(stdin, &length);
    unsigned char *end = input + length;
    unsigned char *newline = memchr(input, '\n', length);
    if (newline == nullptr)
        newline = end;

    Parser parser = {.pattern = input, .length = (size_t)(newline - input)};
    int start;
    if (!parse(&parser, &start)) {
        fputs("invalid\n", stdout);
    } else {
        Matcher matcher = build_matcher(&parser.nfa, start);
        int32_t *alive = nullptr;
        size_t alive_capacity = 0;
        for (unsigned char *line = newline < end ? newline + 1 : end; line < end;) {
            unsigned char *stop = memchr(line, '\n', (size_t)(end - line));
            if (stop == nullptr)
                break;
            size_t n = (size_t)(stop - line);
            alive = reserve(alive, &alive_capacity, n + 1, sizeof *alive);
            printf("%" PRIu32 "\n", count_matches(&matcher, line, n, alive));
            line = stop + 1;
        }
        free(alive);
        free_matcher(&matcher);
    }
    free_nfa(&parser.nfa);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
