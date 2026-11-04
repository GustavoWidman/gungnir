#include <algorithm>
#include <cstdint>
#include <iostream>
#include <string>
#include <utility>
#include <vector>

namespace {

using Shape = std::vector<std::pair<int, int>>;

// Shifts the shape to row and column 0 and sorts its cells.
Shape normalize(Shape cells)
{
    int top = cells[0].first;
    int left = cells[0].second;
    for (const auto& [r, c] : cells) {
        top = std::min(top, r);
        left = std::min(left, c);
    }
    for (auto& [r, c] : cells) {
        r -= top;
        c -= left;
    }
    std::ranges::sort(cells);
    return cells;
}

// The distinct shapes the 4 rotations and their mirror images give, the smallest first.
std::vector<Shape> orientations(Shape cells)
{
    std::vector<Shape> found;
    for (int mirror = 0; mirror < 2; ++mirror) {
        for (int turn = 0; turn < 4; ++turn) {
            for (auto& [r, c] : cells)
                std::tie(r, c) = std::pair{c, -r};
            found.push_back(normalize(cells));
        }
        for (auto& cell : cells)
            cell.second = -cell.second;
    }
    std::ranges::sort(found);
    auto [first, last] = std::ranges::unique(found);
    found.erase(first, last);
    return found;
}

// Algorithm X with dancing links. Cell columns are primary; each shape has one more column,
// covered once all its copies are placed, so copies of a shape are never told apart.
class Dlx {
public:
    Dlx(int cells, std::vector<int> copies)
        : cells_(cells), root_(cells + static_cast<int>(copies.size())), copies_(std::move(copies))
    {
        for (int c = 0; c <= root_; ++c) {
            left_.push_back(c);
            right_.push_back(c);
            up_.push_back(c);
            down_.push_back(c);
            column_.push_back(c);
        }
        size_.assign(static_cast<std::size_t>(root_) + 1, 0);
        // Only the cell columns and the root form the header ring.
        int previous = root_;
        for (int c = 0; c < cells_; ++c) {
            right_[previous] = c;
            left_[c] = previous;
            previous = c;
        }
        right_[previous] = root_;
        left_[root_] = previous;
    }

    void add_row(const std::vector<int>& columns)
    {
        const int first = static_cast<int>(left_.size());
        const int count = static_cast<int>(columns.size());
        for (int k = 0; k < count; ++k) {
            const int node = first + k;
            const int col = columns[k];
            column_.push_back(col);
            up_.push_back(up_[col]);
            down_.push_back(col);
            down_[up_[col]] = node;
            up_[col] = node;
            ++size_[col];
            left_.push_back(k > 0 ? node - 1 : first + count - 1);
            right_.push_back(k + 1 < count ? node + 1 : first);
        }
    }

    // The number of exact covers, choosing at each step the cell with the fewest options.
    std::uint64_t count()
    {
        int best = right_[root_];
        if (best == root_)
            return 1;
        for (int c = right_[best]; c != root_ && size_[best] > 0; c = right_[c]) {
            if (size_[c] < size_[best])
                best = c;
        }
        if (size_[best] == 0)
            return 0;
        cover(best);
        std::uint64_t total = 0;
        for (int row = down_[best]; row != best; row = down_[row]) {
            place(row);
            total += count();
            unplace(row);
        }
        uncover(best);
        return total;
    }

private:
    void cover(int c)
    {
        left_[right_[c]] = left_[c];
        right_[left_[c]] = right_[c];
        for (int i = down_[c]; i != c; i = down_[i]) {
            for (int j = right_[i]; j != i; j = right_[j]) {
                up_[down_[j]] = up_[j];
                down_[up_[j]] = down_[j];
                --size_[column_[j]];
            }
        }
    }

    void uncover(int c)
    {
        for (int i = up_[c]; i != c; i = up_[i]) {
            for (int j = left_[i]; j != i; j = left_[j]) {
                ++size_[column_[j]];
                up_[down_[j]] = j;
                down_[up_[j]] = j;
            }
        }
        left_[right_[c]] = c;
        right_[left_[c]] = c;
    }

    void place(int row)
    {
        for (int j = right_[row]; j != row; j = right_[j]) {
            const int col = column_[j];
            if (col < cells_ || --copies_[col - cells_] == 0)
                cover(col);
        }
    }

    void unplace(int row)
    {
        for (int j = left_[row]; j != row; j = left_[j]) {
            const int col = column_[j];
            if (col < cells_ || copies_[col - cells_]++ == 0)
                uncover(col);
        }
    }

    int cells_;
    int root_;
    std::vector<int> copies_;
    std::vector<int> left_, right_, up_, down_, column_, size_;
};

}  // namespace

int main()
{
    std::ios::sync_with_stdio(false);
    int height = 0;
    int width = 0;
    std::cin >> height >> width;
    std::vector<int> index(static_cast<std::size_t>(height * width), -1);
    int cells = 0;
    for (int r = 0; r < height; ++r) {
        std::string line;
        std::cin >> line;
        for (int c = 0; c < width; ++c) {
            if (line[c] == '.')
                index[r * width + c] = cells++;
        }
    }

    int pieces = 0;
    std::cin >> pieces;
    std::vector<Shape> shapes;
    std::vector<int> copies;
    int area = 0;
    for (int p = 0; p < pieces; ++p) {
        int rows = 0;
        int columns = 0;
        std::cin >> rows >> columns;
        Shape piece;
        for (int r = 0; r < rows; ++r) {
            std::string line;
            std::cin >> line;
            for (int c = 0; c < columns; ++c) {
                if (line[c] == '#')
                    piece.emplace_back(r, c);
            }
        }
        area += static_cast<int>(piece.size());
        const Shape canonical = orientations(piece).front();
        const auto kind = std::ranges::find(shapes, canonical) - shapes.begin();
        if (kind == std::ssize(shapes)) {
            shapes.push_back(canonical);
            copies.push_back(0);
        }
        ++copies[kind];
    }

    std::uint64_t found = 0;
    if (area == cells) {
        Dlx dlx(cells, copies);
        for (int kind = 0; kind < std::ssize(shapes); ++kind) {
            for (const Shape& shape : orientations(shapes[kind])) {
                for (int top = 0; top < height; ++top) {
                    for (int left = 0; left < width; ++left) {
                        std::vector<int> row;
                        for (const auto& [r, c] : shape) {
                            if (top + r >= height || left + c >= width || index[(top + r) * width + left + c] < 0)
                                break;
                            row.push_back(index[(top + r) * width + left + c]);
                        }
                        if (row.size() == shape.size()) {
                            row.push_back(cells + kind);
                            dlx.add_row(row);
                        }
                    }
                }
            }
        }
        found = dlx.count();
        for (std::size_t kind = 0; kind < copies.size() && found > 0; ++kind) {
            for (int k = 2; k <= copies[kind]; ++k)
                found *= static_cast<std::uint64_t>(k);
        }
    }
    std::cout << found << '\n';
}
