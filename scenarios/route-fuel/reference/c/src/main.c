#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

/* Reads all of stream and terminates it with a NUL, which stops every parsing loop. */
static unsigned char *read_all(FILE *stream)
{
    size_t capacity = 1 << 16;
    size_t length = 0;
    unsigned char *data = checked(malloc(capacity + 1));
    while ((length += fread(data + length, 1, capacity - length, stream)) == capacity) {
        capacity *= 2;
        data = checked(realloc(data, capacity + 1));
    }
    if (ferror(stream)) {
        perror("reading standard input");
        exit(EXIT_FAILURE);
    }
    data[length] = '\0';
    return data;
}

static int64_t next_int(const unsigned char **cursor)
{
    const unsigned char *p = *cursor;
    while (isspace(*p))
        p++;
    int64_t value = 0;
    for (; isdigit(*p); p++)
        value = value * 10 + (*p - '0');
    *cursor = p;
    return value;
}

typedef struct {
    int32_t to;
    int32_t fuel;
    int64_t time;
} Road;

typedef struct {
    int64_t cost;
    int64_t time;
    int32_t place;
    int32_t fuel;
} State;

static bool before(const State *a, const State *b)
{
    return a->cost != b->cost ? a->cost < b->cost : a->time < b->time;
}

/* A binary min-heap of states ordered by (cost, time). */
typedef struct {
    State *items;
    size_t length;
    size_t capacity;
} Heap;

static void heap_push(Heap *heap, State state)
{
    if (heap->length == heap->capacity) {
        heap->capacity = heap->capacity ? 2 * heap->capacity : 1024;
        heap->items = checked(realloc(heap->items, heap->capacity * sizeof *heap->items));
    }
    size_t i = heap->length++;
    while (i > 0 && before(&state, &heap->items[(i - 1) / 2])) {
        heap->items[i] = heap->items[(i - 1) / 2];
        i = (i - 1) / 2;
    }
    heap->items[i] = state;
}

static State heap_pop(Heap *heap)
{
    State top = heap->items[0];
    State last = heap->items[--heap->length];
    size_t i = 0;
    for (;;) {
        size_t child = 2 * i + 1;
        if (child >= heap->length)
            break;
        if (child + 1 < heap->length && before(&heap->items[child + 1], &heap->items[child]))
            child++;
        if (!before(&heap->items[child], &last))
            break;
        heap->items[i] = heap->items[child];
        i = child;
    }
    heap->items[i] = last;
    return top;
}

typedef struct {
    int32_t capacity;
    const int64_t *price; /* -1 where there is no station */
    const size_t *first;  /* the roads out of place v are roads[first[v]] to roads[first[v + 1] - 1] */
    const Road *roads;
    int32_t *most; /* the most fuel each place was expanded with during trip trip[place] */
    int32_t *trip;
    Heap heap;
} Search;

static int32_t expanded(const Search *s, int32_t id, int32_t place)
{
    return s->trip[place] == id ? s->most[place] : -1;
}

/*
 * Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step. A state
 * is dominated once its place was expanded with at least as much fuel, since that earlier state
 * was no costlier and no slower.
 */
static bool best_trip(Search *s, int32_t id, int32_t start, int32_t fuel, int32_t target, State *best)
{
    s->heap.length = 0;
    heap_push(&s->heap, (State){0, 0, start, fuel});
    while (s->heap.length > 0) {
        State cur = heap_pop(&s->heap);
        if (expanded(s, id, cur.place) >= cur.fuel)
            continue;
        if (cur.place == target) {
            *best = cur;
            return true;
        }
        s->trip[cur.place] = id;
        s->most[cur.place] = cur.fuel;
        if (s->price[cur.place] >= 0 && cur.fuel < s->capacity)
            heap_push(&s->heap, (State){cur.cost + s->price[cur.place], cur.time, cur.place, cur.fuel + 1});
        for (size_t r = s->first[cur.place]; r < s->first[cur.place + 1]; r++) {
            const Road *road = &s->roads[r];
            int32_t left = cur.fuel - road->fuel;
            if (left >= 0 && expanded(s, id, road->to) < left)
                heap_push(&s->heap, (State){cur.cost, cur.time + road->time, road->to, left});
        }
    }
    return false;
}

int main(void)
{
    unsigned char *input = read_all(stdin);
    const unsigned char *cursor = input;
    int32_t n = (int32_t)next_int(&cursor);
    int32_t capacity = (int32_t)next_int(&cursor);
    size_t stations = (size_t)next_int(&cursor);
    size_t m = (size_t)next_int(&cursor);
    size_t q = (size_t)next_int(&cursor);

    int64_t *price = checked(malloc((size_t)(n + 1) * sizeof *price));
    for (int32_t v = 0; v <= n; v++)
        price[v] = -1;
    for (size_t i = 0; i < stations; i++) {
        int64_t place = next_int(&cursor);
        price[place] = next_int(&cursor);
    }

    /* Roads that need more than the capacity can never be driven, so they are dropped. */
    int64_t (*raw)[4] = checked(malloc((m + 1) * sizeof *raw));
    size_t *first = checked(calloc((size_t)n + 2, sizeof *first));
    size_t kept = 0;
    for (size_t i = 0; i < m; i++) {
        for (int k = 0; k < 4; k++)
            raw[kept][k] = next_int(&cursor);
        if (raw[kept][2] <= capacity) {
            first[raw[kept][0] + 1]++;
            kept++;
        }
    }
    for (int32_t v = 1; v <= n + 1; v++)
        first[v] += first[v - 1];
    Road *roads = checked(malloc((kept + 1) * sizeof *roads));
    size_t *fill = checked(malloc((size_t)(n + 1) * sizeof *fill));
    for (int32_t v = 0; v <= n; v++)
        fill[v] = first[v];
    for (size_t i = 0; i < kept; i++)
        roads[fill[raw[i][0]]++] = (Road){(int32_t)raw[i][1], (int32_t)raw[i][2], raw[i][3]};
    free(fill);
    free(raw);

    Search search = {
        .capacity = capacity,
        .price = price,
        .first = first,
        .roads = roads,
        .most = checked(malloc((size_t)(n + 1) * sizeof(int32_t))),
        .trip = checked(malloc((size_t)(n + 1) * sizeof(int32_t))),
    };
    for (int32_t v = 0; v <= n; v++)
        search.trip[v] = -1;

    for (size_t i = 0; i < q; i++) {
        int32_t start = (int32_t)next_int(&cursor);
        int32_t fuel = (int32_t)next_int(&cursor);
        int32_t target = (int32_t)next_int(&cursor);
        State best;
        if (best_trip(&search, (int32_t)i, start, fuel, target, &best))
            printf("%lld %lld\n", (long long)best.cost, (long long)best.time);
        else
            fputs("unreachable\n", stdout);
    }

    free(search.heap.items);
    free(search.trip);
    free(search.most);
    free(roads);
    free(first);
    free(price);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
