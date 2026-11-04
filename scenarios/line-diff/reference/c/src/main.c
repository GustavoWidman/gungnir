#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define NONE UINT32_MAX

typedef struct {
    size_t x, a, y, b;
} Hunk;

typedef struct {
    const unsigned char *line; /* nullptr marks an empty slot */
    size_t length;
    uint64_t hash;
    uint32_t id;
} Slot;

/* Numbers lines so that equal lines get equal numbers: an open-addressing hash table with
   linear probing, kept at most half full. */
typedef struct {
    Slot *slots;
    size_t allocated;
    size_t capacity; /* the slots in use, a power of two */
    uint32_t size;
} Interner;

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
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

/* 64-bit FNV-1a. */
static uint64_t hash_line(const unsigned char *line, size_t length)
{
    uint64_t hash = 0xcbf29ce484222325;
    for (size_t i = 0; i < length; i++)
        hash = (hash ^ line[i]) * 0x100000001b3;
    return hash;
}

/* Empties the table and sizes it for up to `lines` distinct lines. */
static void reset(Interner *interner, size_t lines)
{
    size_t capacity = 16;
    while (capacity < 2 * lines)
        capacity *= 2;
    if (capacity > interner->allocated) {
        free(interner->slots);
        interner->slots = checked(malloc(capacity * sizeof *interner->slots));
        interner->allocated = capacity;
    }
    memset(interner->slots, 0, capacity * sizeof *interner->slots);
    interner->capacity = capacity;
    interner->size = 0;
}

static uint32_t intern(Interner *interner, const unsigned char *line, size_t length)
{
    uint64_t hash = hash_line(line, length);
    size_t mask = interner->capacity - 1;
    for (size_t i = hash & mask;; i = (i + 1) & mask) {
        Slot *slot = &interner->slots[i];
        if (slot->line == nullptr) {
            *slot = (Slot){.line = line, .length = length, .hash = hash, .id = interner->size++};
            return slot->id;
        }
        if (slot->hash == hash && slot->length == length && memcmp(slot->line, line, length) == 0)
            return slot->id;
    }
}

static size_t count_lines(const unsigned char *doc, size_t length)
{
    size_t lines = 0;
    for (const unsigned char *p = doc, *end = doc + length; p < end; lines++) {
        const unsigned char *newline = memchr(p, '\n', (size_t)(end - p));
        p = newline == nullptr ? end : newline + 1;
    }
    return lines;
}

/* Writes the line numbers of `doc` to `ids`. A line keeps its line feed, so a last line
   without one differs from the same text with one. */
static void split(Interner *interner, const unsigned char *doc, size_t length, uint32_t *ids)
{
    for (const unsigned char *p = doc, *end = doc + length; p < end;) {
        const unsigned char *newline = memchr(p, '\n', (size_t)(end - p));
        const unsigned char *next = newline == nullptr ? end : newline + 1;
        *ids++ = intern(interner, p, (size_t)(next - p));
        p = next;
    }
}

/* Where round e starts in the flat array of rounds: rounds 0 to e - 1 hold 1 to e entries. */
static size_t offset(size_t e)
{
    return e * (e + 1) / 2;
}

typedef struct {
    uint32_t *data;
    size_t capacity;
} Rounds;

/* Myers' search run from the end of both documents. Entry i of round e is the smallest x on
   diagonal k = n - m - e + 2i such that (x, x - k) is at most e edits from the end, or NONE when
   no point of that diagonal is. Returns the first round that reaches (0, 0), which is the
   shortest script's cost. */
static size_t search_back(const uint32_t *a, size_t n, const uint32_t *b, size_t m, Rounds *rounds)
{
    const int64_t kend = (int64_t)n - (int64_t)m;
    size_t x = n, y = m;
    while (x > 0 && y > 0 && a[x - 1] == b[y - 1]) {
        x--;
        y--;
    }
    rounds->data[0] = (uint32_t)x;
    for (int64_t e = 0;; e++) {
        int64_t at = e - kend;
        if ((at & 1) == 0 && at >= 0 && at <= 2 * e && rounds->data[offset((size_t)e) + (size_t)(at / 2)] == 0)
            return (size_t)e;
        const size_t prev = offset((size_t)e);
        const size_t cur = offset((size_t)e + 1);
        const int64_t next = e + 1;
        if (cur + (size_t)next + 1 > rounds->capacity) {
            while (cur + (size_t)next + 1 > rounds->capacity)
                rounds->capacity *= 2;
            rounds->data = checked(realloc(rounds->data, rounds->capacity * sizeof *rounds->data));
        }
        for (int64_t i = 0; i <= next; i++)
            rounds->data[cur + (size_t)i] = NONE;
        /* Only the diagonals -m <= k <= n cross the grid. */
        int64_t first = next - (int64_t)n > 0 ? (next - (int64_t)n + 1) / 2 : 0;
        int64_t last = ((int64_t)m + next) / 2 < next ? ((int64_t)m + next) / 2 : next;
        for (int64_t i = first; i <= last; i++) {
            const int64_t k = kend - next + 2 * i;
            const int64_t lo = k > 0 ? k : 0;
            int64_t best = INT64_MAX;
            if (i < next && rounds->data[prev + (size_t)i] != NONE) {
                /* Back from diagonal k + 1 over a deletion. */
                int64_t from = (int64_t)rounds->data[prev + (size_t)i] - 1;
                best = from > lo ? from : lo;
            }
            if (i > 0 && rounds->data[prev + (size_t)i - 1] != NONE) {
                /* Back from diagonal k - 1 over an insertion. */
                int64_t from = rounds->data[prev + (size_t)i - 1];
                from = from > lo ? from : lo;
                best = from < best ? from : best;
            }
            if (best != INT64_MAX) {
                size_t sx = (size_t)best, sy = (size_t)(best - k);
                while (sx > 0 && sy > 0 && a[sx - 1] == b[sy - 1]) {
                    sx--;
                    sy--;
                }
                rounds->data[cur + (size_t)i] = (uint32_t)sx;
            }
        }
    }
}

/* Writes the hunks of the first shortest edit script in keep, delete, insert order to
   `hunks`, which has room for n + m of them, and returns how many there are. */
static size_t diff(const uint32_t *a, size_t n, const uint32_t *b, size_t m, Rounds *rounds, Hunk *hunks)
{
    const int64_t kend = (int64_t)n - (int64_t)m;
    size_t e = search_back(a, n, b, m, rounds);
    size_t count = 0;
    size_t x = 0, y = 0;
    for (;;) {
        while (x < n && y < m && a[x] == b[y]) {
            x++;
            y++;
        }
        if (x == n && y == m)
            return count;
        size_t x0 = x, y0 = y;
        do {
            /* Delete when (x + 1, y) is still within e - 1 edits of the end. */
            size_t i = (size_t)(((int64_t)x - (int64_t)y + (int64_t)e - kend) / 2);
            if (x < n && i < e && rounds->data[offset(e - 1) + i] <= x + 1)
                x++;
            else
                y++;
            e--;
        } while (!((x < n && y < m && a[x] == b[y]) || (x == n && y == m)));
        hunks[count++] = (Hunk){x0, x - x0, y0, y - y0};
    }
}

static size_t number(const unsigned char **p)
{
    size_t value = 0;
    for (; **p >= '0' && **p <= '9'; (*p)++)
        value = value * 10 + (size_t)(**p - '0');
    return value;
}

int main(void)
{
    size_t length;
    unsigned char *input = read_all(stdin, &length);
    const unsigned char *p = input;
    size_t pairs = number(&p);
    p++;

    Interner interner = {0};
    Rounds rounds = {.data = checked(malloc(1024 * sizeof(uint32_t))), .capacity = 1024};
    for (size_t pair = 0; pair < pairs; pair++) {
        size_t n_bytes = number(&p);
        p++;
        size_t m_bytes = number(&p);
        p++;
        const unsigned char *first = p, *second = p + n_bytes;
        p = second + m_bytes;

        size_t n = count_lines(first, n_bytes), m = count_lines(second, m_bytes);
        uint32_t *a = checked(malloc((n + m + 1) * sizeof *a));
        uint32_t *b = a + n;
        reset(&interner, n + m);
        split(&interner, first, n_bytes, a);
        split(&interner, second, m_bytes, b);

        Hunk *hunks = checked(malloc((n + m + 1) * sizeof *hunks));
        size_t count = diff(a, n, b, m, &rounds, hunks);
        size_t deleted = 0, inserted = 0;
        for (size_t i = 0; i < count; i++) {
            deleted += hunks[i].a;
            inserted += hunks[i].b;
        }
        printf("%zu %zu %zu\n", deleted, inserted, count);
        for (size_t i = 0; i < count; i++)
            printf("%zu %zu %zu %zu\n", hunks[i].x, hunks[i].a, hunks[i].y, hunks[i].b);
        free(hunks);
        free(a);
    }

    free(rounds.data);
    free(interner.slots);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
