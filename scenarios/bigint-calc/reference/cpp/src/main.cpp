#include <algorithm>
#include <bit>
#include <cstddef>
#include <cstdint>
#include <iostream>
#include <optional>
#include <span>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace {

using Limb = std::uint32_t;
using Limbs = std::vector<Limb>;

// Limbs are base 10^9, so reading and printing decimal take linear time.
constexpr Limb BASE = 1'000'000'000;
constexpr std::uint64_t BASE64 = BASE;
constexpr std::size_t BASE_DIGITS = 9;
// Below this many limbs in the shorter operand, schoolbook multiplication beats Karatsuba.
constexpr std::size_t KARATSUBA_CUTOFF = 40;

std::span<const Limb> trimmed(std::span<const Limb> a)
{
    std::size_t n = a.size();
    while (n > 0 && a[n - 1] == 0)
        --n;
    return a.first(n);
}

int compare(std::span<const Limb> a, std::span<const Limb> b)
{
    if (a.size() != b.size())
        return a.size() < b.size() ? -1 : 1;
    for (std::size_t i = a.size(); i-- > 0;)
        if (a[i] != b[i])
            return a[i] < b[i] ? -1 : 1;
    return 0;
}

// r += a, for a no longer than r and a sum that fits in r.
void add_into(std::span<Limb> r, std::span<const Limb> a)
{
    bool carry = false;
    std::size_t i = 0;
    for (; i < a.size(); ++i) {
        Limb s = r[i] + a[i] + carry;
        carry = s >= BASE;
        r[i] = carry ? s - BASE : s;
    }
    for (; carry && i < r.size(); ++i) {
        carry = r[i] == BASE - 1;
        r[i] = carry ? 0 : r[i] + 1;
    }
}

// r -= a, for a no longer than r and r >= a.
void sub_into(std::span<Limb> r, std::span<const Limb> a)
{
    bool borrow = false;
    std::size_t i = 0;
    for (; i < a.size(); ++i) {
        Limb s = a[i] + borrow;
        borrow = r[i] < s;
        r[i] = borrow ? r[i] + BASE - s : r[i] - s;
    }
    for (; borrow && i < r.size(); ++i) {
        borrow = r[i] == 0;
        r[i] = borrow ? BASE - 1 : r[i] - 1;
    }
}

void mul_schoolbook(std::span<Limb> r, std::span<const Limb> a, std::span<const Limb> b)
{
    std::ranges::fill(r, 0);
    for (std::size_t i = 0; i < a.size(); ++i) {
        std::uint64_t carry = 0;
        for (std::size_t j = 0; j < b.size(); ++j) {
            std::uint64_t t = r[i + j] + std::uint64_t{a[i]} * b[j] + carry;
            r[i + j] = static_cast<Limb>(t % BASE64);
            carry = t / BASE64;
        }
        r[i + b.size()] = static_cast<Limb>(carry);
    }
}

// r = a * b, where r has a.size() + b.size() limbs. The operands may have high zero limbs.
void mul_into(std::span<Limb> r, std::span<const Limb> a, std::span<const Limb> b)
{
    if (a.size() < b.size())
        std::swap(a, b);
    const std::size_t an = a.size(), bn = b.size();
    if (bn < KARATSUBA_CUTOFF) {
        mul_schoolbook(r, a, b);
        return;
    }
    if (an >= 2 * bn) {
        // Unbalanced: multiply b by each bn-limb slice of a and add the products.
        std::ranges::fill(r, 0);
        Limbs t(2 * bn);
        for (std::size_t i = 0; i < an; i += bn) {
            const std::size_t n = std::min(bn, an - i);
            const std::span<Limb> product = std::span(t).first(n + bn);
            mul_into(product, a.subspan(i, n), b);
            add_into(r.subspan(i), product);
        }
        return;
    }
    // Karatsuba with a = a1 B^m + a0 and b = b1 B^m + b0, where m < bn <= an < 2 bn.
    const std::size_t m = an / 2;
    const auto a0 = a.first(m), a1 = a.subspan(m), b0 = b.first(m), b1 = b.subspan(m);
    Limbs sa(a1.size() + 1);
    std::ranges::copy(a1, sa.begin());
    add_into(sa, a0);
    Limbs sb(std::max(m, b1.size()) + 1);
    std::ranges::copy(b0, sb.begin());
    add_into(sb, b1);
    mul_into(r.first(2 * m), a0, b0);
    mul_into(r.subspan(2 * m), a1, b1);
    Limbs z(sa.size() + sb.size());
    mul_into(z, sa, sb);
    sub_into(z, r.first(2 * m));
    sub_into(z, r.subspan(2 * m));
    add_into(r.subspan(m), trimmed(z));
}

// a * k for a single limb k, with one more limb than a.
Limbs mul_limb(std::span<const Limb> a, Limb k)
{
    Limbs r(a.size() + 1);
    std::uint64_t carry = 0;
    for (std::size_t i = 0; i < a.size(); ++i) {
        std::uint64_t t = std::uint64_t{a[i]} * k + carry;
        r[i] = static_cast<Limb>(t % BASE64);
        carry = t / BASE64;
    }
    r[a.size()] = static_cast<Limb>(carry);
    return r;
}

// (u / v, u % v) for u no shorter than v and v without high zero limbs: Knuth's Algorithm D
// (TAOCP 4.3.1).
std::pair<Limbs, Limbs> divmod(std::span<const Limb> u, std::span<const Limb> v)
{
    const std::size_t un = u.size(), vn = v.size();
    Limbs q(un - vn + 1);
    if (vn == 1) {
        std::uint64_t rem = 0;
        for (std::size_t i = un; i-- > 0;) {
            std::uint64_t cur = rem * BASE64 + u[i];
            q[i] = static_cast<Limb>(cur / v[0]);
            rem = cur % v[0];
        }
        return {std::move(q), Limbs{static_cast<Limb>(rem)}};
    }
    // Scale both so the divisor's top limb is at least BASE / 2, which keeps each estimated
    // quotient limb at most one too large after the two-limb test.
    const Limb d = static_cast<Limb>(BASE64 / (std::uint64_t{v[vn - 1]} + 1));
    Limbs w = mul_limb(u, d);
    const Limbs y = mul_limb(v, d);
    const std::uint64_t top = y[vn - 1], next = y[vn - 2];
    for (std::size_t j = un - vn + 1; j-- > 0;) {
        const std::uint64_t num = w[j + vn] * BASE64 + w[j + vn - 1];
        std::uint64_t qhat = num / top, rhat = num % top;
        while (qhat >= BASE64 || qhat * next > rhat * BASE64 + w[j + vn - 2]) {
            --qhat;
            rhat += top;
            if (rhat >= BASE64)
                break;
        }
        std::uint64_t carry = 0;
        std::int64_t borrow = 0;
        for (std::size_t i = 0; i < vn; ++i) {
            const std::uint64_t p = qhat * y[i] + carry;
            carry = p / BASE64;
            const std::int64_t t = std::int64_t{w[i + j]} - static_cast<std::int64_t>(p % BASE64) - borrow;
            borrow = t < 0;
            w[i + j] = static_cast<Limb>(t < 0 ? t + static_cast<std::int64_t>(BASE64) : t);
        }
        std::int64_t t = std::int64_t{w[j + vn]} - static_cast<std::int64_t>(carry) - borrow;
        if (t < 0) {
            // The estimate was one too large, which is rare: add the divisor back.
            --qhat;
            bool c = false;
            for (std::size_t i = 0; i < vn; ++i) {
                const Limb s = w[i + j] + y[i] + c;
                c = s >= BASE;
                w[i + j] = c ? s - BASE : s;
            }
            t += c;
        }
        w[j + vn] = static_cast<Limb>(t);
        q[j] = static_cast<Limb>(qhat);
    }
    Limbs r(vn);
    std::uint64_t rem = 0;
    for (std::size_t i = vn; i-- > 0;) {
        const std::uint64_t cur = rem * BASE64 + w[i];
        r[i] = static_cast<Limb>(cur / d);
        rem = cur % d;
    }
    return {std::move(q), std::move(r)};
}

// A signed integer: little-endian limbs with no high zero limb, so zero has no limbs and is
// never negative.
struct Int {
    Limbs limbs;
    bool negative = false;

    Int(Limbs l, bool neg) : limbs(std::move(l))
    {
        limbs.resize(trimmed(limbs).size());
        negative = neg && !limbs.empty();
    }

    static Int parse(std::string_view digits)
    {
        digits.remove_prefix(std::min(digits.find_first_not_of('0'), digits.size()));
        Limbs limbs((digits.size() + BASE_DIGITS - 1) / BASE_DIGITS);
        for (std::size_t i = 0; i < limbs.size(); ++i) {
            const std::size_t end = digits.size() - i * BASE_DIGITS;
            const std::size_t start = end > BASE_DIGITS ? end - BASE_DIGITS : 0;
            Limb limb = 0;
            for (std::size_t k = start; k < end; ++k)
                limb = limb * 10 + static_cast<Limb>(digits[k] - '0');
            limbs[i] = limb;
        }
        return Int(std::move(limbs), false);
    }

    // a + b, or a - b when subtract is set.
    friend Int add(const Int& x, const Int& y, bool subtract)
    {
        const bool yneg = y.negative != subtract;
        const Int* a = &x;
        const Int* b = &y;
        bool negative = x.negative;
        if (a->negative == yneg) {
            if (a->limbs.size() < b->limbs.size())
                std::swap(a, b);
            Limbs r = a->limbs;
            r.push_back(0);
            add_into(r, b->limbs);
            return Int(std::move(r), negative);
        }
        // Opposite signs: subtract the smaller magnitude from the larger.
        if (compare(a->limbs, b->limbs) < 0) {
            std::swap(a, b);
            negative = yneg;
        }
        Limbs r = a->limbs;
        sub_into(r, b->limbs);
        return Int(std::move(r), negative);
    }

    friend Int mul(const Int& a, const Int& b)
    {
        if (a.limbs.empty() || b.limbs.empty())
            return Int({}, false);
        Limbs r(a.limbs.size() + b.limbs.size());
        mul_into(r, a.limbs, b.limbs);
        return Int(std::move(r), a.negative != b.negative);
    }

    // The quotient rounded toward negative infinity, or the remainder that goes with it. b is
    // nonzero.
    friend Int divide(const Int& a, const Int& b, bool remainder)
    {
        auto [q, r] = a.limbs.size() < b.limbs.size() ? std::pair{Limbs{}, a.limbs} : divmod(a.limbs, b.limbs);
        if (!trimmed(r).empty() && a.negative != b.negative) {
            // Truncation rounded toward zero, so step the quotient down by one and move the
            // remainder to the divisor's sign: |r| becomes |b| - |r|.
            q.push_back(0);
            const Limb one = 1;
            add_into(q, std::span(&one, 1));
            Limbs flipped = b.limbs;
            sub_into(flipped, trimmed(r));
            r = std::move(flipped);
        }
        return remainder ? Int(std::move(r), b.negative) : Int(std::move(q), a.negative != b.negative);
    }

    // a ^ e for e >= 0.
    friend Int power(const Int& a, const Int& e)
    {
        if (a.limbs.empty())
            return Int({e.limbs.empty() ? 1u : 0u}, false);
        if (a.limbs.size() == 1 && a.limbs[0] == 1)
            return Int({1}, a.negative && !e.limbs.empty() && (e.limbs[0] & 1));
        // Any other base keeps the exponent below 664386, so it fits in one limb.
        const Limb n = e.limbs.empty() ? 0 : e.limbs[0];
        Int result({1}, false);
        for (int bit = std::bit_width(n) - 1; bit >= 0; --bit) {
            result = mul(result, result);
            if ((n >> bit) & 1)
                result = mul(result, a);
        }
        return result;
    }

    void write(std::string& out) const
    {
        if (limbs.empty()) {
            out += '0';
            return;
        }
        if (negative)
            out += '-';
        out += std::to_string(limbs.back());
        for (std::size_t i = limbs.size() - 1; i-- > 0;) {
            char digits[BASE_DIGITS];
            Limb v = limbs[i];
            for (std::size_t k = BASE_DIGITS; k-- > 0;) {
                digits[k] = static_cast<char>('0' + v % 10);
                v /= 10;
            }
            out.append(digits, BASE_DIGITS);
        }
    }
};

enum class Kind { Number, Add, Sub, Mul, Div, Mod, Neg, Pow, Open };

// Binding strength. `^` and unary minus group from the right, the others from the left.
constexpr int rank(Kind kind)
{
    switch (kind) {
    case Kind::Add:
    case Kind::Sub:
        return 1;
    case Kind::Mul:
    case Kind::Div:
    case Kind::Mod:
        return 2;
    case Kind::Neg:
        return 3;
    case Kind::Pow:
        return 4;
    default:
        return 0;
    }
}

struct Item {
    Kind kind;
    std::string_view digits;
};

constexpr bool is_digit(char c)
{
    return '0' <= c && c <= '9';
}

// Converts a line to postfix order with the shunting-yard algorithm, or returns nothing when
// it does not match the grammar.
std::optional<std::vector<Item>> postfix(std::string_view line)
{
    std::vector<Item> items, ops;
    bool want_operand = true;
    for (std::size_t i = 0; i < line.size();) {
        const char c = line[i];
        if (c == ' ' || c == '\t') {
            ++i;
            continue;
        }
        if (is_digit(c)) {
            if (!want_operand)
                return std::nullopt;
            const std::size_t start = i;
            while (i < line.size() && is_digit(line[i]))
                ++i;
            items.push_back({Kind::Number, line.substr(start, i - start)});
            want_operand = false;
            continue;
        }
        ++i;
        if (want_operand) {
            if (c == '(')
                ops.push_back({Kind::Open, {}});
            else if (c == '-')
                ops.push_back({Kind::Neg, {}});
            else
                return std::nullopt;
            continue;
        }
        if (c == ')') {
            while (!ops.empty() && ops.back().kind != Kind::Open) {
                items.push_back(ops.back());
                ops.pop_back();
            }
            if (ops.empty())
                return std::nullopt;
            ops.pop_back();
            continue;
        }
        Kind kind;
        switch (c) {
        case '+': kind = Kind::Add; break;
        case '-': kind = Kind::Sub; break;
        case '*': kind = Kind::Mul; break;
        case '/': kind = Kind::Div; break;
        case '%': kind = Kind::Mod; break;
        case '^': kind = Kind::Pow; break;
        default: return std::nullopt;
        }
        while (!ops.empty() && ops.back().kind != Kind::Open) {
            const int top = rank(ops.back().kind);
            if (top < rank(kind) || (top == rank(kind) && kind == Kind::Pow))
                break;
            items.push_back(ops.back());
            ops.pop_back();
        }
        ops.push_back({kind, {}});
        want_operand = true;
    }
    if (want_operand)
        return std::nullopt;
    while (!ops.empty()) {
        if (ops.back().kind == Kind::Open)
            return std::nullopt;
        items.push_back(ops.back());
        ops.pop_back();
    }
    return items;
}

// Appends the line's value or error message to out.
void evaluate(std::string_view line, std::string& out)
{
    const auto items = postfix(line);
    if (!items) {
        out += "syntax error";
        return;
    }
    std::vector<Int> values;
    for (const Item& item : *items) {
        if (item.kind == Kind::Number) {
            values.push_back(Int::parse(item.digits));
            continue;
        }
        if (item.kind == Kind::Neg) {
            Int& x = values.back();
            x.negative = !x.negative && !x.limbs.empty();
            continue;
        }
        const Int b = std::move(values.back());
        values.pop_back();
        Int& a = values.back();
        switch (item.kind) {
        case Kind::Add: a = add(a, b, false); break;
        case Kind::Sub: a = add(a, b, true); break;
        case Kind::Mul: a = mul(a, b); break;
        case Kind::Pow:
            if (b.negative) {
                out += "negative exponent";
                return;
            }
            a = power(a, b);
            break;
        default:
            if (b.limbs.empty()) {
                out += "division by zero";
                return;
            }
            a = divide(a, b, item.kind == Kind::Mod);
        }
    }
    values[0].write(out);
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
    std::string out;
    for (std::size_t start = 0; start < input.size();) {
        const std::size_t end = std::min(input.find('\n', start), input.size());
        evaluate(std::string_view(input).substr(start, end - start), out);
        out += '\n';
        start = end + 1;
    }
    std::cout << out;
}
