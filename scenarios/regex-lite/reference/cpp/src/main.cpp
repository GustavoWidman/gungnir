#include <algorithm>
#include <array>
#include <bit>
#include <bitset>
#include <cstddef>
#include <cstdint>
#include <format>
#include <iostream>
#include <iterator>
#include <span>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace {

constexpr std::string_view escapable = "\\.*+?|()[]^$-";
// A text line has at most 100000 characters, so one line adds at most about 300000 DFA states.
constexpr std::size_t state_limit = 10000;

enum class Kind { character, epsilon, begin, end, match };

struct Invalid {};

struct Node {
    Kind kind;
    std::bitset<128> chars;
    std::vector<int> out;
};

struct Fragment {
    int entry;
    int exit;
};

// Builds a Thompson NFA while parsing. Every fragment is an (entry, exit) pair whose exit is an
// epsilon node that later gets the edges to whatever follows. Throws Invalid on a bad pattern.
class Parser {
public:
    explicit Parser(std::string_view pattern) : pattern_(pattern) {}

    // Parses the whole pattern and returns the start node. The accepting node comes last.
    int parse()
    {
        const Fragment f = alternation();
        if (pos_ != pattern_.size()) {
            throw Invalid{};
        }
        const int accept = node(Kind::match);
        nodes_[f.exit].out.push_back(accept);
        return f.entry;
    }

    const std::vector<Node>& nodes() const { return nodes_; }

private:
    int node(Kind kind, std::bitset<128> chars = {})
    {
        nodes_.push_back({kind, chars, {}});
        return static_cast<int>(nodes_.size()) - 1;
    }

    int peek() const { return pos_ < pattern_.size() ? static_cast<unsigned char>(pattern_[pos_]) : -1; }

    int take()
    {
        const int c = peek();
        if (c < 0) {
            throw Invalid{};
        }
        ++pos_;
        return c;
    }

    int escaped()
    {
        const int c = take();
        if (!escapable.contains(static_cast<char>(c))) {
            throw Invalid{};
        }
        return c;
    }

    Fragment alternation()
    {
        const int entry = node(Kind::epsilon);
        const int exit = node(Kind::epsilon);
        while (true) {
            const Fragment f = sequence();
            nodes_[entry].out.push_back(f.entry);
            nodes_[f.exit].out.push_back(exit);
            if (peek() != '|') {
                return {entry, exit};
            }
            ++pos_;
        }
    }

    Fragment sequence()
    {
        const int entry = node(Kind::epsilon);
        int last = entry;
        for (int c = peek(); c != -1 && c != '|' && c != ')'; c = peek()) {
            const Fragment f = item();
            nodes_[last].out.push_back(f.entry);
            last = f.exit;
        }
        return {entry, last};
    }

    Fragment item()
    {
        const Fragment f = atom();
        switch (peek()) {
        case '*': {
            ++pos_;
            const int loop = node(Kind::epsilon);
            nodes_[loop].out.push_back(f.entry);
            nodes_[f.exit].out.push_back(loop);
            return {loop, loop};
        }
        case '+':
            ++pos_;
            nodes_[f.exit].out.push_back(f.entry);
            return f;
        case '?': {
            ++pos_;
            const int skip = node(Kind::epsilon);
            nodes_[skip].out.push_back(f.entry);
            nodes_[skip].out.push_back(f.exit);
            return {skip, f.exit};
        }
        default:
            return f;
        }
    }

    Fragment atom()
    {
        std::bitset<128> chars;
        switch (const int c = take()) {
        case '(': {
            const Fragment f = alternation();
            if (take() != ')') {
                throw Invalid{};
            }
            return f;
        }
        case '*':
        case '+':
        case '?':
            throw Invalid{};
        case '^':
            return wrap(Kind::begin, chars);
        case '$':
            return wrap(Kind::end, chars);
        case '.':
            chars.set();
            break;
        case '[':
            chars = bracket();
            break;
        case '\\':
            chars.set(static_cast<std::size_t>(escaped()));
            break;
        default:
            chars.set(static_cast<std::size_t>(c));
        }
        return wrap(Kind::character, chars);
    }

    Fragment wrap(Kind kind, std::bitset<128> chars)
    {
        const int n = node(kind, chars);
        const int exit = node(Kind::epsilon);
        nodes_[n].out.push_back(exit);
        return {n, exit};
    }

    std::bitset<128> bracket()
    {
        const bool negated = peek() == '^';
        if (negated) {
            ++pos_;
        }
        struct Token {
            int c;
            bool dash;  // an unescaped '-'
        };
        std::vector<Token> body;
        for (int c = take(); c != ']'; c = take()) {
            body.push_back(c == '\\' ? Token{escaped(), false} : Token{c, c == '-'});
        }
        if (body.empty()) {
            throw Invalid{};
        }
        std::bitset<128> chars;
        std::size_t start = 0;
        std::size_t stop = body.size();
        if (body.front().dash) {
            chars.set('-');
            start = 1;
        }
        if (stop > start && body[stop - 1].dash) {
            chars.set('-');
            --stop;
        }
        for (std::size_t i = start; i < stop;) {
            if (body[i].dash) {
                throw Invalid{};
            }
            if (i + 1 < stop && body[i + 1].dash) {
                if (i + 2 >= stop || body[i + 2].dash || body[i].c > body[i + 2].c) {
                    throw Invalid{};
                }
                for (int c = body[i].c; c <= body[i + 2].c; ++c) {
                    chars.set(static_cast<std::size_t>(c));
                }
                i += 3;
            } else {
                chars.set(static_cast<std::size_t>(body[i].c));
                ++i;
            }
        }
        return negated ? ~chars : chars;
    }

    std::string_view pattern_;
    std::size_t pos_ = 0;
    std::vector<Node> nodes_;
};

using Bits = std::vector<std::uint64_t>;

std::uint64_t mix(std::uint64_t hash, std::uint64_t word)
{
    return (std::rotl(hash, 5) ^ word) * 0x517cc1b727220a95;
}

struct BitsHash {
    std::size_t operator()(const Bits& bits) const noexcept
    {
        std::uint64_t hash = 0;
        for (const std::uint64_t word : bits) {
            hash = mix(hash, word);
        }
        return hash;
    }
};

struct PairHash {
    std::size_t operator()(std::uint64_t key) const noexcept { return mix(0, key); }
};

void set(std::span<std::uint64_t> bits, std::size_t k)
{
    bits[k / 64] |= std::uint64_t{1} << (k % 64);
}

bool intersects(std::span<const std::uint64_t> a, std::span<const std::uint64_t> b)
{
    for (std::size_t w = 0; w < a.size(); ++w) {
        if (a[w] & b[w]) {
            return true;
        }
    }
    return false;
}

constexpr std::array<std::int32_t, 128> unknown_row = [] {
    std::array<std::int32_t, 128> row{};
    row.fill(-1);
    return row;
}();

// One interned DFA state: a bitset over the kernel and what is cached about it.
struct State {
    Bits bits;
    std::array<std::int32_t, 128> back = unknown_row;  // reverse transitions by character
    std::int32_t next = -1;              // the state after every member consumes a character
    std::int32_t entry = -1;  // this state intersected with the start closure at an inner position
    bool accepts;
    bool consumes;
    bool starts;
};

// Counts leftmost-longest matches in linear time per line.
//
// The DFA states are bitsets over the kernel: bit 0 is the accepting node and every other bit a
// node that consumes a character. A reverse pass records at each position which kernel states
// can still reach acceptance from there. Forward scans start only where a match starts and keep
// only states that can still accept, so each scan stops right after its longest match. The
// forward DFA reads the reverse pass's state at each position as its input symbol.
class Matcher {
public:
    Matcher(const std::vector<Node>& nodes, int start_node)
    {
        std::vector<int> kernel(nodes.size(), -1);
        kernel.back() = 0;
        std::size_t size = 1;
        for (std::size_t v = 0; v < nodes.size(); ++v) {
            if (nodes[v].kind == Kind::character) {
                kernel[v] = static_cast<int>(size++);
            }
        }
        words_ = (size + 63) / 64;

        std::vector<std::uint32_t> mark(nodes.size(), 0);
        std::uint32_t stamp = 0;
        std::vector<int> stack;
        auto closure = [&](std::span<const int> roots, bool begin, bool end) {
            ++stamp;
            Bits bits(words_, 0);
            auto visit = [&](int v) {
                if (mark[v] != stamp) {
                    mark[v] = stamp;
                    stack.push_back(v);
                }
            };
            std::ranges::for_each(roots, visit);
            while (!stack.empty()) {
                const int v = stack.back();
                stack.pop_back();
                const Kind kind = nodes[v].kind;
                if (kind == Kind::character || kind == Kind::match) {
                    set(bits, static_cast<std::size_t>(kernel[v]));
                } else if ((kind != Kind::begin || begin) && (kind != Kind::end || end)) {
                    std::ranges::for_each(nodes[v].out, visit);
                }
            }
            return bits;
        };

        follow_.assign(size, Bits(words_, 0));
        std::vector<Bits> follow_end(size, Bits(words_, 0));
        for (std::size_t v = 0; v < nodes.size(); ++v) {
            if (kernel[v] <= 0) {
                continue;
            }
            const auto k = static_cast<std::size_t>(kernel[v]);
            follow_[k] = closure(nodes[v].out, false, false);
            follow_end[k] = closure(nodes[v].out, false, true);
            for (std::size_t c = 32; c < 127; ++c) {
                if (nodes[v].chars.test(c)) {
                    consumers_[c].push_back(k);
                }
            }
        }
        const std::array<int, 1> root{start_node};
        for (int b = 0; b < 2; ++b) {
            for (int e = 0; e < 2; ++e) {
                start_[b][e] = closure(root, b, e);
            }
        }
        for (std::size_t c = 0; c < 128; ++c) {
            last_[c].assign(words_, 0);
            set(last_[c], 0);
            for (const std::size_t k : consumers_[c]) {
                if (follow_end[k][0] & 1) {
                    set(last_[c], k);
                }
            }
        }
        scratch_.assign(words_, 0);
        reset();
    }

    std::uint32_t count(std::string_view line)
    {
        if (states_.size() + forward_.size() > state_limit) {
            reset();
        }
        const std::size_t n = line.size();
        alive_.assign(n + 1, accept_only_);
        if (n > 0) {
            std::int32_t state = alive_[n - 1] = last_ids_[static_cast<unsigned char>(line[n - 1])];
            for (std::size_t i = n - 1; i-- > 0;) {
                const auto c = static_cast<unsigned char>(line[i]);
                std::int32_t next = states_[state].back[c];
                if (next < 0) {
                    next = step_back(state, c);
                }
                alive_[i] = state = next;
            }
        }

        std::uint32_t matches = 0;
        for (std::size_t p = 0; p <= n;) {
            std::size_t s = p;
            while (0 < s && s < n && !states_[alive_[s]].starts) {
                ++s;
            }
            std::int32_t current = entry(s, n, alive_[s]);
            if (!states_[current].accepts && !states_[current].consumes) {
                if (s >= n) {
                    break;
                }
                p = s + 1;
                continue;
            }
            std::size_t end = states_[current].accepts ? s : SIZE_MAX;
            std::size_t i = s;
            while (states_[current].consumes) {
                ++i;
                current = i == n ? accept_only_ : step(current, alive_[i]);
                if (states_[current].accepts) {
                    end = i;
                }
            }
            ++matches;
            p = end > s ? end : s + 1;
        }
        return matches;
    }

private:
    void reset()
    {
        ids_.clear();
        states_.clear();
        forward_.clear();
        std::ranges::fill(scratch_, 0);
        set(scratch_, 0);
        accept_only_ = intern_scratch();
        for (std::size_t c = 0; c < 128; ++c) {
            scratch_ = last_[c];
            last_ids_[c] = intern_scratch();
        }
    }

    std::int32_t intern_scratch()
    {
        const auto [it, inserted] = ids_.try_emplace(scratch_, static_cast<std::int32_t>(states_.size()));
        if (inserted) {
            const Bits& bits = it->first;
            const auto nonzero = [](std::uint64_t word) { return word != 0; };
            states_.push_back({
                .bits = bits,
                .accepts = (bits[0] & 1) != 0,
                .consumes = bits[0] > 1 || std::any_of(bits.begin() + 1, bits.end(), nonzero),
                .starts = intersects(start_[0][0], bits),
            });
        }
        return it->second;
    }

    // The state holding the members of both `a` and state `b`.
    std::int32_t meet(std::span<const std::uint64_t> a, std::int32_t b)
    {
        const Bits& bits = states_[b].bits;
        for (std::size_t w = 0; w < words_; ++w) {
            scratch_[w] = a[w] & bits[w];
        }
        return intern_scratch();
    }

    // The kernel states that can accept from a position holding c, given those that can accept
    // from the next position.
    std::int32_t step_back(std::int32_t state, unsigned char c)
    {
        std::ranges::fill(scratch_, 0);
        set(scratch_, 0);
        for (const std::size_t k : consumers_[c]) {
            if (intersects(follow_[k], states_[state].bits)) {
                set(scratch_, k);
            }
        }
        const std::int32_t next = intern_scratch();
        states_[state].back[c] = next;
        return next;
    }

    // The state after every member of `state` consumes a character. The character is implied:
    // every state in a pruned set accepts the character at its position.
    std::int32_t consume(std::int32_t state)
    {
        if (states_[state].next >= 0) {
            return states_[state].next;
        }
        std::ranges::fill(scratch_, 0);
        for (std::size_t w = 0; w < words_; ++w) {
            std::uint64_t rest = states_[state].bits[w] & (w == 0 ? ~std::uint64_t{1} : ~std::uint64_t{0});
            for (; rest != 0; rest &= rest - 1) {
                const Bits& follow = follow_[w * 64 + static_cast<std::size_t>(std::countr_zero(rest))];
                for (std::size_t v = 0; v < words_; ++v) {
                    scratch_[v] |= follow[v];
                }
            }
        }
        const std::int32_t next = intern_scratch();
        states_[state].next = next;
        return next;
    }

    // The pruned forward state after `state`, at a position whose reverse state is `alive`.
    std::int32_t step(std::int32_t state, std::int32_t alive)
    {
        const std::uint64_t key = static_cast<std::uint64_t>(state) << 32 | static_cast<std::uint32_t>(alive);
        if (const auto it = forward_.find(key); it != forward_.end()) {
            return it->second;
        }
        const std::int32_t consumed = consume(state);
        const Bits bits = states_[consumed].bits;
        const std::int32_t next = meet(bits, alive);
        forward_.emplace(key, next);
        return next;
    }

    // The forward state at a match candidate s of a line of n characters.
    std::int32_t entry(std::size_t s, std::size_t n, std::int32_t alive)
    {
        if (s == 0 || s == n) {
            return meet(start_[s == 0][s == n], alive);
        }
        if (states_[alive].entry < 0) {
            const std::int32_t met = meet(start_[0][0], alive);
            states_[alive].entry = met;
        }
        return states_[alive].entry;
    }

    std::size_t words_ = 0;
    std::vector<Bits> follow_;
    std::array<std::vector<std::size_t>, 128> consumers_;
    Bits start_[2][2];  // the start closure, indexed by [at line start][at line end]
    std::array<Bits, 128> last_;  // per character: what can accept from the last position
    Bits scratch_;
    std::unordered_map<Bits, std::int32_t, BitsHash> ids_;
    std::vector<State> states_;
    std::unordered_map<std::uint64_t, std::int32_t, PairHash> forward_;
    std::array<std::int32_t, 128> last_ids_{};
    std::int32_t accept_only_ = 0;
    std::vector<std::int32_t> alive_;
};

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

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    const std::string_view all = input;
    const std::size_t newline = std::min(all.find('\n'), all.size());
    Parser parser(all.substr(0, newline));
    std::string output;
    try {
        const int start = parser.parse();
        Matcher matcher(parser.nodes(), start);
        std::string_view text = all.substr(std::min(newline + 1, all.size()));
        for (std::size_t stop = text.find('\n'); stop != std::string_view::npos; stop = text.find('\n')) {
            std::format_to(std::back_inserter(output), "{}\n", matcher.count(text.substr(0, stop)));
            text.remove_prefix(stop + 1);
        }
    } catch (const Invalid&) {
        output = "invalid\n";
    }
    std::cout << output;
}
