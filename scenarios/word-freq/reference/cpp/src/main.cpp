#include <algorithm>
#include <charconv>
#include <cstdint>
#include <format>
#include <iostream>
#include <iterator>
#include <ranges>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
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

constexpr bool is_letter(char c)
{
    return ('a' <= c && c <= 'z') || ('A' <= c && c <= 'Z');
}

constexpr char to_lower(char c)
{
    return 'A' <= c && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c;
}

}  // namespace

int main()
{
    std::string input = read_all(std::cin);
    const std::size_t newline = std::min(input.find('\n'), input.size());
    std::size_t k = 0;
    std::from_chars(input.data(), input.data() + newline, k);
    std::ranges::transform(input, input.begin(), to_lower);
    const std::string_view text = std::string_view(input).substr(std::min(newline + 1, input.size()));

    std::unordered_map<std::string_view, std::uint32_t> counts;
    for (auto first = std::ranges::find_if(text, is_letter); first != text.end();) {
        const auto last = std::find_if_not(first, text.end(), is_letter);
        ++counts[std::string_view(first, last)];
        first = std::find_if(last, text.end(), is_letter);
    }

    std::vector<std::pair<std::string_view, std::uint32_t>> ranked(counts.begin(), counts.end());
    std::ranges::sort(ranked, [](const auto& a, const auto& b) {
        return a.second != b.second ? a.second > b.second : a.first < b.first;
    });

    std::string output;
    for (const auto& [word, count] : ranked | std::views::take(k)) {
        std::format_to(std::back_inserter(output), "{} {}\n", word, count);
    }
    std::cout << output;
}
