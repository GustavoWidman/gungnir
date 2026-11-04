#include <ctype.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define MAX_CELLS 64

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

/* Reads all of stream and terminates it with a NUL. */
static char *read_all(FILE *stream)
{
    size_t capacity = 1 << 16;
    size_t length = 0;
    char *data = checked(malloc(capacity + 1));
    while ((length += fread(data + length, 1, capacity - length, stream)) == capacity) {
        capacity *= 2;
        data = checked(realloc(data, capacity + 1));
    }
    data[length] = '\0';
    return data;
}

static int next_int(char **cursor)
{
    char *p = *cursor;
    while (isspace((unsigned char)*p))
        p++;
    int value = 0;
    for (; isdigit((unsigned char)*p); p++)
        value = value * 10 + (*p - '0');
    *cursor = p;
    return value;
}

/* The next line, without its line feed. */
static char *next_line(char **cursor)
{
    char *line = *cursor;
    char *end = strchr(line, '\n');
    *end = '\0';
    *cursor = end + 1;
    return line;
}

typedef struct {
    int count;
    int r[MAX_CELLS];
    int c[MAX_CELLS];
} Shape;

/* Shifts the shape to row and column 0 and sorts its cells. */
static void normalize(Shape *s)
{
    int top = s->r[0];
    int left = s->c[0];
    for (int i = 1; i < s->count; i++) {
        top = s->r[i] < top ? s->r[i] : top;
        left = s->c[i] < left ? s->c[i] : left;
    }
    for (int i = 0; i < s->count; i++) {
        s->r[i] -= top;
        s->c[i] -= left;
    }
    for (int i = 1; i < s->count; i++) {
        int r = s->r[i];
        int c = s->c[i];
        int j = i;
        for (; j > 0 && (s->r[j - 1] > r || (s->r[j - 1] == r && s->c[j - 1] > c)); j--) {
            s->r[j] = s->r[j - 1];
            s->c[j] = s->c[j - 1];
        }
        s->r[j] = r;
        s->c[j] = c;
    }
}

static int compare_shapes(const Shape *a, const Shape *b)
{
    if (a->count != b->count)
        return a->count < b->count ? -1 : 1;
    for (int i = 0; i < a->count; i++) {
        if (a->r[i] != b->r[i])
            return a->r[i] < b->r[i] ? -1 : 1;
        if (a->c[i] != b->c[i])
            return a->c[i] < b->c[i] ? -1 : 1;
    }
    return 0;
}

/* Fills out with the distinct shapes the 4 rotations and their mirror images give, the
 * smallest first, and returns how many there are. */
static int orientations(Shape shape, Shape out[8])
{
    int found = 0;
    for (int mirror = 0; mirror < 2; mirror++) {
        for (int turn = 0; turn < 4; turn++) {
            for (int i = 0; i < shape.count; i++) {
                int r = shape.r[i];
                shape.r[i] = shape.c[i];
                shape.c[i] = -r;
            }
            Shape s = shape;
            normalize(&s);
            int seen = 0;
            for (int k = 0; k < found && !seen; k++)
                seen = compare_shapes(&out[k], &s) == 0;
            if (!seen)
                out[found++] = s;
        }
        for (int i = 0; i < shape.count; i++)
            shape.c[i] = -shape.c[i];
    }
    for (int k = 1; k < found; k++) {
        if (compare_shapes(&out[k], &out[0]) < 0) {
            Shape t = out[0];
            out[0] = out[k];
            out[k] = t;
        }
    }
    return found;
}

/* Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
 * covered once all its copies are placed, so copies of a shape are never told apart. */
typedef struct {
    int cells;
    int root;
    int *copies;
    int *left, *right, *up, *down, *column, *size;
    int length, capacity;
} Dlx;

static void grow(Dlx *x, int nodes)
{
    if (x->length + nodes <= x->capacity)
        return;
    while (x->length + nodes > x->capacity)
        x->capacity = x->capacity > 0 ? 2 * x->capacity : 1024;
    int **arrays[] = {&x->left, &x->right, &x->up, &x->down, &x->column};
    for (size_t k = 0; k < sizeof arrays / sizeof *arrays; k++)
        *arrays[k] = checked(realloc(*arrays[k], (size_t)x->capacity * sizeof(int)));
}

static void dlx_init(Dlx *x, int cells, int shapes, int *copies)
{
    int columns = cells + shapes;
    x->cells = cells;
    x->root = columns;
    x->copies = copies;
    x->length = 0;
    x->capacity = 0;
    x->left = x->right = x->up = x->down = x->column = nullptr;
    grow(x, columns + 1);
    x->size = checked(calloc((size_t)columns + 1, sizeof(int)));
    for (int c = 0; c <= columns; c++) {
        x->left[c] = x->right[c] = x->up[c] = x->down[c] = x->column[c] = c;
    }
    x->length = columns + 1;
    /* Only the cell columns and the root form the header ring. */
    for (int c = 0; c < cells; c++) {
        x->right[c] = c + 1 < cells ? c + 1 : x->root;
        x->left[c] = c > 0 ? c - 1 : x->root;
    }
    if (cells > 0) {
        x->right[x->root] = 0;
        x->left[x->root] = cells - 1;
    }
}

static void dlx_free(Dlx *x)
{
    free(x->left);
    free(x->right);
    free(x->up);
    free(x->down);
    free(x->column);
    free(x->size);
}

static void add_row(Dlx *x, const int *columns, int count)
{
    grow(x, count);
    int first = x->length;
    for (int k = 0; k < count; k++) {
        int node = x->length++;
        int col = columns[k];
        x->column[node] = col;
        x->up[node] = x->up[col];
        x->down[node] = col;
        x->down[x->up[col]] = node;
        x->up[col] = node;
        x->size[col]++;
        x->left[node] = k > 0 ? node - 1 : first + count - 1;
        x->right[node] = k + 1 < count ? node + 1 : first;
    }
}

static void cover(Dlx *x, int c)
{
    x->left[x->right[c]] = x->left[c];
    x->right[x->left[c]] = x->right[c];
    for (int i = x->down[c]; i != c; i = x->down[i]) {
        for (int j = x->right[i]; j != i; j = x->right[j]) {
            x->up[x->down[j]] = x->up[j];
            x->down[x->up[j]] = x->down[j];
            x->size[x->column[j]]--;
        }
    }
}

static void uncover(Dlx *x, int c)
{
    for (int i = x->up[c]; i != c; i = x->up[i]) {
        for (int j = x->left[i]; j != i; j = x->left[j]) {
            x->size[x->column[j]]++;
            x->up[x->down[j]] = j;
            x->down[x->up[j]] = j;
        }
    }
    x->left[x->right[c]] = c;
    x->right[x->left[c]] = c;
}

static void place(Dlx *x, int row)
{
    for (int j = x->right[row]; j != row; j = x->right[j]) {
        int col = x->column[j];
        if (col < x->cells)
            cover(x, col);
        else if (--x->copies[col - x->cells] == 0)
            cover(x, col);
    }
}

static void unplace(Dlx *x, int row)
{
    for (int j = x->left[row]; j != row; j = x->left[j]) {
        int col = x->column[j];
        if (col < x->cells)
            uncover(x, col);
        else if (x->copies[col - x->cells]++ == 0)
            uncover(x, col);
    }
}

/* The number of exact covers, choosing at each step the cell with the fewest options. */
static uint64_t count(Dlx *x)
{
    int best = x->right[x->root];
    if (best == x->root)
        return 1;
    for (int c = x->right[best]; c != x->root && x->size[best] > 0; c = x->right[c]) {
        if (x->size[c] < x->size[best])
            best = c;
    }
    if (x->size[best] == 0)
        return 0;
    cover(x, best);
    uint64_t total = 0;
    for (int row = x->down[best]; row != best; row = x->down[row]) {
        place(x, row);
        total += count(x);
        unplace(x, row);
    }
    uncover(x, best);
    return total;
}

int main(void)
{
    char *input = read_all(stdin);
    char *cursor = input;
    int height = next_int(&cursor);
    int width = next_int(&cursor);
    next_line(&cursor);
    int *index = checked(malloc((size_t)height * (size_t)width * sizeof(int)));
    int cells = 0;
    for (int r = 0; r < height; r++) {
        char *line = next_line(&cursor);
        for (int c = 0; c < width; c++)
            index[r * width + c] = line[c] == '.' ? cells++ : -1;
    }

    int pieces = next_int(&cursor);
    next_line(&cursor);
    Shape *shapes = checked(malloc(((size_t)pieces + 1) * sizeof *shapes));
    int *copies = checked(calloc((size_t)pieces + 1, sizeof(int)));
    int kinds = 0;
    int area = 0;
    for (int p = 0; p < pieces; p++) {
        int rows = next_int(&cursor);
        next_int(&cursor);
        next_line(&cursor);
        Shape piece = {};
        for (int r = 0; r < rows; r++) {
            char *line = next_line(&cursor);
            for (int c = 0; line[c] != '\0'; c++) {
                if (line[c] == '#') {
                    piece.r[piece.count] = r;
                    piece.c[piece.count] = c;
                    piece.count++;
                }
            }
        }
        area += piece.count;
        Shape all[8];
        orientations(piece, all);
        int kind = 0;
        while (kind < kinds && compare_shapes(&shapes[kind], &all[0]) != 0)
            kind++;
        if (kind == kinds)
            shapes[kinds++] = all[0];
        copies[kind]++;
    }

    uint64_t found = 0;
    if (area == cells) {
        Dlx x;
        dlx_init(&x, cells, kinds, copies);
        int row[MAX_CELLS + 1];
        for (int kind = 0; kind < kinds; kind++) {
            Shape all[8];
            int n = orientations(shapes[kind], all);
            for (int o = 0; o < n; o++) {
                for (int top = 0; top < height; top++) {
                    for (int left = 0; left < width; left++) {
                        int k = 0;
                        for (; k < all[o].count; k++) {
                            int r = top + all[o].r[k];
                            int c = left + all[o].c[k];
                            if (r >= height || c >= width || index[r * width + c] < 0)
                                break;
                            row[k] = index[r * width + c];
                        }
                        if (k == all[o].count) {
                            row[k] = cells + kind;
                            add_row(&x, row, k + 1);
                        }
                    }
                }
            }
        }
        found = count(&x);
        dlx_free(&x);
        for (int kind = 0; kind < kinds && found > 0; kind++) {
            for (int k = 2; k <= copies[kind]; k++)
                found *= (uint64_t)k;
        }
    }
    free(copies);
    free(shapes);
    free(index);
    free(input);
    printf("%llu\n", (unsigned long long)found);
    return EXIT_SUCCESS;
}
