"""Prints the release notes for one tag: its section of CHANGELOG.md.

The release workflow publishes these as the GitHub release's description, so
a release never goes out without a changelog entry:

    python3 scripts/release/notes.py v0.1.0-beta.2 > notes.md
"""

import re
import sys

from changelog import releases


def main():
    if len(sys.argv) != 2 or not re.fullmatch(r"v[0-9][A-Za-z0-9.-]*", sys.argv[1]):
        raise SystemExit("Give the release tag, for example v0.1.0-beta.2")
    version = sys.argv[1].removeprefix("v")
    notes = next((body for found, _, body in releases() if found == version), None)
    if not notes:
        raise SystemExit(f"CHANGELOG.md has no entry for {version}")
    print(notes)


if __name__ == "__main__":
    main()
