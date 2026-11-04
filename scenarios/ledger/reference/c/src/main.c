#include <stddef.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* Bytes owned elsewhere: a string literal, or the input buffer, which lives until exit. */
typedef struct {
    const char *data;
    size_t length;
} Slice;

#define LITERAL(text) {text, sizeof text - 1}

static void *checked(void *pointer)
{
    if (pointer == nullptr) {
        fputs("out of memory\n", stderr);
        exit(EXIT_FAILURE);
    }
    return pointer;
}

/* Returns the array data, regrown to double capacity if it has no room for element length. */
static void *reserve(void *data, size_t *capacity, size_t length, size_t size)
{
    if (length < *capacity)
        return data;
    *capacity = *capacity == 0 ? 16 : *capacity * 2;
    return checked(realloc(data, *capacity * size));
}

static char *read_all(FILE *stream, size_t *length)
{
    size_t capacity = 1 << 16;
    size_t size = 0;
    char *data = checked(malloc(capacity));
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

static bool equal(Slice a, Slice b)
{
    return a.length == b.length && memcmp(a.data, b.data, a.length) == 0;
}

/* Removes the text up to the next separator, and the separator, from the front of text. */
static Slice take(Slice *text, char separator)
{
    const char *found = memchr(text->data, separator, text->length);
    Slice taken = {text->data, found == nullptr ? text->length : (size_t)(found - text->data)};
    size_t skipped = found == nullptr ? taken.length : taken.length + 1;
    text->data += skipped;
    text->length -= skipped;
    return taken;
}

static Slice take_word(Slice *text)
{
    return take(text, ' ');
}

static int64_t take_int(Slice *text)
{
    Slice word = take_word(text);
    int64_t value = 0;
    for (size_t i = 0; i < word.length; i++)
        value = value * 10 + (word.data[i] - '0');
    return value;
}

enum { NONE = UINT32_MAX };

typedef struct {
    Slice key; /* an empty key marks an empty slot, since every name has a character */
    uint64_t hash;
    uint32_t value;
} Slot;

/* An open-addressing hash table with linear probing, kept at most half full, from a name in the
   input to an index. */
typedef struct {
    Slot *slots;
    size_t capacity; /* a power of two */
    size_t size;
} Table;

/* 64-bit FNV-1a. */
static uint64_t hash_of(Slice key)
{
    uint64_t hash = 0xcbf29ce484222325;
    for (size_t i = 0; i < key.length; i++)
        hash = (hash ^ (unsigned char)key.data[i]) * 0x100000001b3;
    return hash;
}

static Slot *find_slot(Slot *slots, size_t capacity, Slice key, uint64_t hash)
{
    size_t mask = capacity - 1;
    for (size_t i = hash & mask;; i = (i + 1) & mask) {
        Slot *slot = &slots[i];
        if (slot->key.length == 0 || (slot->hash == hash && equal(slot->key, key)))
            return slot;
    }
}

static Table new_table(void)
{
    enum { INITIAL_CAPACITY = 1024 };
    return (Table){.slots = checked(calloc(INITIAL_CAPACITY, sizeof(Slot))),
                   .capacity = INITIAL_CAPACITY};
}

/* The value stored for key, or NONE. */
static uint32_t table_get(const Table *table, Slice key)
{
    const Slot *slot = find_slot(table->slots, table->capacity, key, hash_of(key));
    return slot->key.length == 0 ? NONE : slot->value;
}

/* Adds a key the table does not hold yet. */
static void table_add(Table *table, Slice key, uint32_t value)
{
    uint64_t hash = hash_of(key);
    *find_slot(table->slots, table->capacity, key, hash) = (Slot){key, hash, value};
    if (++table->size * 2 <= table->capacity)
        return;
    size_t capacity = table->capacity * 2;
    Slot *slots = checked(calloc(capacity, sizeof *slots));
    for (size_t i = 0; i < table->capacity; i++) {
        Slot *old = &table->slots[i];
        if (old->key.length != 0)
            *find_slot(slots, capacity, old->key, old->hash) = *old;
    }
    free(table->slots);
    table->slots = slots;
    table->capacity = capacity;
}

typedef enum {
    OK,
    OUT_OF_ORDER,
    ID_CONFLICT,
    ACCOUNT_EXISTS,
    NO_ACCOUNT,
    ACCOUNT_CLOSED,
    INSUFFICIENT_FUNDS,
    SAME_ACCOUNT,
    NO_HOLD,
    HOLD_INACTIVE,
    EXCEEDS_HOLD,
    NO_TRANSACTION,
    NOT_REVERSIBLE,
    ALREADY_REVERSED,
    LIMIT_TOO_LOW,
    HOLDS_ACTIVE,
    NONZERO_BALANCE,
    BAD_RANGE,
} Code;

static const Slice messages[] = {
    [OK] = LITERAL("OK"),
    [OUT_OF_ORDER] = LITERAL("ERR OUT_OF_ORDER"),
    [ID_CONFLICT] = LITERAL("ERR ID_CONFLICT"),
    [ACCOUNT_EXISTS] = LITERAL("ERR ACCOUNT_EXISTS"),
    [NO_ACCOUNT] = LITERAL("ERR NO_ACCOUNT"),
    [ACCOUNT_CLOSED] = LITERAL("ERR ACCOUNT_CLOSED"),
    [INSUFFICIENT_FUNDS] = LITERAL("ERR INSUFFICIENT_FUNDS"),
    [SAME_ACCOUNT] = LITERAL("ERR SAME_ACCOUNT"),
    [NO_HOLD] = LITERAL("ERR NO_HOLD"),
    [HOLD_INACTIVE] = LITERAL("ERR HOLD_INACTIVE"),
    [EXCEEDS_HOLD] = LITERAL("ERR EXCEEDS_HOLD"),
    [NO_TRANSACTION] = LITERAL("ERR NO_TRANSACTION"),
    [NOT_REVERSIBLE] = LITERAL("ERR NOT_REVERSIBLE"),
    [ALREADY_REVERSED] = LITERAL("ERR ALREADY_REVERSED"),
    [LIMIT_TOO_LOW] = LITERAL("ERR LIMIT_TOO_LOW"),
    [HOLDS_ACTIVE] = LITERAL("ERR HOLDS_ACTIVE"),
    [NONZERO_BALANCE] = LITERAL("ERR NONZERO_BALANCE"),
    [BAD_RANGE] = LITERAL("ERR BAD_RANGE"),
};

typedef enum {
    OPEN,
    DEPOSIT,
    WITHDRAW,
    TRANSFER,
    HOLD,
    RELEASE,
    CAPTURE,
    REVERSE,
    LIMIT,
    CLOSE,
    BALANCE,
    BALANCEAT,
    STATEMENT,
} Command;

static const Slice command_names[] = {
    [OPEN] = LITERAL("OPEN"),
    [DEPOSIT] = LITERAL("DEPOSIT"),
    [WITHDRAW] = LITERAL("WITHDRAW"),
    [TRANSFER] = LITERAL("TRANSFER"),
    [HOLD] = LITERAL("HOLD"),
    [RELEASE] = LITERAL("RELEASE"),
    [CAPTURE] = LITERAL("CAPTURE"),
    [REVERSE] = LITERAL("REVERSE"),
    [LIMIT] = LITERAL("LIMIT"),
    [CLOSE] = LITERAL("CLOSE"),
    [BALANCE] = LITERAL("BALANCE"),
    [BALANCEAT] = LITERAL("BALANCEAT"),
    [STATEMENT] = LITERAL("STATEMENT"),
};

static Command command_named(Slice name)
{
    for (Command command = OPEN; command < STATEMENT; command++)
        if (equal(command_names[command], name))
            return command;
    /* The input is well formed, so a name that is none of the others is the last one. */
    return STATEMENT;
}

/* Entries arrive in timestamp order. Before entry i, at times[i], the ledger balance was
   balances[i] and the credits so far summed to credits[i]; balances[entries] and
   credits[entries] are the totals. */
typedef struct {
    int64_t *times;
    int64_t *balances;
    int64_t *credits;
    size_t entries;
    size_t capacity; /* of each array */
    int64_t ledger;
    int64_t held;
    int64_t limit;
    bool closed;
} Account;

static Account new_account(void)
{
    enum { INITIAL_CAPACITY = 4 };
    return (Account){
        .times = checked(malloc(INITIAL_CAPACITY * sizeof(int64_t))),
        .balances = checked(calloc(INITIAL_CAPACITY, sizeof(int64_t))),
        .credits = checked(calloc(INITIAL_CAPACITY, sizeof(int64_t))),
        .capacity = INITIAL_CAPACITY,
    };
}

static void post(Account *account, int64_t t, int64_t amount)
{
    if (account->entries + 1 == account->capacity) {
        account->capacity *= 2;
        size_t bytes = account->capacity * sizeof(int64_t);
        account->times = checked(realloc(account->times, bytes));
        account->balances = checked(realloc(account->balances, bytes));
        account->credits = checked(realloc(account->credits, bytes));
    }
    size_t i = account->entries++;
    account->ledger += amount;
    account->times[i] = t;
    account->balances[i + 1] = account->ledger;
    account->credits[i + 1] = account->credits[i] + (amount > 0 ? amount : 0);
}

/* The overdraft rule for taking amount out of the available balance. */
static bool can_spend(const Account *account, int64_t amount)
{
    return account->ledger - account->held - amount >= -account->limit;
}

/* The number of entries whose timestamp is at most time. */
static size_t entries_through(const Account *account, int64_t time)
{
    size_t low = 0;
    size_t high = account->entries;
    while (low < high) {
        size_t middle = low + (high - low) / 2;
        if (account->times[middle] <= time)
            low = middle + 1;
        else
            high = middle;
    }
    return low;
}

/* The number of entries whose timestamp is below time. */
static size_t entries_before(const Account *account, int64_t time)
{
    size_t low = 0;
    size_t high = account->entries;
    while (low < high) {
        size_t middle = low + (high - low) / 2;
        if (account->times[middle] < time)
            low = middle + 1;
        else
            high = middle;
    }
    return low;
}

/* Postings and holds name accounts by index, since the array that holds them moves as it
   grows. */
typedef struct {
    uint32_t account;
    int64_t amount;
} Posting;

typedef struct {
    uint32_t account;
    int64_t amount;
    bool active;
} Hold;

/* An accepted deposit, withdrawal, transfer or capture: the entries it posted. */
typedef struct {
    Posting postings[2];
    size_t count;
    bool reversed;
} Movement;

/* A recorded request id. An accepted HOLD keeps its hold, and an accepted deposit, withdrawal,
   transfer or capture its movement. */
typedef struct {
    Slice text; /* the line after its timestamp, which a retry repeats */
    Code result;
    Command command;
    union {
        Hold hold;
        Movement movement;
    };
} Request;

static bool is_movement(Command command)
{
    return command == DEPOSIT || command == WITHDRAW || command == TRANSFER || command == CAPTURE;
}

typedef struct {
    int64_t clock;
    Account *accounts;
    size_t account_count;
    size_t account_capacity;
    Request *requests;
    size_t request_count;
    size_t request_capacity;
    Table account_names; /* to indices into accounts */
    Table request_ids;   /* to indices into requests */
} Ledger;

static const Account *find_account(const Ledger *ledger, Slice name)
{
    uint32_t index = table_get(&ledger->account_names, name);
    return index == NONE ? nullptr : &ledger->accounts[index];
}

/* Finds the account named name, which must exist and be open. */
static Code find_open(const Ledger *ledger, Slice name, uint32_t *index)
{
    *index = table_get(&ledger->account_names, name);
    if (*index == NONE)
        return NO_ACCOUNT;
    return ledger->accounts[*index].closed ? ACCOUNT_CLOSED : OK;
}

/* Finds the hold placed by the accepted HOLD with request id id, which must be active. */
static Code find_active_hold(const Ledger *ledger, Slice id, Hold **hold)
{
    uint32_t index = table_get(&ledger->request_ids, id);
    Request *request = index == NONE ? nullptr : &ledger->requests[index];
    if (request == nullptr || request->result != OK || request->command != HOLD)
        return NO_HOLD;
    if (!request->hold.active)
        return HOLD_INACTIVE;
    *hold = &request->hold;
    return OK;
}

static void end_hold(Ledger *ledger, Hold *hold)
{
    hold->active = false;
    ledger->accounts[hold->account].held -= hold->amount;
}

/* Posts the entries of an accepted deposit, withdrawal, transfer or capture. */
static void move(Ledger *ledger, int64_t t, Request *request, size_t count, const Posting *postings)
{
    request->movement = (Movement){.count = count};
    for (size_t i = 0; i < count; i++) {
        post(&ledger->accounts[postings[i].account], t, postings[i].amount);
        request->movement.postings[i] = postings[i];
    }
}

static Code ledger_open(Ledger *ledger, Slice args)
{
    Slice name = take_word(&args);
    if (table_get(&ledger->account_names, name) != NONE)
        return ACCOUNT_EXISTS;
    ledger->accounts = reserve(ledger->accounts, &ledger->account_capacity, ledger->account_count,
                               sizeof *ledger->accounts);
    ledger->accounts[ledger->account_count] = new_account();
    table_add(&ledger->account_names, name, (uint32_t)ledger->account_count++);
    return OK;
}

static Code ledger_deposit(Ledger *ledger, int64_t t, Slice args, Request *request)
{
    uint32_t account;
    Code code = find_open(ledger, take_word(&args), &account);
    if (code != OK)
        return code;
    move(ledger, t, request, 1, &(Posting){account, take_int(&args)});
    return OK;
}

static Code ledger_withdraw(Ledger *ledger, int64_t t, Slice args, Request *request)
{
    uint32_t account;
    Code code = find_open(ledger, take_word(&args), &account);
    if (code != OK)
        return code;
    int64_t amount = take_int(&args);
    if (!can_spend(&ledger->accounts[account], amount))
        return INSUFFICIENT_FUNDS;
    move(ledger, t, request, 1, &(Posting){account, -amount});
    return OK;
}

static Code ledger_transfer(Ledger *ledger, int64_t t, Slice args, Request *request)
{
    Slice source = take_word(&args);
    Slice target = take_word(&args);
    if (equal(source, target))
        return SAME_ACCOUNT;
    uint32_t payer = table_get(&ledger->account_names, source);
    uint32_t payee = table_get(&ledger->account_names, target);
    if (payer == NONE || payee == NONE)
        return NO_ACCOUNT;
    if (ledger->accounts[payer].closed || ledger->accounts[payee].closed)
        return ACCOUNT_CLOSED;
    int64_t amount = take_int(&args);
    if (!can_spend(&ledger->accounts[payer], amount))
        return INSUFFICIENT_FUNDS;
    move(ledger, t, request, 2, (Posting[]){{payer, -amount}, {payee, amount}});
    return OK;
}

static Code ledger_hold(Ledger *ledger, Slice args, Request *request)
{
    uint32_t account;
    Code code = find_open(ledger, take_word(&args), &account);
    if (code != OK)
        return code;
    int64_t amount = take_int(&args);
    if (!can_spend(&ledger->accounts[account], amount))
        return INSUFFICIENT_FUNDS;
    ledger->accounts[account].held += amount;
    request->hold = (Hold){account, amount, true};
    return OK;
}

static Code ledger_release(Ledger *ledger, Slice args)
{
    Hold *hold;
    Code code = find_active_hold(ledger, take_word(&args), &hold);
    if (code != OK)
        return code;
    end_hold(ledger, hold);
    return OK;
}

static Code ledger_capture(Ledger *ledger, int64_t t, Slice args, Request *request)
{
    Hold *hold;
    Code code = find_active_hold(ledger, take_word(&args), &hold);
    if (code != OK)
        return code;
    int64_t amount = take_int(&args);
    if (amount > hold->amount)
        return EXCEEDS_HOLD;
    end_hold(ledger, hold);
    move(ledger, t, request, 1, &(Posting){hold->account, -amount});
    return OK;
}

static Code ledger_reverse(Ledger *ledger, int64_t t, Slice args)
{
    uint32_t index = table_get(&ledger->request_ids, take_word(&args));
    Request *target = index == NONE ? nullptr : &ledger->requests[index];
    if (target == nullptr || target->result != OK)
        return NO_TRANSACTION;
    if (!is_movement(target->command))
        return NOT_REVERSIBLE;
    Movement *movement = &target->movement;
    if (movement->reversed)
        return ALREADY_REVERSED;
    for (size_t i = 0; i < movement->count; i++)
        if (ledger->accounts[movement->postings[i].account].closed)
            return ACCOUNT_CLOSED;
    for (size_t i = 0; i < movement->count; i++) {
        const Posting *posting = &movement->postings[i];
        if (posting->amount > 0 && !can_spend(&ledger->accounts[posting->account], posting->amount))
            return INSUFFICIENT_FUNDS;
    }
    movement->reversed = true;
    for (size_t i = 0; i < movement->count; i++)
        post(&ledger->accounts[movement->postings[i].account], t, -movement->postings[i].amount);
    return OK;
}

static Code ledger_limit(Ledger *ledger, Slice args)
{
    uint32_t index;
    Code code = find_open(ledger, take_word(&args), &index);
    if (code != OK)
        return code;
    Account *account = &ledger->accounts[index];
    int64_t limit = take_int(&args);
    if (account->ledger - account->held < -limit)
        return LIMIT_TOO_LOW;
    account->limit = limit;
    return OK;
}

static Code ledger_close(Ledger *ledger, Slice args)
{
    uint32_t index;
    Code code = find_open(ledger, take_word(&args), &index);
    if (code != OK)
        return code;
    Account *account = &ledger->accounts[index];
    if (account->held != 0)
        return HOLDS_ACTIVE;
    if (account->ledger != 0)
        return NONZERO_BALANCE;
    account->closed = true;
    return OK;
}

static Code execute(Ledger *ledger, Command command, int64_t t, Slice args, Request *request)
{
    switch (command) {
    case OPEN:
        return ledger_open(ledger, args);
    case DEPOSIT:
        return ledger_deposit(ledger, t, args, request);
    case WITHDRAW:
        return ledger_withdraw(ledger, t, args, request);
    case TRANSFER:
        return ledger_transfer(ledger, t, args, request);
    case HOLD:
        return ledger_hold(ledger, args, request);
    case RELEASE:
        return ledger_release(ledger, args);
    case CAPTURE:
        return ledger_capture(ledger, t, args, request);
    case REVERSE:
        return ledger_reverse(ledger, t, args);
    case LIMIT:
        return ledger_limit(ledger, args);
    case CLOSE:
        return ledger_close(ledger, args);
    case BALANCE:
    case BALANCEAT:
    case STATEMENT:
        break;
    }
    unreachable();
}

/* Every line the ledger prints is shorter than this. */
enum { LONGEST_LINE = 128 };

/* Holds room for a whole line whenever a line starts. */
typedef struct {
    size_t length;
    char data[1 << 16];
} Output;

static void flush(Output *out)
{
    fwrite(out->data, 1, out->length, stdout);
    out->length = 0;
}

static void write_code(Output *out, Code code)
{
    memcpy(out->data + out->length, messages[code].data, messages[code].length);
    out->length += messages[code].length;
}

/* Writes count integers separated by spaces. */
static void write_ints(Output *out, size_t count, const int64_t *values)
{
    for (size_t i = 0; i < count; i++) {
        if (i > 0)
            out->data[out->length++] = ' ';
        int64_t value = values[i];
        if (value < 0) {
            out->data[out->length++] = '-';
            value = -value;
        }
        char digits[20];
        size_t length = 0;
        do {
            digits[length++] = (char)('0' + value % 10);
            value /= 10;
        } while (value != 0);
        while (length > 0)
            out->data[out->length++] = digits[--length];
    }
}

static void end_line(Output *out)
{
    out->data[out->length++] = '\n';
    if (sizeof out->data - out->length < LONGEST_LINE)
        flush(out);
}

static void ledger_balance(const Ledger *ledger, Slice args, Output *out)
{
    const Account *account = find_account(ledger, take_word(&args));
    if (account == nullptr) {
        write_code(out, NO_ACCOUNT);
        return;
    }
    write_ints(out, 2, (int64_t[]){account->ledger, account->ledger - account->held});
}

static void ledger_balance_at(const Ledger *ledger, Slice args, Output *out)
{
    const Account *account = find_account(ledger, take_word(&args));
    if (account == nullptr) {
        write_code(out, NO_ACCOUNT);
        return;
    }
    write_ints(out, 1, &account->balances[entries_through(account, take_int(&args))]);
}

static void ledger_statement(const Ledger *ledger, Slice args, Output *out)
{
    Slice name = take_word(&args);
    int64_t first = take_int(&args);
    int64_t last = take_int(&args);
    if (first > last) {
        write_code(out, BAD_RANGE);
        return;
    }
    const Account *account = find_account(ledger, name);
    if (account == nullptr) {
        write_code(out, NO_ACCOUNT);
        return;
    }
    size_t low = entries_before(account, first);
    size_t high = entries_through(account, last);
    int64_t opening = account->balances[low];
    int64_t closing = account->balances[high];
    int64_t credits = account->credits[high] - account->credits[low];
    int64_t debits = credits - (closing - opening);
    write_ints(out, 5, (int64_t[]){opening, credits, debits, closing, (int64_t)(high - low)});
}

/* Processes one input line and writes its result line, without the line feed. */
static void process(Ledger *ledger, Slice line, Output *out)
{
    int64_t t = take_int(&line);
    if (t < ledger->clock) {
        write_code(out, OUT_OF_ORDER);
        return;
    }
    ledger->clock = t;
    Slice text = line;
    Command command = command_named(take_word(&line));
    switch (command) {
    case BALANCE:
        ledger_balance(ledger, line, out);
        return;
    case BALANCEAT:
        ledger_balance_at(ledger, line, out);
        return;
    case STATEMENT:
        ledger_statement(ledger, line, out);
        return;
    default:
        break;
    }

    Slice id = take_word(&line);
    uint32_t recorded = table_get(&ledger->request_ids, id);
    if (recorded != NONE) {
        const Request *request = &ledger->requests[recorded];
        write_code(out, equal(request->text, text) ? request->result : ID_CONFLICT);
        return;
    }
    Request request = {.text = text, .command = command};
    request.result = execute(ledger, command, t, line, &request);
    ledger->requests = reserve(ledger->requests, &ledger->request_capacity, ledger->request_count,
                               sizeof *ledger->requests);
    ledger->requests[ledger->request_count] = request;
    table_add(&ledger->request_ids, id, (uint32_t)ledger->request_count++);
    write_code(out, request.result);
}

int main(void)
{
    size_t length;
    char *input = read_all(stdin, &length);
    Slice rest = {input, length};
    Slice header = take(&rest, '\n');
    size_t n = (size_t)take_int(&header);

    Ledger ledger = {.account_names = new_table(), .request_ids = new_table()};
    Output out = {};
    for (size_t i = 0; i < n; i++) {
        process(&ledger, take(&rest, '\n'), &out);
        end_line(&out);
    }
    flush(&out);

    for (size_t i = 0; i < ledger.account_count; i++) {
        free(ledger.accounts[i].times);
        free(ledger.accounts[i].balances);
        free(ledger.accounts[i].credits);
    }
    free(ledger.accounts);
    free(ledger.requests);
    free(ledger.account_names.slots);
    free(ledger.request_ids.slots);
    free(input);
    if (fflush(stdout) != 0 || ferror(stdout)) {
        perror("writing standard output");
        return EXIT_FAILURE;
    }
    return EXIT_SUCCESS;
}
