"""Writes the docs' Changelog page, apps/docs/changelog.mdx, from CHANGELOG.md.

CHANGELOG.md is the one place release notes are written. Run this after
adding a release to it, and commit both:

    python3 scripts/release/docs_changelog.py

With --check it changes nothing and fails when the page is out of date. The
docs check and the release workflow run it that way, so a release can't go
out without its entry in the docs.
"""

import sys
from datetime import date
from pathlib import Path

from changelog import releases

PAGE = Path("apps/docs/changelog.mdx")
RELEASES = "https://github.com/TheDhejavu/hoista/releases/tag"
DOCS = "https://docs.hoista.dev/"

HEADER = """---
title: "Changelog"
description: "What changed in each Hoista release."
rss: true
---

{/* Written by scripts/release/docs_changelog.py from CHANGELOG.md. Edit CHANGELOG.md, then run the script. */}

Each release is on [GitHub releases](https://github.com/TheDhejavu/hoista/releases), with signed downloads. Betas are published as pre-releases.
"""


def label(version):
    """0.1.0-beta.2 as "0.1.0 Beta 2"."""
    number, _, suffix = version.partition("-")
    if not suffix:
        return number
    name, _, count = suffix.partition(".")
    return f"{number} {name.capitalize()} {count}".rstrip()


def entry(version, released, body):
    day = date.fromisoformat(released)
    # Section headings would crowd the page's table of contents.
    lines = [f"**{line[4:]}**" if line.startswith("### ") else line for line in body.splitlines()]
    text = "\n".join(lines).replace(DOCS, "/")
    tags = '["Beta"]' if "-beta" in version else "[]"
    return (
        f'<Update label="{label(version)}" description="v{version} · {day.day} {day:%B %Y}" tags={{{tags}}}>\n'
        f"{text}\n\n"
        f"[v{version} on GitHub]({RELEASES}/v{version})\n"
        "</Update>\n"
    )


def page():
    return HEADER + "".join("\n" + entry(*release) for release in releases())


def main():
    wanted = page()
    if sys.argv[1:] == ["--check"]:
        if not PAGE.exists() or PAGE.read_text(encoding="utf-8") != wanted:
            raise SystemExit(f"{PAGE} is out of date: run python3 scripts/release/docs_changelog.py")
        return
    if sys.argv[1:]:
        raise SystemExit("Run with no arguments to write the page, or --check to check it")
    PAGE.write_text(wanted, encoding="utf-8")
    print(f"Wrote {PAGE}")


if __name__ == "__main__":
    main()
