#include <algorithm>
#include <array>
#include <charconv>
#include <cstdint>
#include <iostream>
#include <iterator>
#include <optional>
#include <string>
#include <string_view>
#include <unordered_map>
#include <utility>
#include <vector>

namespace {

constexpr std::size_t stack_limit = 1'000'000;
constexpr std::size_t call_limit = 100'000;
constexpr std::size_t locals = 8;
constexpr std::int64_t memory_size = 1 << 20;

enum class Op : std::uint8_t {
    push, drop, dup, swap, over, add, sub, mul, div, mod, eq, ne, lt, le, gt, ge,
    jmp, jz, jnz, call, ret, get, set, load, store, read, print, halt,
};

enum class Operand : std::uint8_t { none, integer, local, label };

struct Mnemonic {
    std::string_view name;
    Op op;
    Operand operand;
};

constexpr std::array<Mnemonic, 28> mnemonics{{
    {"push", Op::push, Operand::integer}, {"drop", Op::drop, Operand::none},
    {"dup", Op::dup, Operand::none},      {"swap", Op::swap, Operand::none},
    {"over", Op::over, Operand::none},    {"add", Op::add, Operand::none},
    {"sub", Op::sub, Operand::none},      {"mul", Op::mul, Operand::none},
    {"div", Op::div, Operand::none},      {"mod", Op::mod, Operand::none},
    {"eq", Op::eq, Operand::none},        {"ne", Op::ne, Operand::none},
    {"lt", Op::lt, Operand::none},        {"le", Op::le, Operand::none},
    {"gt", Op::gt, Operand::none},        {"ge", Op::ge, Operand::none},
    {"jmp", Op::jmp, Operand::label},     {"jz", Op::jz, Operand::label},
    {"jnz", Op::jnz, Operand::label},     {"call", Op::call, Operand::label},
    {"ret", Op::ret, Operand::none},      {"get", Op::get, Operand::local},
    {"set", Op::set, Operand::local},     {"load", Op::load, Operand::none},
    {"store", Op::store, Operand::none},  {"read", Op::read, Operand::none},
    {"print", Op::print, Operand::none},  {"halt", Op::halt, Operand::none},
}};

// arg is the literal of push, the local of get and set, or the target index of a jump or
// call, which is -1 when the program never defines the label.
struct Instruction {
    Op op;
    std::size_t line;
    std::int64_t arg;
};

struct Error {
    std::string_view message;
    std::size_t line;
};

bool is_label(std::string_view name)
{
    auto letter = [](char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || c == '_'; };
    auto digit = [](char c) { return c >= '0' && c <= '9'; };
    return !name.empty() && letter(name[0]) &&
           std::ranges::all_of(name.substr(1), [&](char c) { return letter(c) || digit(c); });
}

// Parses -?[0-9]+ into a signed 64-bit value, failing on any other form or on overflow.
std::optional<std::int64_t> parse_integer(std::string_view token)
{
    std::string_view digits = token.starts_with('-') ? token.substr(1) : token;
    if (digits.empty() || !std::ranges::all_of(digits, [](char c) { return c >= '0' && c <= '9'; }))
        return std::nullopt;
    std::int64_t value{};
    auto [end, error] = std::from_chars(token.data(), token.data() + token.size(), value);
    if (error != std::errc{} || end != token.data() + token.size())
        return std::nullopt;
    return value;
}

// Splits text into the runs of bytes between spaces and tabs.
std::vector<std::string_view> tokens_of(std::string_view text)
{
    std::vector<std::string_view> tokens;
    std::size_t at = 0;
    while ((at = text.find_first_not_of(" \t", at)) != std::string_view::npos) {
        std::size_t end = std::min(text.find_first_of(" \t", at), text.size());
        tokens.push_back(text.substr(at, end - at));
        at = end;
    }
    return tokens;
}

std::optional<Mnemonic> lookup(std::string_view token)
{
    std::string lower(token);
    std::ranges::transform(lower, lower.begin(), [](char c) {
        return c >= 'A' && c <= 'Z' ? static_cast<char>(c - 'A' + 'a') : c;
    });
    auto found = std::ranges::find(mnemonics, std::string_view(lower), &Mnemonic::name);
    if (found == mnemonics.end())
        return std::nullopt;
    return *found;
}

// Assembles the source lines. Label operands resolve after the last line, so a jump may name
// a label defined later.
std::pair<std::vector<Instruction>, std::optional<Error>> assemble(const std::vector<std::string_view>& source)
{
    std::vector<Instruction> code;
    std::vector<std::pair<std::size_t, std::string_view>> targets;
    std::unordered_map<std::string_view, std::int64_t> labels;
    for (std::size_t index = 0; index < source.size(); ++index) {
        const std::size_t line = index + 1;
        auto fail = [&](std::string_view message) { return std::pair{code, std::optional{Error{message, line}}}; };
        std::string_view text = source[index].substr(0, source[index].find(';'));
        std::vector<std::string_view> tokens = tokens_of(text);
        std::size_t at = 0;
        for (; at < tokens.size() && tokens[at].ends_with(':'); ++at) {
            std::string_view name = tokens[at].substr(0, tokens[at].size() - 1);
            if (!is_label(name))
                return fail("bad label");
            if (!labels.emplace(name, static_cast<std::int64_t>(code.size())).second)
                return fail("duplicate label");
        }
        if (at == tokens.size())
            continue;
        std::optional<Mnemonic> mnemonic = lookup(tokens[at]);
        if (!mnemonic)
            return fail("unknown instruction");
        const std::size_t operands = tokens.size() - at - 1;
        Instruction in{mnemonic->op, line, 0};
        bool valid = operands == (mnemonic->operand == Operand::none ? 0 : 1);
        if (valid && mnemonic->operand != Operand::none) {
            std::string_view operand = tokens[at + 1];
            if (mnemonic->operand == Operand::label) {
                valid = is_label(operand);
                targets.emplace_back(code.size(), operand);
            } else {
                std::optional<std::int64_t> value = parse_integer(operand);
                valid = value.has_value() &&
                        (mnemonic->operand == Operand::integer ||
                         (*value >= 0 && *value < static_cast<std::int64_t>(locals)));
                in.arg = value.value_or(0);
            }
        }
        if (!valid)
            return fail("bad operand");
        code.push_back(in);
    }
    for (auto [index, name] : targets) {
        auto found = labels.find(name);
        code[index].arg = found == labels.end() ? -1 : found->second;
    }
    // Running off the end halts, and a label after the last instruction points here.
    code.push_back({Op::halt, 0, 0});
    return {std::move(code), std::nullopt};
}

void write_int(std::string& out, std::int64_t value)
{
    std::array<char, 24> digits;
    auto end = std::to_chars(digits.data(), digits.data() + digits.size(), value).ptr;
    out.append(digits.data(), end);
    out += '\n';
}

// Runs the program, appending what it prints to out, until it halts or an instruction fails.
std::optional<Error> execute(const std::vector<Instruction>& code, const std::vector<std::int64_t>& data,
                             std::string& out)
{
    std::vector<std::int64_t> stack(stack_limit);
    std::vector<std::int64_t> frames(locals * (call_limit + 1));
    std::vector<std::int64_t> memory(memory_size);
    std::vector<std::size_t> calls;
    calls.reserve(call_limit);
    std::size_t pc = 0, sp = 0, base = 0, cursor = 0;
    for (;;) {
        const Instruction& in = code[pc++];
        auto fail = [&](std::string_view message) { return std::optional{Error{message, in.line}}; };
        switch (in.op) {
        case Op::push:
            if (sp == stack_limit)
                return fail("stack overflow");
            stack[sp++] = in.arg;
            break;
        case Op::drop:
            if (sp < 1)
                return fail("stack underflow");
            --sp;
            break;
        case Op::dup:
            if (sp < 1)
                return fail("stack underflow");
            if (sp == stack_limit)
                return fail("stack overflow");
            stack[sp] = stack[sp - 1];
            ++sp;
            break;
        case Op::swap:
            if (sp < 2)
                return fail("stack underflow");
            std::swap(stack[sp - 1], stack[sp - 2]);
            break;
        case Op::over:
            if (sp < 2)
                return fail("stack underflow");
            if (sp == stack_limit)
                return fail("stack overflow");
            stack[sp] = stack[sp - 2];
            ++sp;
            break;
        case Op::add:
        case Op::sub:
        case Op::mul:
        case Op::div:
        case Op::mod:
        case Op::eq:
        case Op::ne:
        case Op::lt:
        case Op::le:
        case Op::gt:
        case Op::ge: {
            if (sp < 2)
                return fail("stack underflow");
            const std::int64_t a = stack[sp - 2];
            const std::int64_t b = stack[sp - 1];
            const auto ua = static_cast<std::uint64_t>(a);
            const auto ub = static_cast<std::uint64_t>(b);
            std::int64_t r;
            switch (in.op) {
            case Op::add: r = static_cast<std::int64_t>(ua + ub); break;
            case Op::sub: r = static_cast<std::int64_t>(ua - ub); break;
            case Op::mul: r = static_cast<std::int64_t>(ua * ub); break;
            case Op::div:
            case Op::mod:
                if (b == 0)
                    return fail("division by zero");
                // INT64_MIN / -1 overflows in C++, and the wrapped results are -a and 0.
                if (b == -1)
                    r = in.op == Op::div ? static_cast<std::int64_t>(0 - ua) : 0;
                else
                    r = in.op == Op::div ? a / b : a % b;
                break;
            case Op::eq: r = a == b; break;
            case Op::ne: r = a != b; break;
            case Op::lt: r = a < b; break;
            case Op::le: r = a <= b; break;
            case Op::gt: r = a > b; break;
            default: r = a >= b; break;
            }
            stack[sp - 2] = r;
            --sp;
            break;
        }
        case Op::jmp:
            if (in.arg < 0)
                return fail("undefined label");
            pc = static_cast<std::size_t>(in.arg);
            break;
        case Op::jz:
        case Op::jnz:
            if (sp < 1)
                return fail("stack underflow");
            if ((stack[--sp] == 0) == (in.op == Op::jz)) {
                if (in.arg < 0)
                    return fail("undefined label");
                pc = static_cast<std::size_t>(in.arg);
            }
            break;
        case Op::call:
            if (in.arg < 0)
                return fail("undefined label");
            if (calls.size() == call_limit)
                return fail("call stack overflow");
            calls.push_back(pc);
            base += locals;
            std::fill_n(frames.begin() + static_cast<std::ptrdiff_t>(base), locals, 0);
            pc = static_cast<std::size_t>(in.arg);
            break;
        case Op::ret:
            if (calls.empty())
                return fail("return without call");
            pc = calls.back();
            calls.pop_back();
            base -= locals;
            break;
        case Op::get:
            if (sp == stack_limit)
                return fail("stack overflow");
            stack[sp++] = frames[base + static_cast<std::size_t>(in.arg)];
            break;
        case Op::set:
            if (sp < 1)
                return fail("stack underflow");
            frames[base + static_cast<std::size_t>(in.arg)] = stack[--sp];
            break;
        case Op::load:
            if (sp < 1)
                return fail("stack underflow");
            if (stack[sp - 1] < 0 || stack[sp - 1] >= memory_size)
                return fail("bad address");
            stack[sp - 1] = memory[static_cast<std::size_t>(stack[sp - 1])];
            break;
        case Op::store:
            if (sp < 2)
                return fail("stack underflow");
            if (stack[sp - 1] < 0 || stack[sp - 1] >= memory_size)
                return fail("bad address");
            memory[static_cast<std::size_t>(stack[sp - 1])] = stack[sp - 2];
            sp -= 2;
            break;
        case Op::read:
            if (cursor == data.size())
                return fail("end of input");
            if (sp == stack_limit)
                return fail("stack overflow");
            stack[sp++] = data[cursor++];
            break;
        case Op::print:
            if (sp < 1)
                return fail("stack underflow");
            write_int(out, stack[--sp]);
            break;
        case Op::halt:
            return std::nullopt;
        }
    }
}

}  // namespace

int main()
{
    const std::string input{std::istreambuf_iterator<char>(std::cin), std::istreambuf_iterator<char>()};
    std::vector<std::string_view> lines;
    for (std::size_t at = 0; at < input.size();) {
        std::size_t end = std::min(input.find('\n', at), input.size());
        lines.push_back(std::string_view(input).substr(at, end - at));
        at = end + 1;
    }
    // The source ends at the first line that is exactly ".input", and the data follows it.
    auto separator = std::ranges::find(lines, std::string_view(".input"));
    std::vector<std::string_view> source(lines.begin(), separator);
    std::vector<std::int64_t> data;
    if (separator != lines.end()) {
        for (auto line = separator + 1; line != lines.end(); ++line) {
            for (std::string_view token : tokens_of(*line))
                data.push_back(parse_integer(token).value_or(0));
        }
    }

    std::string out;
    auto [code, error] = assemble(source);
    if (!error)
        error = execute(code, data, out);
    if (error) {
        out += "error: ";
        out += error->message;
        out += " at line " + std::to_string(error->line) + "\n";
    }
    std::cout << out;
}
