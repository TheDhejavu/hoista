"""Packs a release build into release/hoista-<target>.tar.gz, with a .sha256 beside it.

The archive holds hoist, hoistd, our LICENSE and the third-party notices.
install.sh expects exactly these files. Run after `cargo build --release
--target <target>`, with RELEASE_TARGET set to that target:

    RELEASE_TARGET=x86_64-unknown-linux-gnu python3 scripts/release/package.py
"""

import hashlib
import io
import os
import re
import tarfile
from pathlib import Path

from notices import generate

PROGRAMS = ("hoist", "hoistd")
OUTPUT = Path("release")


def main():
    target = os.environ["RELEASE_TARGET"]
    if not re.fullmatch(r"[A-Za-z0-9_-]+", target):
        raise SystemExit(f"Not a target name: {target}")
    extension = ".exe" if "windows" in target else ""

    notices, dependencies = generate(target)
    documents = {
        "LICENSE": Path("LICENSE").read_bytes(),
        "THIRD-PARTY-NOTICES.txt": notices.encode("utf-8"),
        "DEPENDENCIES.json": dependencies.encode("utf-8"),
    }

    OUTPUT.mkdir(exist_ok=True)
    archive = OUTPUT / f"hoista-{target}.tar.gz"
    with tarfile.open(archive, "w:gz") as tar:
        for name in PROGRAMS:
            program = Path("target") / target / "release" / f"{name}{extension}"
            tar.add(program, arcname=program.name, recursive=False)
        for name, data in documents.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            info.mode = 0o644
            tar.addfile(info, io.BytesIO(data))

    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(f"{archive.name}.sha256").write_text(f"{digest}  {archive.name}\n")
    print(f"Packed {archive}")


if __name__ == "__main__":
    main()
