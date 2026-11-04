#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    const unsigned char *word;
    size_t length; /* 0 marks an empty slot, since every word has a letter */
    uint64_t hash;
    uint32_t count;
} Entry;

/* An open-addressing hash table with linear probing, kept at most half full. */
typedef struct {
    Entry *slots;
    size_t capacity; /* a power of two */
    size_t size;
} Counts;

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

/* Words are ASCII letters by definition. isalpha and tolower would also consult the locale,
   which made them cost a third of the run. */
static bool is_letter(unsigned char c)
{
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z');
}

static unsigned char to_lower(unsigned char c)
{
    return c >= 'A' && c <= 'Z' ? (unsigned char)(c - 'A' + 'a') : c;
}

/* 64-bit FNV-1a. */
static uint64_t hash_word(const unsigned char *word, size_t length)
{
    uint64_t hash = 0xcbf29ce484222325;
    for (size_t i = 0; i < length; i++)
        hash = (hash ^ word[i]) * 0x100000001b3;
    return hash;
}

static Entry *find_slot(Entry *slots, size_t capacity, const unsigned char *word, size_t length,
                        uint64_t hash)
{
    size_t mask = capacity - 1;
    for (size_t i = hash & mask;; i = (i + 1) & mask) {
        Entry *slot = &slots[i];
        if (slot->length == 0 || (slot->hash == hash && slot->length == length &&
                                  memcmp(slot->word, word, length) == 0))
            return slot;
    }
}

static void grow(Counts *counts)
{
    size_t capacity = counts->capacity * 2;
    Entry *slots = checked(calloc(capacity, sizeof *slots));
    for (size_t i = 0; i < counts->capacity; i++) {
        Entry *old = &counts->slots[i];
        if (old->length != 0)
            *find_slot(slots, capacity, old->word, old->length, old->hash) = *old;
    }
    free(counts->slots);
    counts->slots = slots;
    counts->capacity = capacity;
}

static void count_word(Counts *counts, const unsigned char *word, size_t length)
{
    uint64_t hash = hash_word(word, length);
    Entry *slot = find_slot(counts->slots, counts->capacity, word, length, hash);
    if (slot->length != 0) {
        slot->count++;
        return;
    }
    *slot = (Entry){.word = word, .length = length, .hash = hash, .count = 1};
    if (++counts->size * 2 > counts->capacity)
        grow(counts);
}

static int more_frequent_first(const void *a, const void *b)
{
    const Entry *x = a;
    const Entry *y = b;
    if (x->count != y->count)
        return x->count > y->count ? -1 : 1;
    int order = memcmp(x->word, y->word, x->length < y->length ? x->length : y->length);
    if (order != 0)
        return order;
    return (x->length > y->length) - (x->length < y->length);
}

int main(void)
{
    size_t length;
    unsigned char *input = read_all(stdin, &length);
    unsigned char *end = input + length;
    unsigned char *newline = memchr(input, '\n', length);
    size_t k = 0;
    for (const unsigned char *p = input; p < end && *p >= '0' && *p <= '9'; p++)
        k = k * 10 + (size_t)(*p - '0');

    Counts counts = {.slots = checked(calloc(1024, sizeof(Entry))), .capacity = 1024};
    for (unsigned char *p = newline == nullptr ? end : newline + 1; p < end;) {
        while (p < end && !is_letter(*p))
            p++;
        unsigned char *word = p;
        for (; p < end && is_letter(*p); p++)
            *p = to_lower(*p);
        if (p > word)
            count_word(&counts, word, (size_t)(p - word));
    }

    /* Move the occupied slots to the front, then rank them. */
    Entry *ranked = counts.slots;
    size_t distinct = 0;
    for (size_t i = 0; i < counts.capacity; i++)
        if (counts.slots[i].length != 0)
            ranked[distinct++] = counts.slots[i];
    qsort(ranked, distinct, sizeof *ranked, more_frequent_first);

    for (size_t i = 0; i < distinct && i < k; i++)
        printf("%.*s %" PRIu32 "\n", (int)ranked[i].length, (const char *)ranked[i].word,
               ranked[i].count);

    free(counts.slots);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
