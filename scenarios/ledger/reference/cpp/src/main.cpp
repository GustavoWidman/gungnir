#include <algorithm>
#include <array>
#include <charconv>
#include <cstdint>
#include <expected>
#include <format>
#include <initializer_list>
#include <iostream>
#include <iterator>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <variant>
#include <vector>

namespace {

// Every result line that is not a query's integers.
namespace result {
constexpr std::string_view ok = "OK";
constexpr std::string_view out_of_order = "ERR OUT_OF_ORDER";
constexpr std::string_view id_conflict = "ERR ID_CONFLICT";
constexpr std::string_view account_exists = "ERR ACCOUNT_EXISTS";
constexpr std::string_view no_account = "ERR NO_ACCOUNT";
constexpr std::string_view account_closed = "ERR ACCOUNT_CLOSED";
constexpr std::string_view insufficient_funds = "ERR INSUFFICIENT_FUNDS";
constexpr std::string_view same_account = "ERR SAME_ACCOUNT";
constexpr std::string_view no_hold = "ERR NO_HOLD";
constexpr std::string_view hold_inactive = "ERR HOLD_INACTIVE";
constexpr std::string_view exceeds_hold = "ERR EXCEEDS_HOLD";
constexpr std::string_view no_transaction = "ERR NO_TRANSACTION";
constexpr std::string_view not_reversible = "ERR NOT_REVERSIBLE";
constexpr std::string_view already_reversed = "ERR ALREADY_REVERSED";
constexpr std::string_view limit_too_low = "ERR LIMIT_TOO_LOW";
constexpr std::string_view holds_active = "ERR HOLDS_ACTIVE";
constexpr std::string_view nonzero_balance = "ERR NONZERO_BALANCE";
constexpr std::string_view bad_range = "ERR BAD_RANGE";
}  // namespace result

enum class Command {
    open,
    deposit,
    withdraw,
    transfer,
    hold,
    release,
    capture,
    reverse,
    limit,
    close,
    balance,
    balance_at,
    statement,
};

constexpr std::array<std::pair<std::string_view, Command>, 13> commands{{
    {"OPEN", Command::open},
    {"DEPOSIT", Command::deposit},
    {"WITHDRAW", Command::withdraw},
    {"TRANSFER", Command::transfer},
    {"HOLD", Command::hold},
    {"RELEASE", Command::release},
    {"CAPTURE", Command::capture},
    {"REVERSE", Command::reverse},
    {"LIMIT", Command::limit},
    {"CLOSE", Command::close},
    {"BALANCE", Command::balance},
    {"BALANCEAT", Command::balance_at},
    {"STATEMENT", Command::statement},
}};

Command command_named(std::string_view name)
{
    return std::ranges::find(commands, name, &std::pair<std::string_view, Command>::first)->second;
}

std::string read_all(std::istream& in)
{
    std::string data;
    std::size_t length = 0;
    while (in) {
        data.resize(std::max(2 * data.size(), std::size_t{1} << 16));
        in.read(data.data() + length, static_cast<std::streamsize>(data.size() - length));
        length += static_cast<std::size_t>(in.gcount());
    }
    data.resize(length);
    return data;
}

// Removes the text up to the next separator, and the separator, from the front of text.
std::string_view take(std::string_view& text, char separator)
{
    const std::size_t end = std::min(text.find(separator), text.size());
    const std::string_view taken = text.substr(0, end);
    text.remove_prefix(std::min(end + 1, text.size()));
    return taken;
}

std::string_view take_word(std::string_view& text)
{
    return take(text, ' ');
}

std::int64_t take_int(std::string_view& text)
{
    const std::string_view word = take_word(text);
    std::int64_t value = 0;
    std::from_chars(word.data(), word.data() + word.size(), value);
    return value;
}

struct Account {
    bool closed = false;
    std::int64_t ledger = 0;
    std::int64_t held = 0;
    std::int64_t limit = 0;
    // Entries arrive in timestamp order. Before entry i, at times[i], the ledger balance was
    // balances[i] and the credits so far summed to credits[i]; the last element is the total.
    std::vector<std::int64_t> times;
    std::vector<std::int64_t> balances{0};
    std::vector<std::int64_t> credits{0};

    void post(std::int64_t t, std::int64_t amount)
    {
        ledger += amount;
        times.push_back(t);
        balances.push_back(ledger);
        credits.push_back(credits.back() + std::max(amount, std::int64_t{0}));
    }

    // The overdraft rule for taking amount out of the available balance.
    bool can_spend(std::int64_t amount) const { return ledger - held - amount >= -limit; }

    std::size_t entries_through(std::int64_t time) const
    {
        return static_cast<std::size_t>(std::ranges::upper_bound(times, time) - times.begin());
    }

    std::size_t entries_before(std::int64_t time) const
    {
        return static_cast<std::size_t>(std::ranges::lower_bound(times, time) - times.begin());
    }
};

struct Posting {
    Account* account;
    std::int64_t amount;
};

struct Hold {
    Account* account;
    std::int64_t amount;
    bool active = true;

    void end()
    {
        active = false;
        account->held -= amount;
    }
};

// An accepted deposit, withdrawal, transfer or capture: the entries it posted.
struct Movement {
    std::array<Posting, 2> postings{};
    std::size_t count = 0;
    bool reversed = false;

    std::span<const Posting> entries() const { return std::span(postings).first(count); }
};

// What an accepted HOLD or movement leaves for later commands to find.
using Effect = std::variant<std::monostate, Hold, Movement>;

// A recorded request id.
struct Request {
    std::string_view text;  // the line after its timestamp, which a retry repeats
    std::string_view result;
    Effect effect;
};

// Posts the entries of a deposit, withdrawal, transfer or capture.
Movement post_movement(std::int64_t t, std::initializer_list<Posting> postings)
{
    Movement movement;
    for (const Posting& posting : postings) {
        posting.account->post(t, posting.amount);
        movement.postings[movement.count++] = posting;
    }
    return movement;
}

// The names in the maps' keys view the input, which outlives the ledger, and Account and Hold
// pointers stay valid because unordered_map never moves its elements.
class Ledger {
public:
    // Processes one input line and appends its result line, without the line feed, to out.
    void process(std::string_view line, std::string& out)
    {
        const std::int64_t t = take_int(line);
        if (t < clock_) {
            out += result::out_of_order;
            return;
        }
        clock_ = t;
        const std::string_view text = line;
        const Command command = command_named(take_word(line));
        switch (command) {
        case Command::balance:
            return balance(line, out);
        case Command::balance_at:
            return balance_at(line, out);
        case Command::statement:
            return statement(line, out);
        default:
            break;
        }

        const std::string_view id = take_word(line);
        if (const auto recorded = requests_.find(id); recorded != requests_.end()) {
            const Request& request = recorded->second;
            out += request.text == text ? request.result : result::id_conflict;
            return;
        }
        Effect effect;
        const std::string_view outcome = execute(command, t, line, effect);
        out += outcome;
        requests_.emplace(id, Request{text, outcome, effect});
    }

private:
    std::string_view execute(Command command, std::int64_t t, std::string_view args, Effect& effect)
    {
        switch (command) {
        case Command::open:
            return open(args);
        case Command::deposit:
            return deposit(t, args, effect);
        case Command::withdraw:
            return withdraw(t, args, effect);
        case Command::transfer:
            return transfer(t, args, effect);
        case Command::hold:
            return hold(args, effect);
        case Command::release:
            return release(args);
        case Command::capture:
            return capture(t, args, effect);
        case Command::reverse:
            return reverse(t, args);
        case Command::limit:
            return limit(args);
        case Command::close:
            return close(args);
        case Command::balance:
        case Command::balance_at:
        case Command::statement:
            break;
        }
        std::unreachable();
    }

    const Account* find_account(std::string_view name) const
    {
        const auto found = accounts_.find(name);
        return found == accounts_.end() ? nullptr : &found->second;
    }

    // Finds the account named name, which must exist and be open.
    std::expected<Account*, std::string_view> open_account(std::string_view name)
    {
        const auto found = accounts_.find(name);
        if (found == accounts_.end()) {
            return std::unexpected(result::no_account);
        }
        if (found->second.closed) {
            return std::unexpected(result::account_closed);
        }
        return &found->second;
    }

    // Finds the hold placed by the accepted HOLD with request id id, which must be active.
    std::expected<Hold*, std::string_view> active_hold(std::string_view id)
    {
        const auto found = requests_.find(id);
        Hold* hold = found == requests_.end() ? nullptr : std::get_if<Hold>(&found->second.effect);
        if (hold == nullptr) {
            return std::unexpected(result::no_hold);
        }
        if (!hold->active) {
            return std::unexpected(result::hold_inactive);
        }
        return hold;
    }

    std::string_view open(std::string_view args)
    {
        return accounts_.try_emplace(take_word(args)).second ? result::ok : result::account_exists;
    }

    std::string_view deposit(std::int64_t t, std::string_view args, Effect& effect)
    {
        const auto account = open_account(take_word(args));
        if (!account) {
            return account.error();
        }
        effect = post_movement(t, {{*account, take_int(args)}});
        return result::ok;
    }

    std::string_view withdraw(std::int64_t t, std::string_view args, Effect& effect)
    {
        const auto account = open_account(take_word(args));
        if (!account) {
            return account.error();
        }
        const std::int64_t amount = take_int(args);
        if (!(*account)->can_spend(amount)) {
            return result::insufficient_funds;
        }
        effect = post_movement(t, {{*account, -amount}});
        return result::ok;
    }

    std::string_view transfer(std::int64_t t, std::string_view args, Effect& effect)
    {
        const std::string_view source = take_word(args);
        const std::string_view target = take_word(args);
        if (source == target) {
            return result::same_account;
        }
        const auto payer = accounts_.find(source);
        const auto payee = accounts_.find(target);
        if (payer == accounts_.end() || payee == accounts_.end()) {
            return result::no_account;
        }
        if (payer->second.closed || payee->second.closed) {
            return result::account_closed;
        }
        const std::int64_t amount = take_int(args);
        if (!payer->second.can_spend(amount)) {
            return result::insufficient_funds;
        }
        effect = post_movement(t, {{&payer->second, -amount}, {&payee->second, amount}});
        return result::ok;
    }

    std::string_view hold(std::string_view args, Effect& effect)
    {
        const auto account = open_account(take_word(args));
        if (!account) {
            return account.error();
        }
        const std::int64_t amount = take_int(args);
        if (!(*account)->can_spend(amount)) {
            return result::insufficient_funds;
        }
        (*account)->held += amount;
        effect = Hold{*account, amount};
        return result::ok;
    }

    std::string_view release(std::string_view args)
    {
        const auto hold = active_hold(take_word(args));
        if (!hold) {
            return hold.error();
        }
        (*hold)->end();
        return result::ok;
    }

    std::string_view capture(std::int64_t t, std::string_view args, Effect& effect)
    {
        const auto found = active_hold(take_word(args));
        if (!found) {
            return found.error();
        }
        Hold& hold = **found;
        const std::int64_t amount = take_int(args);
        if (amount > hold.amount) {
            return result::exceeds_hold;
        }
        hold.end();
        effect = post_movement(t, {{hold.account, -amount}});
        return result::ok;
    }

    std::string_view reverse(std::int64_t t, std::string_view args)
    {
        const auto found = requests_.find(take_word(args));
        if (found == requests_.end() || found->second.result != result::ok) {
            return result::no_transaction;
        }
        auto* movement = std::get_if<Movement>(&found->second.effect);
        if (movement == nullptr) {
            return result::not_reversible;
        }
        if (movement->reversed) {
            return result::already_reversed;
        }
        const auto postings = movement->entries();
        if (std::ranges::any_of(postings, [](const Posting& p) { return p.account->closed; })) {
            return result::account_closed;
        }
        if (!std::ranges::all_of(postings, [](const Posting& p) {
                return p.amount <= 0 || p.account->can_spend(p.amount);
            })) {
            return result::insufficient_funds;
        }
        movement->reversed = true;
        for (const auto& [account, amount] : postings) {
            account->post(t, -amount);
        }
        return result::ok;
    }

    std::string_view limit(std::string_view args)
    {
        const auto account = open_account(take_word(args));
        if (!account) {
            return account.error();
        }
        const std::int64_t limit = take_int(args);
        if ((*account)->ledger - (*account)->held < -limit) {
            return result::limit_too_low;
        }
        (*account)->limit = limit;
        return result::ok;
    }

    std::string_view close(std::string_view args)
    {
        const auto account = open_account(take_word(args));
        if (!account) {
            return account.error();
        }
        if ((*account)->held != 0) {
            return result::holds_active;
        }
        if ((*account)->ledger != 0) {
            return result::nonzero_balance;
        }
        (*account)->closed = true;
        return result::ok;
    }

    void balance(std::string_view args, std::string& out) const
    {
        const Account* account = find_account(take_word(args));
        if (account == nullptr) {
            out += result::no_account;
            return;
        }
        std::format_to(std::back_inserter(out), "{} {}", account->ledger,
                       account->ledger - account->held);
    }

    void balance_at(std::string_view args, std::string& out) const
    {
        const Account* account = find_account(take_word(args));
        if (account == nullptr) {
            out += result::no_account;
            return;
        }
        std::format_to(std::back_inserter(out), "{}",
                       account->balances[account->entries_through(take_int(args))]);
    }

    void statement(std::string_view args, std::string& out) const
    {
        const std::string_view name = take_word(args);
        const std::int64_t first = take_int(args);
        const std::int64_t last = take_int(args);
        if (first > last) {
            out += result::bad_range;
            return;
        }
        const Account* account = find_account(name);
        if (account == nullptr) {
            out += result::no_account;
            return;
        }
        const std::size_t low = account->entries_before(first);
        const std::size_t high = account->entries_through(last);
        const std::int64_t opening = account->balances[low];
        const std::int64_t closing = account->balances[high];
        const std::int64_t credits = account->credits[high] - account->credits[low];
        const std::int64_t debits = credits - (closing - opening);
        std::format_to(std::back_inserter(out), "{} {} {} {} {}", opening, credits, debits, closing,
                       high - low);
    }

    std::int64_t clock_ = 0;
    std::unordered_map<std::string_view, Account> accounts_;
    std::unordered_map<std::string_view, Request> requests_;
};

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    std::string_view rest = input;
    std::string_view header = take(rest, '\n');
    const auto n = static_cast<std::size_t>(take_int(header));

    Ledger ledger;
    std::string output;
    for (std::size_t i = 0; i < n; ++i) {
        ledger.process(take(rest, '\n'), output);
        output += '\n';
    }
    std::cout << output;
}
