"""Small Rust lexer for structural guards (not a Rust type checker)."""

import re

TOKEN = re.compile(r"r#\w+|[A-Za-z_]\w*|::|[^\s]")
RAW = re.compile(r'(?:br|cr|r)(#*)"')
QUOTED = re.compile(r"""(?:b|c)?"(?:\\.|[^"\\])*"|b?'(?:\\.|[^'\\])' """, re.X | re.S)


def comment_end(source, start):
    """Skip nested Rust block comments."""
    depth = 1
    cursor = start + 2
    while depth and cursor < len(source):
        pair = source[cursor : cursor + 2]
        if pair in ("/*", "*/"):
            depth += 1 if pair == "/*" else -1
            cursor += 2
        else:
            cursor += 1
    return cursor


def ignored_end(source, cursor):
    """Return the end of a comment or literal, preserving lifetime tokens."""
    if source.startswith("//", cursor):
        end = source.find("\n", cursor)
        return len(source) if end < 0 else end
    if source.startswith("/*", cursor):
        return comment_end(source, cursor)
    raw = RAW.match(source, cursor)
    if raw:
        delimiter = '"' + raw[1]
        end = source.find(delimiter, raw.end())
        return len(source) if end < 0 else end + len(delimiter)
    quoted = QUOTED.match(source, cursor)
    return quoted.end() if quoted else None


def tokens(source):
    """Yield (token, source offset), excluding comments and strings."""
    cursor = 0
    while cursor < len(source):
        end = ignored_end(source, cursor)
        if end is not None:
            cursor = end
            continue
        match = TOKEN.match(source, cursor)
        if match:
            yield match[0].removeprefix("r#"), cursor
            cursor = match.end()
        else:
            cursor += 1


def path_segments(items, start):
    """Collect the a::b::c segments of a path starting right after `start`."""
    segments = []
    index = start + 1
    while index + 1 < len(items) and items[index][0] == "::":
        segments.append(items[index + 1][0])
        index += 2
    return segments


def completed_path(current, base):
    """A path grown past its group prefix; a bare prefix is not a path."""
    if current != base:
        yield current


def use_paths(items, prefix=()):
    """Expand nested use groups; aliases name bindings, not dependencies."""
    base = list(prefix)
    current = list(prefix)
    for token in items:
        if token in ("}", ","):
            # A trailing comma before `}` leaves `current` at the bare prefix.
            yield from completed_path(current, base)
            if token == "}":
                return
            current = list(prefix)
        elif token == "{":
            yield from use_paths(items, tuple(current))
            current = list(prefix)
        elif token == "as":
            next(items, None)
        elif token != "::":
            current.append(token)
    yield from completed_path(current, base)
