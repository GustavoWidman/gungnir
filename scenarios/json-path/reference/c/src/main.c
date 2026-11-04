#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef enum { SCALAR, ARRAY, OBJECT } Kind;

/* A value of the document. Nodes are stored in document order, so a node's index orders it,
   and its descendants are exactly the nodes from its index + 1 up to end. */
typedef struct {
    Kind kind;
    /* A scalar's printed text in Document.text, or a container's children in Document.kids. */
    uint32_t start, length;
    /* The member name printed as a string, in Document.text, when the parent is an object. */
    uint32_t name_start, name_length;
    uint32_t end;
} Node;

typedef struct {
    unsigned char *data;
    size_t size, capacity;
} Bytes;

typedef struct {
    uint32_t *data;
    size_t size, capacity;
} Ids;

typedef struct {
    Node *data;
    size_t size, capacity;
} Nodes;

typedef struct {
    Nodes nodes;
    Ids kids;
    Bytes text;
} Document;

typedef enum { NAME, WILD, INDEX, SLICE } SelectorKind;

/* A name is the member name printed as a string. Printing is one-to-one on code unit
   sequences, so two names are equal exactly when their printed forms are. */
typedef struct {
    bool descendant;
    SelectorKind kind;
    Bytes name;
    int64_t index, start, end, step;
    bool has_start, has_end;
} Segment;

typedef struct {
    Segment *data;
    size_t size, capacity;
} Segments;

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

#define RESERVE(vec, extra)                                                                  \
    do {                                                                                     \
        if ((vec)->size + (extra) > (vec)->capacity) {                                       \
            size_t want = (vec)->capacity ? (vec)->capacity * 2 : 64;                        \
            while (want < (vec)->size + (extra))                                             \
                want *= 2;                                                                   \
            (vec)->data = checked(realloc((vec)->data, want * sizeof *(vec)->data));         \
            (vec)->capacity = want;                                                          \
        }                                                                                    \
    } while (0)

#define PUSH(vec, value)                                                                     \
    do {                                                                                     \
        RESERVE(vec, 1);                                                                     \
        (vec)->data[(vec)->size++] = (value);                                                \
    } while (0)

static void append(Bytes *out, const void *data, size_t length)
{
    RESERVE(out, length);
    memcpy(out->data + out->size, data, length);
    out->size += length;
}

static void put_unit(Bytes *out, uint32_t unit)
{
    static const char hex[] = "0123456789abcdef";
    switch (unit) {
    case 0x22: append(out, "\\\"", 2); return;
    case 0x5C: append(out, "\\\\", 2); return;
    case 0x08: append(out, "\\b", 2); return;
    case 0x0C: append(out, "\\f", 2); return;
    case 0x0A: append(out, "\\n", 2); return;
    case 0x0D: append(out, "\\r", 2); return;
    case 0x09: append(out, "\\t", 2); return;
    default: break;
    }
    if (unit >= 0x20 && unit <= 0x7E) {
        PUSH(out, (unsigned char)unit);
        return;
    }
    unsigned char escape[6] = {'\\', 'u', hex[unit >> 12 & 15], hex[unit >> 8 & 15], hex[unit >> 4 & 15],
                               hex[unit & 15]};
    append(out, escape, 6);
}

static int hex_value(unsigned char c)
{
    if (c >= '0' && c <= '9')
        return c - '0';
    if (c >= 'a' && c <= 'f')
        return c - 'a' + 10;
    if (c >= 'A' && c <= 'F')
        return c - 'A' + 10;
    return -1;
}

/* Parses the string literal at src[pos], which must be '"', and appends it to out as it
   prints. Returns the position after the closing quote, or SIZE_MAX if the literal is invalid. */
static size_t parse_string(const unsigned char *src, size_t len, size_t pos, Bytes *out)
{
    PUSH(out, '"');
    pos++;
    for (;;) {
        size_t run = pos;
        while (run < len && src[run] >= 0x20 && src[run] < 0x7F && src[run] != '"' && src[run] != '\\')
            run++;
        append(out, src + pos, run - pos);
        pos = run;
        if (pos >= len)
            return SIZE_MAX;
        unsigned char c = src[pos++];
        if (c == '"')
            break;
        if (c == '\\') {
            if (pos >= len)
                return SIZE_MAX;
            uint32_t unit;
            switch (src[pos]) {
            case '"': unit = 0x22; break;
            case '\\': unit = 0x5C; break;
            case '/': unit = 0x2F; break;
            case 'b': unit = 0x08; break;
            case 'f': unit = 0x0C; break;
            case 'n': unit = 0x0A; break;
            case 'r': unit = 0x0D; break;
            case 't': unit = 0x09; break;
            case 'u':
                if (pos + 5 > len)
                    return SIZE_MAX;
                unit = 0;
                for (size_t i = pos + 1; i < pos + 5; i++) {
                    int digit = hex_value(src[i]);
                    if (digit < 0)
                        return SIZE_MAX;
                    unit = unit << 4 | (uint32_t)digit;
                }
                pos += 4;
                break;
            default: return SIZE_MAX;
            }
            pos++;
            put_unit(out, unit);
        } else if (c == 0x7F) {
            put_unit(out, c);
        } else if (c < 0x20) {
            return SIZE_MAX;
        } else {
            size_t count;
            unsigned char low = 0x80, high = 0xBF;
            if (c >= 0xC2 && c <= 0xDF)
                count = 1;
            else if (c == 0xE0)
                count = 2, low = 0xA0;
            else if ((c >= 0xE1 && c <= 0xEC) || c == 0xEE || c == 0xEF)
                count = 2;
            else if (c == 0xED)
                count = 2, high = 0x9F;
            else if (c == 0xF0)
                count = 3, low = 0x90;
            else if (c >= 0xF1 && c <= 0xF3)
                count = 3;
            else if (c == 0xF4)
                count = 3, high = 0x8F;
            else
                return SIZE_MAX;
            if (pos + count > len || src[pos] < low || src[pos] > high)
                return SIZE_MAX;
            uint32_t point = c & (0x3Fu >> count);
            for (size_t i = pos; i < pos + count; i++) {
                if (src[i] < 0x80 || src[i] > 0xBF)
                    return SIZE_MAX;
                point = point << 6 | (src[i] & 0x3Fu);
            }
            pos += count;
            if (point >= 0x10000) {
                put_unit(out, 0xD800 + ((point - 0x10000) >> 10));
                put_unit(out, 0xDC00 + ((point - 0x10000) & 0x3FF));
            } else {
                put_unit(out, point);
            }
        }
    }
    PUSH(out, '"');
    return pos;
}

/* Parses -?(0|[1-9][0-9]*) at src[pos]. Returns its end, or SIZE_MAX if there is none, and
   sets *fits to whether its value fits in 64 bits. */
static size_t parse_integer(const unsigned char *src, size_t len, size_t pos, bool *fits)
{
    bool negative = pos < len && src[pos] == '-';
    pos += negative;
    size_t first = pos;
    if (pos < len && src[pos] == '0')
        pos++;
    else if (pos < len && src[pos] >= '1' && src[pos] <= '9')
        while (pos < len && src[pos] >= '0' && src[pos] <= '9')
            pos++;
    else
        return SIZE_MAX;
    const char *limit = negative ? "9223372036854775808" : "9223372036854775807";
    size_t digits = pos - first;
    *fits = digits < 19 || (digits == 19 && memcmp(src + first, limit, 19) <= 0);
    return pos;
}

typedef struct {
    const unsigned char *src;
    size_t len, pos;
    Document doc;
    Ids pending;
} Parser;

static void skip_whitespace(Parser *p)
{
    while (p->pos < p->len &&
           (p->src[p->pos] == ' ' || p->src[p->pos] == '\t' || p->src[p->pos] == '\n' || p->src[p->pos] == '\r'))
        p->pos++;
}

static bool eat(Parser *p, unsigned char c)
{
    skip_whitespace(p);
    if (p->pos < p->len && p->src[p->pos] == c) {
        p->pos++;
        return true;
    }
    return false;
}

static bool parse_text_string(Parser *p, uint32_t *start, uint32_t *length)
{
    size_t before = p->doc.text.size;
    p->pos = parse_string(p->src, p->len, p->pos, &p->doc.text);
    *start = (uint32_t)before;
    *length = (uint32_t)(p->doc.text.size - before);
    return p->pos != SIZE_MAX;
}

static bool parse_value(Parser *p, uint32_t name_start, uint32_t name_length)
{
    skip_whitespace(p);
    if (p->pos >= p->len)
        return false;
    size_t id = p->doc.nodes.size;
    Node node = {.kind = SCALAR, .name_start = name_start, .name_length = name_length};
    PUSH(&p->doc.nodes, node);
    unsigned char c = p->src[p->pos];
    if (c == '[' || c == '{') {
        p->pos++;
        unsigned char close = c == '[' ? ']' : '}';
        node.kind = c == '[' ? ARRAY : OBJECT;
        size_t mark = p->pending.size;
        if (!eat(p, close)) {
            for (;;) {
                uint32_t ns = 0, nl = 0;
                if (node.kind == OBJECT) {
                    skip_whitespace(p);
                    if (p->pos >= p->len || p->src[p->pos] != '"' || !parse_text_string(p, &ns, &nl) ||
                        !eat(p, ':'))
                        return false;
                }
                PUSH(&p->pending, (uint32_t)p->doc.nodes.size);
                if (!parse_value(p, ns, nl))
                    return false;
                if (eat(p, close))
                    break;
                if (!eat(p, ','))
                    return false;
            }
        }
        size_t count = p->pending.size - mark;
        node.start = (uint32_t)p->doc.kids.size;
        node.length = (uint32_t)count;
        if (count > 0) {
            RESERVE(&p->doc.kids, count);
            memcpy(p->doc.kids.data + p->doc.kids.size, p->pending.data + mark, count * sizeof(uint32_t));
            p->doc.kids.size += count;
        }
        p->pending.size = mark;
    } else if (c == '"') {
        if (!parse_text_string(p, &node.start, &node.length))
            return false;
    } else if (c == '-' || (c >= '0' && c <= '9')) {
        bool fits;
        size_t end = parse_integer(p->src, p->len, p->pos, &fits);
        if (end == SIZE_MAX || !fits)
            return false;
        node.start = (uint32_t)p->doc.text.size;
        if (end - p->pos == 2 && p->src[p->pos] == '-' && p->src[p->pos + 1] == '0')
            append(&p->doc.text, "0", 1);
        else
            append(&p->doc.text, p->src + p->pos, end - p->pos);
        node.length = (uint32_t)(p->doc.text.size - node.start);
        p->pos = end;
    } else {
        static const char *const words[] = {"true", "false", "null"};
        const char *word = nullptr;
        for (size_t i = 0; i < 3; i++)
            if (p->len - p->pos >= strlen(words[i]) && memcmp(p->src + p->pos, words[i], strlen(words[i])) == 0)
                word = words[i];
        if (word == nullptr)
            return false;
        node.start = (uint32_t)p->doc.text.size;
        node.length = (uint32_t)strlen(word);
        append(&p->doc.text, word, node.length);
        p->pos += node.length;
    }
    node.end = (uint32_t)p->doc.nodes.size;
    p->doc.nodes.data[id] = node;
    return true;
}

static bool parse_document(const unsigned char *src, size_t len, Document *doc)
{
    Parser p = {.src = src, .len = len};
    RESERVE(&p.doc.text, len);
    bool valid = parse_value(&p, 0, 0);
    if (valid) {
        skip_whitespace(&p);
        valid = p.pos == len;
    }
    free(p.pending.data);
    *doc = p.doc;
    return valid;
}

/* Parses an optional slice bound or index at q[*pos]; returns false if one is malformed. */
static bool parse_bound(const unsigned char *q, size_t len, size_t *pos, int64_t *value, bool *present)
{
    *present = *pos < len && (q[*pos] == '-' || (q[*pos] >= '0' && q[*pos] <= '9'));
    if (!*present)
        return true;
    bool fits;
    size_t end = parse_integer(q, len, *pos, &fits);
    if (end == SIZE_MAX || !fits)
        return false;
    bool negative = q[*pos] == '-';
    uint64_t magnitude = 0;
    for (size_t i = *pos + negative; i < end; i++)
        magnitude = magnitude * 10 + (q[i] - '0');
    *value = negative ? (int64_t)(0 - magnitude) : (int64_t)magnitude;
    *pos = end;
    return true;
}

/* Parses the selector of the bracket that opens at q[pos]; returns the position after ']',
   or SIZE_MAX if the bracket is invalid. */
static size_t parse_bracket(const unsigned char *q, size_t len, size_t pos, Segment *segment)
{
    pos++;
    if (pos < len && q[pos] == '*') {
        segment->kind = WILD;
        pos++;
    } else if (pos < len && q[pos] == '"') {
        segment->kind = NAME;
        pos = parse_string(q, len, pos, &segment->name);
        if (pos == SIZE_MAX)
            return SIZE_MAX;
    } else {
        if (!parse_bound(q, len, &pos, &segment->start, &segment->has_start))
            return SIZE_MAX;
        if (pos < len && q[pos] == ':') {
            pos++;
            segment->kind = SLICE;
            segment->step = 1;
            if (!parse_bound(q, len, &pos, &segment->end, &segment->has_end))
                return SIZE_MAX;
            if (pos < len && q[pos] == ':') {
                pos++;
                bool has_step;
                int64_t step;
                if (!parse_bound(q, len, &pos, &step, &has_step) || (has_step && step < 1))
                    return SIZE_MAX;
                if (has_step)
                    segment->step = step;
            }
        } else {
            if (!segment->has_start)
                return SIZE_MAX;
            segment->kind = INDEX;
            segment->index = segment->start;
        }
    }
    return pos < len && q[pos] == ']' ? pos + 1 : SIZE_MAX;
}

static bool is_name_start(unsigned char c)
{
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_';
}

static void free_segments(Segments *segments)
{
    for (size_t i = 0; i < segments->size; i++)
        free(segments->data[i].name.data);
    free(segments->data);
}

static bool parse_query(const unsigned char *q, size_t len, Segments *segments)
{
    if (len == 0 || q[0] != '$')
        return false;
    for (size_t pos = 1; pos < len;) {
        Segment segment = {0};
        bool dotted = false;
        if (pos + 1 < len && q[pos] == '.' && q[pos + 1] == '.') {
            segment.descendant = dotted = true;
            pos += 2;
        } else if (q[pos] == '.') {
            dotted = true;
            pos++;
        }
        if (pos < len && q[pos] == '[' && (segment.descendant || !dotted)) {
            pos = parse_bracket(q, len, pos, &segment);
        } else if (dotted && pos < len && q[pos] == '*') {
            segment.kind = WILD;
            pos++;
        } else if (dotted && pos < len && is_name_start(q[pos])) {
            size_t start = pos;
            while (pos < len && (is_name_start(q[pos]) || (q[pos] >= '0' && q[pos] <= '9')))
                pos++;
            segment.kind = NAME;
            PUSH(&segment.name, '"');
            append(&segment.name, q + start, pos - start);
            PUSH(&segment.name, '"');
        } else {
            pos = SIZE_MAX;
        }
        PUSH(segments, segment);
        if (pos == SIZE_MAX)
            return false;
    }
    return true;
}

static const uint32_t *kids_of(const Document *doc, const Node *node)
{
    return doc->kids.data + node->start;
}

static size_t kid_count(const Node *node)
{
    return node->kind == SCALAR ? 0 : node->length;
}

static bool name_is(const Document *doc, uint32_t id, const Bytes *name)
{
    const Node *node = &doc->nodes.data[id];
    return node->name_length == name->size && memcmp(doc->text.data + node->name_start, name->data, name->size) == 0;
}

/* Normalizes a slice to its first position, the bound it stays below, and its step. The step
   is capped at length + 1, which picks the same positions and cannot overflow. */
static void slice_range(const Segment *s, size_t length, size_t *start, size_t *end, size_t *step)
{
    int64_t n = (int64_t)length;
    int64_t bounds[2] = {s->has_start ? s->start : 0, s->has_end ? s->end : n};
    for (size_t i = 0; i < 2; i++) {
        if (bounds[i] < 0)
            bounds[i] += n;
        bounds[i] = bounds[i] < 0 ? 0 : bounds[i] > n ? n : bounds[i];
    }
    *start = (size_t)bounds[0];
    *end = (size_t)bounds[1];
    *step = (size_t)(s->step < n + 1 ? s->step : n + 1);
}

static bool index_position(int64_t index, size_t length, size_t *position)
{
    if (index < 0)
        index += (int64_t)length;
    if (index < 0 || index >= (int64_t)length)
        return false;
    *position = (size_t)index;
    return true;
}

static void children(const Document *doc, uint32_t id, const Segment *s, Ids *out)
{
    const Node *node = &doc->nodes.data[id];
    const uint32_t *kids = kids_of(doc, node);
    size_t count = kid_count(node);
    if (node->kind == OBJECT && s->kind == NAME) {
        for (size_t j = 0; j < count; j++)
            if (name_is(doc, kids[j], &s->name))
                PUSH(out, kids[j]);
    } else if (node->kind != SCALAR && s->kind == WILD) {
        for (size_t j = 0; j < count; j++)
            PUSH(out, kids[j]);
    } else if (node->kind == ARRAY && s->kind == INDEX) {
        size_t j;
        if (index_position(s->index, count, &j))
            PUSH(out, kids[j]);
    } else if (node->kind == ARRAY && s->kind == SLICE) {
        size_t start, end, step;
        slice_range(s, count, &start, &end, &step);
        for (size_t j = start; j < end; j += step)
            PUSH(out, kids[j]);
    }
}

/* Appends what the selector picks from id and each of its descendants, in document order. */
static void descend(const Document *doc, uint32_t id, const Segment *s, Ids *out)
{
    const Node *node = &doc->nodes.data[id];
    if (node->kind == SCALAR)
        return;
    const uint32_t *kids = kids_of(doc, node);
    size_t count = node->length, start = 0, end = 0, step = 1;
    if (node->kind == ARRAY && s->kind == INDEX) {
        if (index_position(s->index, count, &start))
            end = start + 1;
    } else if (node->kind == ARRAY && s->kind == SLICE) {
        slice_range(s, count, &start, &end, &step);
    }
    for (size_t j = 0; j < count; j++) {
        bool picked;
        if (s->kind == NAME)
            picked = node->kind == OBJECT && name_is(doc, kids[j], &s->name);
        else if (s->kind == WILD)
            picked = true;
        else
            picked = j >= start && j < end && (j - start) % step == 0;
        if (picked)
            PUSH(out, kids[j]);
        descend(doc, kids[j], s, out);
    }
}

static int compare_ids(const void *a, const void *b)
{
    uint32_t x = *(const uint32_t *)a, y = *(const uint32_t *)b;
    return (x > y) - (x < y);
}

/* The nodes a query selects, each once, in document order. A set holds no node nested in
   another until a descendant segment, and only a nested set can come out unsorted. */
static Ids evaluate(const Document *doc, const Segments *segments)
{
    Ids nodes = {0};
    PUSH(&nodes, 0);
    bool nested = false;
    for (size_t i = 0; i < segments->size; i++) {
        const Segment *s = &segments->data[i];
        Ids out = {0};
        if (s->descendant) {
            uint32_t covered = 0;
            for (size_t k = 0; k < nodes.size; k++) {
                if (nodes.data[k] >= covered) {
                    descend(doc, nodes.data[k], s, &out);
                    covered = doc->nodes.data[nodes.data[k]].end;
                }
            }
            nested = true;
        } else {
            for (size_t k = 0; k < nodes.size; k++)
                children(doc, nodes.data[k], s, &out);
            if (nested && out.size > 1)
                qsort(out.data, out.size, sizeof *out.data, compare_ids);
        }
        free(nodes.data);
        nodes = out;
    }
    return nodes;
}

static void print_node(const Document *doc, uint32_t id, Bytes *out)
{
    const Node *node = &doc->nodes.data[id];
    if (node->kind == SCALAR) {
        append(out, doc->text.data + node->start, node->length);
        return;
    }
    PUSH(out, node->kind == ARRAY ? '[' : '{');
    const uint32_t *kids = kids_of(doc, node);
    for (size_t j = 0; j < node->length; j++) {
        if (j > 0)
            PUSH(out, ',');
        if (node->kind == OBJECT) {
            const Node *kid = &doc->nodes.data[kids[j]];
            append(out, doc->text.data + kid->name_start, kid->name_length);
            PUSH(out, ':');
        }
        print_node(doc, kids[j], out);
    }
    PUSH(out, node->kind == ARRAY ? ']' : '}');
}

static unsigned char *read_all(FILE *stream, size_t *length)
{
    size_t capacity = 1 << 16, size = 0;
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

int main(void)
{
    size_t length;
    unsigned char *input = read_all(stdin, &length);
    const unsigned char *end = input + length, *p = input;
    size_t count = 0;
    for (; p < end && *p != '\n'; p++)
        count = count * 10 + (size_t)(*p - '0');
    p++;
    const unsigned char **queries = checked(malloc((count + 1) * sizeof *queries));
    size_t *query_lengths = checked(malloc((count + 1) * sizeof *query_lengths));
    for (size_t i = 0; i < count; i++) {
        const unsigned char *newline = memchr(p, '\n', (size_t)(end - p));
        queries[i] = p;
        query_lengths[i] = (size_t)(newline - p);
        p = newline + 1;
    }

    Bytes out = {0};
    Document doc;
    if (!parse_document(p, (size_t)(end - p), &doc)) {
        append(&out, "invalid document\n", 17);
    } else {
        for (size_t i = 0; i < count; i++) {
            Segments segments = {0};
            if (!parse_query(queries[i], query_lengths[i], &segments)) {
                append(&out, "invalid query\n", 14);
                free_segments(&segments);
                continue;
            }
            Ids nodes = evaluate(&doc, &segments);
            char line[32];
            append(&out, line, (size_t)snprintf(line, sizeof line, "%zu\n", nodes.size));
            for (size_t k = 0; k < nodes.size; k++) {
                print_node(&doc, nodes.data[k], &out);
                PUSH(&out, '\n');
            }
            free(nodes.data);
            free_segments(&segments);
        }
    }
    fwrite(out.data, 1, out.size, stdout);
    free(out.data);
    free(doc.nodes.data);
    free(doc.kids.data);
    free(doc.text.data);
    free(queries);
    free(query_lengths);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
