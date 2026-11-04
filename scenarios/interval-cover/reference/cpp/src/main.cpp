#include <algorithm>
#include <array>
#include <charconv>
#include <concepts>
#include <cstdint>
#include <iostream>
#include <string>
#include <string_view>
#include <vector>

namespace {

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

// Parses the integer at the front of text, after any whitespace, and removes it from text.
template <std::integral T>
T take(std::string_view& text)
{
    text.remove_prefix(std::min(text.find_first_not_of(" \t\r\n"), text.size()));
    T value{};
    const char* end = std::from_chars(text.data(), text.data() + text.size(), value).ptr;
    text.remove_prefix(static_cast<std::size_t>(end - text.data()));
    return value;
}

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    std::string_view text = input;
    const auto n = take<std::size_t>(text);
    const auto q = take<std::size_t>(text);

    std::vector<std::int64_t> starts(n);
    std::vector<std::int64_t> ends(n);
    for (std::size_t i = 0; i < n; ++i) {
        starts[i] = take<std::int64_t>(text);
        ends[i] = take<std::int64_t>(text);
    }
    std::ranges::sort(starts);
    std::ranges::sort(ends);

    std::string output;
    for (std::size_t i = 0; i < q; ++i) {
        const auto x = take<std::int64_t>(text);
        // Intervals containing x: those starting at or before x, minus those ending before x.
        const auto count = (std::ranges::upper_bound(starts, x) - starts.begin()) -
                           (std::ranges::lower_bound(ends, x) - ends.begin());
        std::array<char, 20> digits;
        const auto end = std::to_chars(digits.data(), digits.data() + digits.size(), count).ptr;
        output.append(digits.data(), end);
        output += '\n';
    }
    std::cout << output;
}
