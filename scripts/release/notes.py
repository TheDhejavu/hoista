"""Prints the release notes for one tag: its section of CHANGELOG.md.

The release workflow publishes these as the GitHub release's description, so
a release never goes out without a changelog entry:

    python3 scripts/release/notes.py v0.1.0-beta.1 > notes.md
"""

import re
import sys
from pathlib import Path

CHANGELOG = Path("CHANGELOG.md")


def section(changelog, version):
    """The body under `## [version]`, up to the next version or the link list."""
    heading = re.compile(rf"^## \[{re.escape(version)}\][^\n]*\n", re.MULTILINE)
    start = heading.search(changelog)
    if start is None:
        return None
    end = re.compile(r"^(## \[|\[[^\]]+\]: )", re.MULTILINE).search(changelog, start.end())
    body = changelog[start.end() : end.start() if end else len(changelog)]
    return body.strip()


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r"v[0-9][A-Za-z0-9.-]*", sys.argv[1]):
        raise SystemExit("Give the release tag, for example v0.1.0-beta.1")
    version = sys.argv[1].removeprefix("v")
    notes = section(CHANGELOG.read_text(encoding="utf-8"), version)
    if not notes:
        raise SystemExit(f"CHANGELOG.md has no entry for {version}")
    print(notes)


if __name__ == "__main__":
    main()
