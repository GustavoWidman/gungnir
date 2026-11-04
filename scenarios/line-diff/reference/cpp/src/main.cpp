#include <algorithm>
#include <array>
#include <charconv>
#include <cstddef>
#include <cstdint>
#include <format>
#include <iostream>
#include <iterator>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace {

constexpr std::uint32_t none = UINT32_MAX;

using Hunk = std::array<std::size_t, 4>;

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

// Numbers the lines of `doc`, giving equal lines equal numbers. A line keeps its line feed,
// so a last line without one differs from the same text with one.
std::vector<std::uint32_t> intern(std::string_view doc, std::unordered_map<std::string_view, std::uint32_t>& ids)
{
    std::vector<std::uint32_t> lines;
    while (!doc.empty()) {
        const std::size_t length = std::min(doc.find('\n'), doc.size() - 1) + 1;
        const auto [it, _] = ids.try_emplace(doc.substr(0, length), static_cast<std::uint32_t>(ids.size()));
        lines.push_back(it->second);
        doc.remove_prefix(length);
    }
    return lines;
}

// Where round e starts in the flat array of rounds: rounds 0 to e - 1 hold 1 to e entries.
constexpr std::size_t offset(std::int64_t e)
{
    return static_cast<std::size_t>(e * (e + 1) / 2);
}

// Myers' search run from the end of both documents. Entry i of round e is the smallest x on
// diagonal k = n - m - e + 2i such that (x, x - k) is at most e edits from the end, or `none`
// when no point of that diagonal is. Returns the first round that reaches (0, 0), which is the
// shortest script's cost.
std::int64_t search_back(const std::vector<std::uint32_t>& a, const std::vector<std::uint32_t>& b,
                         std::vector<std::uint32_t>& rounds)
{
    const auto n = static_cast<std::int64_t>(a.size());
    const auto m = static_cast<std::int64_t>(b.size());
    const std::int64_t kend = n - m;
    std::int64_t x = n, y = m;
    while (x > 0 && y > 0 && a[x - 1] == b[y - 1]) {
        --x;
        --y;
    }
    rounds.assign(1, static_cast<std::uint32_t>(x));
    for (std::int64_t e = 0;; ++e) {
        const std::int64_t at = e - kend;
        if (at % 2 == 0 && 0 <= at && at <= 2 * e && rounds[offset(e) + at / 2] == 0) {
            return e;
        }
        const std::int64_t next = e + 1;
        const std::size_t prev = offset(e), cur = offset(next);
        rounds.resize(cur + next + 1, none);
        // Only the diagonals -m <= k <= n cross the grid.
        const std::int64_t first = std::max<std::int64_t>(0, (next - n + 1) / 2);
        const std::int64_t last = std::min(next, (m + next) / 2);
        for (std::int64_t i = first; i <= last; ++i) {
            const std::int64_t k = kend - next + 2 * i;
            const std::int64_t lo = std::max<std::int64_t>(k, 0);
            std::int64_t best = INT64_MAX;
            if (i < next && rounds[prev + i] != none) {
                // Back from diagonal k + 1 over a deletion.
                best = std::max(static_cast<std::int64_t>(rounds[prev + i]) - 1, lo);
            }
            if (i > 0 && rounds[prev + i - 1] != none) {
                // Back from diagonal k - 1 over an insertion.
                best = std::min(best, std::max<std::int64_t>(rounds[prev + i - 1], lo));
            }
            if (best != INT64_MAX) {
                std::int64_t sx = best, sy = best - k;
                while (sx > 0 && sy > 0 && a[sx - 1] == b[sy - 1]) {
                    --sx;
                    --sy;
                }
                rounds[cur + i] = static_cast<std::uint32_t>(sx);
            }
        }
    }
}

// The hunks (x, a, y, b) of the first shortest edit script in keep, delete, insert order.
std::vector<Hunk> diff(const std::vector<std::uint32_t>& a, const std::vector<std::uint32_t>& b,
                       std::vector<std::uint32_t>& rounds)
{
    const std::size_t n = a.size(), m = b.size();
    const auto kend = static_cast<std::int64_t>(n) - static_cast<std::int64_t>(m);
    std::int64_t e = search_back(a, b, rounds);
    std::vector<Hunk> hunks;
    std::size_t x = 0, y = 0;
    const auto keeps = [&] { return x < n && y < m && a[x] == b[y]; };
    for (;;) {
        while (keeps()) {
            ++x;
            ++y;
        }
        if (x == n && y == m) {
            return hunks;
        }
        const std::size_t x0 = x, y0 = y;
        do {
            // Delete when (x + 1, y) is still within e - 1 edits of the end.
            const std::int64_t i = (static_cast<std::int64_t>(x) - static_cast<std::int64_t>(y) + e - kend) / 2;
            if (x < n && i < e && rounds[offset(e - 1) + i] <= x + 1) {
                ++x;
            } else {
                ++y;
            }
            --e;
        } while (!keeps() && !(x == n && y == m));
        hunks.push_back({x0, x - x0, y0, y - y0});
    }
}

std::size_t number(std::string_view text)
{
    std::size_t value = 0;
    std::from_chars(text.data(), text.data() + text.size(), value);
    return value;
}

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    std::string_view rest = input;
    const std::size_t pairs = number(rest.substr(0, rest.find('\n')));
    rest.remove_prefix(rest.find('\n') + 1);

    std::unordered_map<std::string_view, std::uint32_t> ids;
    std::vector<std::uint32_t> rounds;
    std::string output;
    for (std::size_t pair = 0; pair < pairs; ++pair) {
        const std::size_t space = rest.find(' '), newline = rest.find('\n');
        const std::size_t n = number(rest.substr(0, space));
        const std::size_t m = number(rest.substr(space + 1, newline - space - 1));
        rest.remove_prefix(newline + 1);
        const std::string_view first = rest.substr(0, n), second = rest.substr(n, m);
        rest.remove_prefix(n + m);

        ids.clear();
        const auto a = intern(first, ids);
        const auto b = intern(second, ids);
        const auto hunks = diff(a, b, rounds);
        std::size_t deleted = 0, inserted = 0;
        for (const auto& hunk : hunks) {
            deleted += hunk[1];
            inserted += hunk[3];
        }
        std::format_to(std::back_inserter(output), "{} {} {}\n", deleted, inserted, hunks.size());
        for (const auto& [x, a, y, b] : hunks) {
            std::format_to(std::back_inserter(output), "{} {} {} {}\n", x, a, y, b);
        }
    }
    std::cout << output;
}
