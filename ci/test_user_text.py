"""Rendered user copy carries no proof, check or certification wording.

The owner's rule (27 Nov 2026): a user never sees "checked", "not checked", "proven",
"proved", "certified" or any other proof wording or mark. This test reads every string
literal in the crates that produce on-screen text, native and browser alike, and fails on
one that uses that wording.

Text that never reaches the screen is left out by where it sits: comments, test modules,
identifiers and codes (a literal with no spaces that holds one of _ - . : / {), JSON keys
read by index, and the arguments of calls whose text is for agents, logs, files or
matching (see SKIPPED_CALLS). What remains and still reads as an identifier is listed in
user-text-allowlist.json with the reason it is not user copy.
"""

import collections
import functools
import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ALLOWLIST = ROOT / "ci" / "user-text-allowlist.json"

# Crates whose strings can reach the screen: the app itself, and the model, assembly,
# drawing, exchange and kernel crates whose messages the app shows as they are.
CRATES = [
    "soliddraft-app",
    "soliddraft-assembly",
    "soliddraft-drawing",
    "soliddraft-exchange",
    "soliddraft-kernel-api",
    "soliddraft-kernel-catallax",
    "soliddraft-model",
    "soliddraft-web",
]

BANNED = re.compile(
    r"certif|\bunprov|\bprov(?:e|ed|en|es|ing)\b|\bproofs?\b|\b(?:un)?verif|\b(?:un)?checked\b",
    re.IGNORECASE,
)

# Calls and macros whose string arguments are never drawn: agent and MCP descriptions and
# schemas, MCP and probe JSON, panics and assertions, logs, file writers, thread names,
# and matchers over text another layer wrote.
SKIPPED_CALLS = {
    "agents", "json",
    "number", "string", "boolean", "integer", "optional", "one_of", "array", "object",
    "non_empty_array", "any_of", "constant",
    "expect", "unreachable", "panic", "todo", "unimplemented",
    "assert", "assert_eq", "assert_ne", "debug_assert", "debug_assert_eq", "debug_assert_ne",
    "debug", "trace", "info", "warn", "eprintln", "println", "write", "writeln",
    "name",
    "contains", "starts_with", "ends_with", "says",
}
# Log macros share names with ordinary functions, so they count only as macros.
MACRO_ONLY = {"debug", "info", "warn", "error", "trace"}

# A cheap first look before BANNED, which is slow over whole files.
WORDS = re.compile(r"cert|prov|proof|verif|checked")
IDENTIFIER = re.compile(r"^\S*[_\-.:/{]\S*$")
WORD = re.compile(r"\w+")


def unescape(body):
    return re.sub(r"\\\n\s*", "", body)


TOKEN = re.compile(
    r"(?P<line>//[^\n]*)"
    r"|(?P<block>/\*)"
    r'|(?P<string>"(?:[^"\\]|\\.)*")'
    r"|(?P<char>'(?:\\.|\\u\{[0-9a-fA-F]+\}|[^\\'])')"
    # Code that holds no literal, comment or bracket: words, spaces and other punctuation.
    r'|(?P<plain>[^"\'/()\[\]{}!;#]+)'
    r"|(?P<other>.)",
    re.DOTALL,
)
TAIL = re.compile(r"\w+$")
# Words after which a [ opens an array, not an index.
KEYWORDS = {"in", "return", "break", "else", "match", "if", "while", "let", "mut", "move", "yield", "as"}
RAW = re.compile(r'(#*)"')


def raw_start(source, i):
    """Whether the r just before `i` opens a raw string: r"..." or br#"..."#."""
    if i < 1 or source[i - 1] != "r":
        return False
    before = source[i - 2] if i >= 2 else " "
    if before == "b":
        before = source[i - 3] if i >= 3 else " "
    return not (before.isalnum() or before == "_")


def literals(source):
    """Yields (offset, text, frames) for every string literal outside comments."""
    frames = []  # (kind, name, is_macro); kind "test" is a #[cfg(test)] item's body
    prev, last = "", ""  # the last two significant tokens, to name calls and spot indexing
    recent = collections.deque(maxlen=6)  # to spot #[cfg(test)]
    test_item = False
    i, n = 0, len(source)
    while i < n:
        if source[i] in '"#' and raw_start(source, i) and (r := RAW.match(source, i)):
            close = '"' + r.group(1)
            end = source.find(close, r.end())
            if not any(frame[0] == "test" for frame in frames):
                yield i, source[r.end():end], list(frames)
            i = end + len(close)
            prev, last = last, '"'
            recent.append('"')
            continue
        m = TOKEN.match(source, i)
        kind, token, i = m.lastgroup, m.group(0), m.end()
        if kind == "line":
            continue
        if kind == "plain":
            token = token.rstrip()
            if not token:
                continue
            word = TAIL.search(token)
            token = word.group(0) if word else token[-1]
        elif kind == "block":
            depth = 1
            while i < n and depth:
                if source.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif source.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
            continue
        elif kind == "string":
            text = unescape(token[1:-1])
            if not any(frame[0] == "test" for frame in frames):
                yield m.start(), text, list(frames)
            token = '"'
        elif token == "(":
            macro = last == "!"
            name = prev if macro else last
            frames.append(("call", name if WORD.fullmatch(name) else "", macro))
        elif token == "[":
            indexed = (WORD.fullmatch(last) and last not in KEYWORDS) or last in (")", "]")
            frames.append(("index" if indexed else "array", "", False))
        elif token == "{":
            frames.append(("test" if test_item else "block", "", False))
            test_item = False
        elif token in ")]}" and frames:
            frames.pop()
        elif token == ";":
            test_item = False
        recent.append(token)
        if tuple(recent) == ("#", "[", "cfg", "(", "test", ")"):
            test_item = True
        prev, last = last, token


def skipped(frames):
    for kind, name, macro in reversed(frames):
        if kind == "index":
            return True
        if kind == "call" and name in SKIPPED_CALLS and (macro or name not in MACRO_ONLY):
            return True
        if kind == "call" and macro and name in MACRO_ONLY:
            return True
    return False


@functools.cache
def findings():
    found = []
    for crate in CRATES:
        for path in sorted((ROOT / "crates" / crate / "src").rglob("*.rs")):
            if path.name == "tests.rs" or "tests" in path.relative_to(ROOT / "crates" / crate / "src").parts:
                continue
            source = path.read_text(encoding="utf-8")
            if not WORDS.search(source.lower()):
                continue
            for offset, text, frames in literals(source):
                if not WORDS.search(text.lower()) or not BANNED.search(text) or IDENTIFIER.match(text) or skipped(frames):
                    continue
                line = source.count("\n", 0, offset) + 1
                found.append((str(path.relative_to(ROOT)), line, " ".join(text.split())))
    return tuple(found)


class UserTextTest(unittest.TestCase):
    def test_no_proof_wording_in_user_copy(self):
        allowed = {(entry["file"], entry["text"]) for entry in json.loads(ALLOWLIST.read_text())}
        stray = [f"{file}:{line}: {text}" for file, line, text in findings() if (file, text) not in allowed]
        self.assertEqual(stray, [], "user copy says checked, proven or certified; reword it plainly")

    def test_allowlist_entries_are_still_needed(self):
        present = {(file, text) for file, _, text in findings()}
        entries = json.loads(ALLOWLIST.read_text())
        for entry in entries:
            self.assertTrue(entry.get("why"), f"{entry} gives no reason")
        stale = [entry for entry in entries if (entry["file"], entry["text"]) not in present]
        self.assertEqual(stale, [], "these allowlist entries match nothing; remove them")

    def test_the_scan_reads_literals_where_they_are(self):
        source = 'fn f() {\n    ui.label("Not checked here");\n    x.expect("checked above");\n' \
                 '    log::debug!("proof pending");\n    m["proof"];\n    // a proven comment\n}\n'
        found = [(source.count("\n", 0, at) + 1, text) for at, text, frames in literals(source) if not skipped(frames)]
        self.assertEqual(found, [(2, "Not checked here")])


if __name__ == "__main__":
    unittest.main()

