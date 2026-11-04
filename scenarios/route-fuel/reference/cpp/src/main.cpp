#include <algorithm>
#include <charconv>
#include <concepts>
#include <cstdint>
#include <functional>
#include <iostream>
#include <optional>
#include <queue>
#include <string>
#include <string_view>
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

struct Road {
    int to;
    int fuel;
    std::int64_t time;
};

struct State {
    std::int64_t cost;
    std::int64_t time;
    int place;
    int fuel;

    bool operator>(const State& other) const
    {
        return std::pair{cost, time} > std::pair{other.cost, other.time};
    }
};

class Search {
public:
    Search(int capacity, std::vector<std::int64_t> price, std::vector<std::vector<Road>> roads)
        : capacity_(capacity), price_(std::move(price)), roads_(std::move(roads)),
          most_(roads_.size()), trip_(roads_.size(), -1)
    {
    }

    // Dijkstra over (place, fuel) states ordered by (cost, time), buying one unit per step. A
    // state is dominated once its place was expanded with at least as much fuel, since that
    // earlier state was no costlier and no slower.
    std::optional<State> best_trip(int id, int start, int fuel, int target)
    {
        std::priority_queue<State, std::vector<State>, std::greater<>> heap;
        heap.push({0, 0, start, fuel});
        while (!heap.empty()) {
            const State cur = heap.top();
            heap.pop();
            if (expanded(id, cur.place) >= cur.fuel) {
                continue;
            }
            if (cur.place == target) {
                return cur;
            }
            trip_[cur.place] = id;
            most_[cur.place] = cur.fuel;
            if (price_[cur.place] >= 0 && cur.fuel < capacity_) {
                heap.push({cur.cost + price_[cur.place], cur.time, cur.place, cur.fuel + 1});
            }
            for (const Road& road : roads_[cur.place]) {
                const int left = cur.fuel - road.fuel;
                if (left >= 0 && expanded(id, road.to) < left) {
                    heap.push({cur.cost, cur.time + road.time, road.to, left});
                }
            }
        }
        return std::nullopt;
    }

private:
    int expanded(int id, int place) const { return trip_[place] == id ? most_[place] : -1; }

    int capacity_;
    std::vector<std::int64_t> price_;  // -1 where there is no station
    std::vector<std::vector<Road>> roads_;
    std::vector<int> most_;  // the most fuel each place was expanded with during trip trip_[place]
    std::vector<int> trip_;
};

}  // namespace

int main()
{
    const std::string input = read_all(std::cin);
    std::string_view text = input;
    const auto n = take<int>(text);
    const auto capacity = take<int>(text);
    const auto stations = take<int>(text);
    const auto m = take<int>(text);
    const auto q = take<int>(text);

    std::vector<std::int64_t> price(n + 1, -1);
    for (int i = 0; i < stations; ++i) {
        const auto place = take<int>(text);
        price[place] = take<std::int64_t>(text);
    }
    std::vector<std::vector<Road>> roads(n + 1);
    for (int i = 0; i < m; ++i) {
        const auto from = take<int>(text);
        const auto to = take<int>(text);
        const auto fuel = take<int>(text);
        const auto time = take<std::int64_t>(text);
        if (fuel <= capacity) {
            roads[from].push_back({to, fuel, time});
        }
    }

    Search search(capacity, std::move(price), std::move(roads));
    std::string output;
    for (int id = 0; id < q; ++id) {
        const auto start = take<int>(text);
        const auto fuel = take<int>(text);
        const auto target = take<int>(text);
        if (const auto best = search.best_trip(id, start, fuel, target)) {
            output += std::to_string(best->cost) + ' ' + std::to_string(best->time) + '\n';
        } else {
            output += "unreachable\n";
        }
    }
    std::cout << output;
}
