"""Reads CHANGELOG.md: one entry per release, newest first.

Each release is a `## [version] - date` heading and the text under it, up to
the next release or the link list at the end of the file.
"""

import re
from pathlib import Path

CHANGELOG = Path("CHANGELOG.md")

HEADING = re.compile(r"^## \[([^\]]+)\] - (\d{4}-\d{2}-\d{2})[^\n]*\n", re.MULTILINE)
END = re.compile(r"^\[[^\]]+\]: ", re.MULTILINE)


def releases(text=None):
    """Every release in the changelog as (version, date, body)."""
    if text is None:
        text = CHANGELOG.read_text(encoding="utf-8")
    headings = list(HEADING.finditer(text))
    links = END.search(text)
    found = []
    for index, heading in enumerate(headings):
        if index + 1 < len(headings):
            end = headings[index + 1].start()
        else:
            end = links.start() if links else len(text)
        found.append((heading.group(1), heading.group(2), text[heading.end() : end].strip()))
    return found
