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

/* Negative values accumulate downward, so INT64_MIN parses without overflowing. */
static int64_t next_int(const unsigned char **cursor)
{
    const unsigned char *p = *cursor;
    while (isspace(*p))
        p++;
    int64_t value = 0;
    if (*p == '-') {
        for (p++; isdigit(*p); p++)
            value = value * 10 - (*p - '0');
    } else {
        for (; isdigit(*p); p++)
            value = value * 10 + (*p - '0');
    }
    *cursor = p;
    return value;
}

static int ascending(const void *a, const void *b)
{
    int64_t x = *(const int64_t *)a;
    int64_t y = *(const int64_t *)b;
    return (x > y) - (x < y);
}

/* The number of leading elements of sorted that are at most x. */
static size_t count_at_most(const int64_t *sorted, size_t length, int64_t x)
{
    size_t low = 0;
    size_t high = length;
    while (low < high) {
        size_t middle = low + (high - low) / 2;
        if (sorted[middle] <= x)
            low = middle + 1;
        else
            high = middle;
    }
    return low;
}

/* The number of leading elements of sorted that are below x. */
static size_t count_below(const int64_t *sorted, size_t length, int64_t x)
{
    size_t low = 0;
    size_t high = length;
    while (low < high) {
        size_t middle = low + (high - low) / 2;
        if (sorted[middle] < x)
            low = middle + 1;
        else
            high = middle;
    }
    return low;
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

static void write_line(Output *out, size_t value)
{
    char digits[20];
    size_t count = 0;
    do {
        digits[count++] = (char)('0' + value % 10);
        value /= 10;
    } while (value != 0);
    if (sizeof out->data - out->length <= count)
        flush(out);
    while (count > 0)
        out->data[out->length++] = digits[--count];
    out->data[out->length++] = '\n';
}

int main(void)
{
    unsigned char *input = read_all(stdin);
    const unsigned char *cursor = input;
    size_t n = (size_t)next_int(&cursor);
    size_t q = (size_t)next_int(&cursor);

    /* One spare element keeps the pointers valid for qsort when n is 0. */
    int64_t *starts = checked(malloc((n + 1) * sizeof *starts));
    int64_t *ends = checked(malloc((n + 1) * sizeof *ends));
    for (size_t i = 0; i < n; i++) {
        starts[i] = next_int(&cursor);
        ends[i] = next_int(&cursor);
    }
    qsort(starts, n, sizeof *starts, ascending);
    qsort(ends, n, sizeof *ends, ascending);

    Output out = {};
    for (size_t i = 0; i < q; i++) {
        int64_t x = next_int(&cursor);
        /* Intervals containing x: those starting at or before x, minus those ending before x. */
        write_line(&out, count_at_most(starts, n, x) - count_below(ends, n, x));
    }
    flush(&out);

    free(ends);
    free(starts);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
