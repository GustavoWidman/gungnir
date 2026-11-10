"""What an agent saw while it worked: the shell commands of the counted attempt, and the
compiler, type-checker and runtime errors in their output."""

import json
import re
from collections import Counter
from pathlib import Path

# Bun's parser puts the location of an error on the next line, four spaces in; the stack of an
# uncaught exception, which Bun also prints after "error:", sits six spaces in.
BUN_PARSE = r"^error: (?P<msg>.*)\n {4}at [^\s(]+\.[mc]?[jt]sx?:\d+:\d+$"

# A diagnostic that a compiler, type checker or parser reports before the program runs.
STATIC = {
    "rust": [r"^error\[(?P<key>E\d{4})\]: (?P<msg>.*)$"],
    "zig": [r"\.zig:\d+:\d+: error: (?P<msg>.*)$"],
    "go": [r"^\S+\.go:\d+:\d+: (?P<msg>(?!.*panic).*)$"],
    "c": [r"\.[ch]:\d+:\d+: error: (?P<msg>.*)$"],
    "cpp": [r"\.(?:cpp|cc|cxx|hpp|hh|h):\d+:\d+: error: (?P<msg>.*)$"],
    "typescript": [r"error (?P<key>TS\d+): (?P<msg>.*)$", BUN_PARSE],
    "javascript": [BUN_PARSE],
    "python": [r"^\s*(?P<key>SyntaxError|IndentationError|TabError): (?P<msg>.*)$"],
}

# A failure of the running program in the agent's own test runs.
RUNTIME = {
    "rust": r"panicked at|thread '[^']*' panicked|stack overflow",
    "zig": r"^panic: |thread \d+ panic|reached unreachable code|integer overflow",
    "go": r"^panic: |goroutine \d+ \[running\]|^fatal error: ",
    "c": r"Segmentation fault|core dumped|Aborted|AddressSanitizer|stack smashing|double free",
    "cpp": r"Segmentation fault|core dumped|Aborted|AddressSanitizer|terminate called|stack smashing",
    "typescript": r"^\s*(TypeError|ReferenceError|RangeError|SyntaxError):|Maximum call stack",
    "javascript": r"^\s*(TypeError|ReferenceError|RangeError|SyntaxError):|Maximum call stack",
    "python": r"Traceback \(most recent call last\)",
}

STATIC_RE = {lang: [re.compile(p, re.M) for p in ps] for lang, ps in STATIC.items()}
RUNTIME_RE = {lang: re.compile(p, re.M) for lang, p in RUNTIME.items()}

# Rust's ownership and borrowing errors, the part of its compiler a crutch would lean on.
BORROW = {"E0373", "E0382", "E0384", "E0499", "E0500", "E0501", "E0502", "E0503", "E0505",
          "E0506", "E0507", "E0515", "E0521", "E0596", "E0597", "E0716"}
STRICT_NULL = {"TS2532", "TS2531", "TS18047", "TS18048", "TS2538"}


def zig_kind(msg):
    if re.search(r"has no member named|no field or member function named|"
                 r"dependency on libc must be explicitly", msg):
        return "api"
    if re.search(r"whitespace on one side|never mutated|unused (local|function)|"
                 r"pointless discard|capture shadows|shadows local", msg):
        return "rules"
    if re.search(r"^expected type|expected error union|cannot be coerced|"
                 r"signed integers must use|expected .* type, found|incompatible types", msg):
        return "types"
    if re.search(r"^expected '?[\w ]+'?, found", msg):
        return "syntax"
    return "other"


def kind(lang, key, msg):
    """Sort one diagnostic into a coarse kind that the paper reports."""
    if lang == "zig":
        return zig_kind(msg)
    if lang == "rust":
        return "borrow" if key in BORROW else "other"
    if lang == "typescript":
        if key in STRICT_NULL or (key == "TS2322" and "undefined" in msg):
            return "null"
        return "other"
    return "other"


def codex_commands(path):
    for line in path.open():
        event = json.loads(line)
        item = event.get("item") or {}
        if event.get("type") == "item.completed" and item.get("type") == "command_execution":
            yield item.get("aggregated_output") or ""


def claude_commands(path):
    pending = set()
    for line in path.open():
        event = json.loads(line)
        message = event.get("message")
        content = message.get("content") if isinstance(message, dict) else None
        if not isinstance(content, list):
            continue
        if event.get("type") == "assistant":
            pending.update(c["id"] for c in content
                           if c.get("type") == "tool_use" and c.get("name") == "Bash")
        elif event.get("type") == "user":
            for c in content:
                if c.get("type") == "tool_result" and c.get("tool_use_id") in pending:
                    pending.discard(c["tool_use_id"])
                    body = c.get("content")
                    if isinstance(body, list):
                        body = "\n".join(x.get("text", "") for x in body if isinstance(x, dict))
                    yield body or ""


def session(trial_dir, harness, lang):
    """Counts for the counted attempt of one trial."""
    attempt = json.loads((trial_dir / "run" / "record.json").read_text())["attempt"]
    path = trial_dir / "attempts" / str(attempt) / "transcript.jsonl"
    outputs = codex_commands(path) if harness == "codex" else claude_commands(path)
    counts = Counter()
    for output in outputs:
        counts["commands"] += 1
        found = {(m.groupdict().get("key"), m.group("msg").strip()[:120])
                 for pattern in STATIC_RE[lang] for m in pattern.finditer(output)}
        if found:
            counts["failed_builds"] += 1
            counts["diagnostics"] += len(found)
            for key, msg in found:
                counts["diag_" + kind(lang, key, msg)] += 1
        elif RUNTIME_RE[lang].search(output):
            counts["runtime_failures"] += 1
    return counts


def sessions(runs, trials):
    """One row of session counts per trial in `trials` (a report's trials table)."""
    rows = []
    for t in trials.itertuples():
        trial_dir = Path(runs) / t.experiment / "trials" / t.system / t.scenario / t.language / f"r{t.rep}"
        counts = session(trial_dir, t.harness, t.language)
        rows.append({"system": t.system, "scenario": t.scenario, "language": t.language, "rep": t.rep}
                    | {k: counts.get(k, 0) for k in
                       ("commands", "failed_builds", "diagnostics", "runtime_failures",
                        "diag_api", "diag_rules", "diag_types", "diag_syntax", "diag_borrow",
                        "diag_null", "diag_other")})
    return rows
