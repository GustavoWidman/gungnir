#include <algorithm>
#include <array>
#include <charconv>
#include <cstdint>
#include <cstdio>
#include <iostream>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <vector>

namespace {

enum class Kind : std::uint8_t { Scalar, Array, Object };

// A value of the document. Nodes are stored in document order, so a node's index orders it,
// and its descendants are exactly the nodes from its index + 1 up to end.
struct Node {
    Kind kind = Kind::Scalar;
    // A scalar's printed text in Document::text, or a container's children in Document::kids.
    std::uint32_t start = 0, length = 0;
    // The member name printed as a string, in Document::text, when the parent is an object.
    std::uint32_t name_start = 0, name_length = 0;
    std::uint32_t end = 0;
};

enum class SelectorKind : std::uint8_t { Name, Wild, Index, Slice };

// A name is the member name printed as a string. Printing is one-to-one on code unit
// sequences, so two names are equal exactly when their printed forms are.
struct Segment {
    bool descendant = false;
    SelectorKind kind = SelectorKind::Name;
    std::string name;
    std::int64_t index = 0;
    std::optional<std::int64_t> start, end;
    std::int64_t step = 1;
};

constexpr std::string_view max_i64 = "9223372036854775807";
constexpr std::string_view min_i64 = "9223372036854775808";

void put_unit(std::string& out, std::uint32_t unit)
{
    static constexpr char hex[] = "0123456789abcdef";
    switch (unit) {
    case 0x22: out += "\\\""; return;
    case 0x5C: out += "\\\\"; return;
    case 0x08: out += "\\b"; return;
    case 0x0C: out += "\\f"; return;
    case 0x0A: out += "\\n"; return;
    case 0x0D: out += "\\r"; return;
    case 0x09: out += "\\t"; return;
    default: break;
    }
    if (unit >= 0x20 && unit <= 0x7E) {
        out += static_cast<char>(unit);
        return;
    }
    const std::array<char, 6> escape{'\\', 'u', hex[unit >> 12 & 15], hex[unit >> 8 & 15], hex[unit >> 4 & 15],
                                     hex[unit & 15]};
    out.append(escape.data(), escape.size());
}

int hex_value(unsigned char c)
{
    if (c >= '0' && c <= '9') return c - '0';
    if (c >= 'a' && c <= 'f') return c - 'a' + 10;
    if (c >= 'A' && c <= 'F') return c - 'A' + 10;
    return -1;
}

// Parses the string literal at src[pos], which must be '"', and appends it to out as it
// prints. Returns the position after the closing quote, or nothing if the literal is invalid.
std::optional<std::size_t> parse_string(std::string_view src, std::size_t pos, std::string& out)
{
    auto byte = [&](std::size_t i) { return static_cast<unsigned char>(src[i]); };
    out += '"';
    ++pos;
    for (;;) {
        std::size_t run = pos;
        while (run < src.size() && byte(run) >= 0x20 && byte(run) < 0x7F && src[run] != '"' && src[run] != '\\')
            ++run;
        out.append(src.substr(pos, run - pos));
        pos = run;
        if (pos >= src.size()) return std::nullopt;
        const unsigned char c = byte(pos++);
        if (c == '"') break;
        if (c == '\\') {
            if (pos >= src.size()) return std::nullopt;
            std::uint32_t unit = 0;
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
                if (pos + 5 > src.size()) return std::nullopt;
                for (std::size_t i = pos + 1; i < pos + 5; ++i) {
                    const int digit = hex_value(byte(i));
                    if (digit < 0) return std::nullopt;
                    unit = unit << 4 | static_cast<std::uint32_t>(digit);
                }
                pos += 4;
                break;
            default: return std::nullopt;
            }
            ++pos;
            put_unit(out, unit);
        } else if (c == 0x7F) {
            put_unit(out, c);
        } else if (c < 0x20) {
            return std::nullopt;
        } else {
            std::size_t count = 0;
            unsigned char low = 0x80, high = 0xBF;
            if (c >= 0xC2 && c <= 0xDF) count = 1;
            else if (c == 0xE0) count = 2, low = 0xA0;
            else if ((c >= 0xE1 && c <= 0xEC) || c == 0xEE || c == 0xEF) count = 2;
            else if (c == 0xED) count = 2, high = 0x9F;
            else if (c == 0xF0) count = 3, low = 0x90;
            else if (c >= 0xF1 && c <= 0xF3) count = 3;
            else if (c == 0xF4) count = 3, high = 0x8F;
            else return std::nullopt;
            if (pos + count > src.size() || byte(pos) < low || byte(pos) > high) return std::nullopt;
            std::uint32_t point = c & (0x3Fu >> count);
            for (std::size_t i = pos; i < pos + count; ++i) {
                if (byte(i) < 0x80 || byte(i) > 0xBF) return std::nullopt;
                point = point << 6 | (byte(i) & 0x3Fu);
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
    out += '"';
    return pos;
}

struct Integer {
    std::size_t end;
    bool fits;
};

// Parses -?(0|[1-9][0-9]*) at src[pos].
std::optional<Integer> parse_integer(std::string_view src, std::size_t pos)
{
    const bool negative = pos < src.size() && src[pos] == '-';
    pos += negative;
    const std::size_t first = pos;
    auto digit = [&](std::size_t i) { return i < src.size() && src[i] >= '0' && src[i] <= '9'; };
    if (pos < src.size() && src[pos] == '0') {
        ++pos;
    } else if (digit(pos)) {
        while (digit(pos)) ++pos;
    } else {
        return std::nullopt;
    }
    const std::string_view digits = src.substr(first, pos - first);
    const std::string_view limit = negative ? min_i64 : max_i64;
    return Integer{pos, digits.size() < limit.size() || (digits.size() == limit.size() && digits <= limit)};
}

struct Document {
    std::vector<Node> nodes;
    std::vector<std::uint32_t> kids;
    std::string text;

    std::span<const std::uint32_t> kids_of(const Node& node) const
    {
        if (node.kind == Kind::Scalar) return {};
        return {kids.data() + node.start, node.length};
    }

    std::string_view name(std::uint32_t id) const
    {
        const Node& node = nodes[id];
        return std::string_view(text).substr(node.name_start, node.name_length);
    }
};

class Parser {
public:
    explicit Parser(std::string_view src) : src_(src) { doc_.text.reserve(src.size()); }

    std::optional<Document> parse()
    {
        if (!value(0, 0)) return std::nullopt;
        skip_whitespace();
        if (pos_ != src_.size()) return std::nullopt;
        return std::move(doc_);
    }

private:
    void skip_whitespace()
    {
        while (pos_ < src_.size() && (src_[pos_] == ' ' || src_[pos_] == '\t' || src_[pos_] == '\n' || src_[pos_] == '\r'))
            ++pos_;
    }

    bool eat(char c)
    {
        skip_whitespace();
        if (pos_ < src_.size() && src_[pos_] == c) {
            ++pos_;
            return true;
        }
        return false;
    }

    bool string(std::uint32_t& start, std::uint32_t& length)
    {
        const std::size_t before = doc_.text.size();
        const auto end = parse_string(src_, pos_, doc_.text);
        if (!end) return false;
        pos_ = *end;
        start = static_cast<std::uint32_t>(before);
        length = static_cast<std::uint32_t>(doc_.text.size() - before);
        return true;
    }

    bool value(std::uint32_t name_start, std::uint32_t name_length)
    {
        skip_whitespace();
        if (pos_ >= src_.size()) return false;
        const std::size_t id = doc_.nodes.size();
        Node node{.name_start = name_start, .name_length = name_length};
        doc_.nodes.push_back(node);
        const char c = src_[pos_];
        if (c == '[' || c == '{') {
            ++pos_;
            const char close = c == '[' ? ']' : '}';
            node.kind = c == '[' ? Kind::Array : Kind::Object;
            const std::size_t mark = pending_.size();
            if (!eat(close)) {
                for (;;) {
                    std::uint32_t ns = 0, nl = 0;
                    if (node.kind == Kind::Object) {
                        skip_whitespace();
                        if (pos_ >= src_.size() || src_[pos_] != '"' || !string(ns, nl) || !eat(':')) return false;
                    }
                    pending_.push_back(static_cast<std::uint32_t>(doc_.nodes.size()));
                    if (!value(ns, nl)) return false;
                    if (eat(close)) break;
                    if (!eat(',')) return false;
                }
            }
            node.start = static_cast<std::uint32_t>(doc_.kids.size());
            node.length = static_cast<std::uint32_t>(pending_.size() - mark);
            doc_.kids.insert(doc_.kids.end(), pending_.begin() + static_cast<std::ptrdiff_t>(mark), pending_.end());
            pending_.resize(mark);
        } else if (c == '"') {
            if (!string(node.start, node.length)) return false;
        } else if (c == '-' || (c >= '0' && c <= '9')) {
            const auto integer = parse_integer(src_, pos_);
            if (!integer || !integer->fits) return false;
            const std::string_view digits = src_.substr(pos_, integer->end - pos_);
            node.start = static_cast<std::uint32_t>(doc_.text.size());
            doc_.text += digits == "-0" ? "0" : digits;
            node.length = static_cast<std::uint32_t>(doc_.text.size() - node.start);
            pos_ = integer->end;
        } else {
            const std::string_view rest = src_.substr(pos_);
            std::string_view word;
            for (const std::string_view w : {"true", "false", "null"})
                if (rest.starts_with(w)) word = w;
            if (word.empty()) return false;
            node.start = static_cast<std::uint32_t>(doc_.text.size());
            node.length = static_cast<std::uint32_t>(word.size());
            doc_.text += word;
            pos_ += word.size();
        }
        node.end = static_cast<std::uint32_t>(doc_.nodes.size());
        doc_.nodes[id] = node;
        return true;
    }

    std::string_view src_;
    std::size_t pos_ = 0;
    Document doc_;
    std::vector<std::uint32_t> pending_;
};

// Parses an optional slice bound or index at q[pos]. The outer optional is empty when one is
// malformed; the inner one when none is there.
std::optional<std::optional<std::int64_t>> parse_bound(std::string_view q, std::size_t& pos)
{
    if (pos >= q.size() || (q[pos] != '-' && (q[pos] < '0' || q[pos] > '9'))) return std::optional<std::int64_t>{};
    const auto integer = parse_integer(q, pos);
    if (!integer || !integer->fits) return std::nullopt;
    std::int64_t value = 0;
    std::from_chars(q.data() + pos, q.data() + integer->end, value);
    pos = integer->end;
    return std::optional<std::int64_t>{value};
}

// Parses the selector of the bracket that opens at q[pos]; returns the position after ']'.
std::optional<std::size_t> parse_bracket(std::string_view q, std::size_t pos, Segment& segment)
{
    ++pos;
    if (pos < q.size() && q[pos] == '*') {
        segment.kind = SelectorKind::Wild;
        ++pos;
    } else if (pos < q.size() && q[pos] == '"') {
        segment.kind = SelectorKind::Name;
        const auto end = parse_string(q, pos, segment.name);
        if (!end) return std::nullopt;
        pos = *end;
    } else {
        const auto start = parse_bound(q, pos);
        if (!start) return std::nullopt;
        if (pos < q.size() && q[pos] == ':') {
            ++pos;
            segment.kind = SelectorKind::Slice;
            segment.start = *start;
            const auto end = parse_bound(q, pos);
            if (!end) return std::nullopt;
            segment.end = *end;
            if (pos < q.size() && q[pos] == ':') {
                ++pos;
                const auto step = parse_bound(q, pos);
                if (!step || (*step && **step < 1)) return std::nullopt;
                segment.step = step->value_or(1);
            }
        } else {
            if (!*start) return std::nullopt;
            segment.kind = SelectorKind::Index;
            segment.index = **start;
        }
    }
    if (pos >= q.size() || q[pos] != ']') return std::nullopt;
    return pos + 1;
}

bool is_name_start(char c)
{
    return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_';
}

std::optional<std::vector<Segment>> parse_query(std::string_view q)
{
    if (q.empty() || q[0] != '$') return std::nullopt;
    std::vector<Segment> segments;
    for (std::size_t pos = 1; pos < q.size();) {
        Segment segment;
        bool dotted = false;
        if (q.substr(pos).starts_with("..")) {
            segment.descendant = dotted = true;
            pos += 2;
        } else if (q[pos] == '.') {
            dotted = true;
            ++pos;
        }
        if (pos < q.size() && q[pos] == '[' && (segment.descendant || !dotted)) {
            const auto end = parse_bracket(q, pos, segment);
            if (!end) return std::nullopt;
            pos = *end;
        } else if (dotted && pos < q.size() && q[pos] == '*') {
            segment.kind = SelectorKind::Wild;
            ++pos;
        } else if (dotted && pos < q.size() && is_name_start(q[pos])) {
            const std::size_t start = pos;
            while (pos < q.size() && (is_name_start(q[pos]) || (q[pos] >= '0' && q[pos] <= '9'))) ++pos;
            segment.kind = SelectorKind::Name;
            segment.name = '"' + std::string(q.substr(start, pos - start)) + '"';
        } else {
            return std::nullopt;
        }
        segments.push_back(std::move(segment));
    }
    return segments;
}

struct Range {
    std::size_t start = 0, end = 0, step = 1;
};

// Normalizes a slice to its first position, the bound it stays below, and its step. The step
// is capped at length + 1, which picks the same positions and cannot overflow.
Range slice_range(const Segment& s, std::size_t length)
{
    const auto n = static_cast<std::int64_t>(length);
    auto clamp = [n](std::int64_t bound) {
        if (bound < 0) bound += n;
        return static_cast<std::size_t>(std::clamp<std::int64_t>(bound, 0, n));
    };
    return {s.start ? clamp(*s.start) : 0, s.end ? clamp(*s.end) : length,
            static_cast<std::size_t>(std::min(s.step, n + 1))};
}

std::optional<std::size_t> index_position(std::int64_t index, std::size_t length)
{
    if (index < 0) index += static_cast<std::int64_t>(length);
    if (index < 0 || index >= static_cast<std::int64_t>(length)) return std::nullopt;
    return static_cast<std::size_t>(index);
}

void children(const Document& doc, std::uint32_t id, const Segment& s, std::vector<std::uint32_t>& out)
{
    const Node& node = doc.nodes[id];
    const auto kids = doc.kids_of(node);
    if (node.kind == Kind::Object && s.kind == SelectorKind::Name) {
        for (const std::uint32_t kid : kids)
            if (doc.name(kid) == s.name) out.push_back(kid);
    } else if (node.kind != Kind::Scalar && s.kind == SelectorKind::Wild) {
        out.insert(out.end(), kids.begin(), kids.end());
    } else if (node.kind == Kind::Array && s.kind == SelectorKind::Index) {
        if (const auto j = index_position(s.index, kids.size())) out.push_back(kids[*j]);
    } else if (node.kind == Kind::Array && s.kind == SelectorKind::Slice) {
        const Range r = slice_range(s, kids.size());
        for (std::size_t j = r.start; j < r.end; j += r.step) out.push_back(kids[j]);
    }
}

// Appends what the selector picks from id and each of its descendants, in document order.
void descend(const Document& doc, std::uint32_t id, const Segment& s, std::vector<std::uint32_t>& out)
{
    const Node& node = doc.nodes[id];
    if (node.kind == Kind::Scalar) return;
    const auto kids = doc.kids_of(node);
    Range r;
    if (node.kind == Kind::Array && s.kind == SelectorKind::Index) {
        if (const auto j = index_position(s.index, kids.size())) r = {*j, *j + 1, 1};
    } else if (node.kind == Kind::Array && s.kind == SelectorKind::Slice) {
        r = slice_range(s, kids.size());
    }
    for (std::size_t j = 0; j < kids.size(); ++j) {
        bool picked = false;
        switch (s.kind) {
        case SelectorKind::Name: picked = node.kind == Kind::Object && doc.name(kids[j]) == s.name; break;
        case SelectorKind::Wild: picked = true; break;
        default: picked = j >= r.start && j < r.end && (j - r.start) % r.step == 0; break;
        }
        if (picked) out.push_back(kids[j]);
        descend(doc, kids[j], s, out);
    }
}

// The nodes a query selects, each once, in document order. A set holds no node nested in
// another until a descendant segment, and only a nested set can come out unsorted.
std::vector<std::uint32_t> evaluate(const Document& doc, const std::vector<Segment>& segments)
{
    std::vector<std::uint32_t> nodes{0};
    bool nested = false;
    for (const Segment& s : segments) {
        std::vector<std::uint32_t> out;
        if (s.descendant) {
            std::uint32_t covered = 0;
            for (const std::uint32_t id : nodes) {
                if (id >= covered) {
                    descend(doc, id, s, out);
                    covered = doc.nodes[id].end;
                }
            }
            nested = true;
        } else {
            for (const std::uint32_t id : nodes) children(doc, id, s, out);
            if (nested) std::ranges::sort(out);
        }
        nodes = std::move(out);
    }
    return nodes;
}

void print(const Document& doc, std::uint32_t id, std::string& out)
{
    const Node& node = doc.nodes[id];
    if (node.kind == Kind::Scalar) {
        out.append(std::string_view(doc.text).substr(node.start, node.length));
        return;
    }
    out += node.kind == Kind::Array ? '[' : '{';
    bool first = true;
    for (const std::uint32_t kid : doc.kids_of(node)) {
        if (!first) out += ',';
        first = false;
        if (node.kind == Kind::Object) {
            out.append(doc.name(kid));
            out += ':';
        }
        print(doc, kid, out);
    }
    out += node.kind == Kind::Array ? ']' : '}';
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

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    std::string_view rest = input;
    std::size_t newline = rest.find('\n');
    std::size_t count = 0;
    std::from_chars(rest.data(), rest.data() + newline, count);
    rest.remove_prefix(newline + 1);
    std::vector<std::string_view> queries;
    for (std::size_t i = 0; i < count; ++i) {
        newline = rest.find('\n');
        queries.push_back(rest.substr(0, newline));
        rest.remove_prefix(newline + 1);
    }

    std::string out;
    if (auto doc = Parser(rest).parse()) {
        for (const std::string_view query : queries) {
            const auto segments = parse_query(query);
            if (!segments) {
                out += "invalid query\n";
                continue;
            }
            const auto nodes = evaluate(*doc, *segments);
            out += std::to_string(nodes.size());
            out += '\n';
            for (const std::uint32_t id : nodes) {
                print(*doc, id, out);
                out += '\n';
            }
        }
    } else {
        out = "invalid document\n";
    }
    std::fwrite(out.data(), 1, out.size(), stdout);
    return std::fflush(stdout) == 0 ? 0 : 1;
}
