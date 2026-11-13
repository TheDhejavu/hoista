"""Collects the license text of every Rust crate a release is built from.

Writes THIRD-PARTY-NOTICES.txt (the texts) and DEPENDENCIES.json (what was
included, with checksums), and stops if any crate has no license text. Covers
what Cargo builds, build-time crates included; anything outside Cargo (system
libraries, for one) still needs checking by hand.

Run: python3 scripts/release/notices.py --target x86_64-unknown-linux-gnu --output DIR
package.py calls generate() itself when it packs a release.
"""

import argparse
import hashlib
import json
import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
LICENSES = ROOT / "scripts" / "release" / "licenses"

# Files and folders whose names start with these hold license or notice text.
NOTICE_PREFIXES = ("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT")
MAX_NOTICE_BYTES = 2 * 1024 * 1024
RULE = "=" * 72


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def cargo_metadata(target):
    cargo = os.environ.get("CARGO", "cargo")
    command = [
        cargo, "metadata", "--locked", "--offline",
        "--format-version", "1", "--filter-platform", target,
    ]
    return json.loads(subprocess.check_output(command, cwd=ROOT))


def built_crates(metadata):
    """Every crate our own crates depend on, directly or not, dev-only ones left out."""
    nodes = {node["id"]: node for node in metadata["resolve"]["nodes"]}
    ours = set(metadata["workspace_members"])
    seen = set()
    pending = list(ours)
    while pending:
        crate = pending.pop()
        if crate in seen:
            continue
        seen.add(crate)
        for dependency in nodes[crate]["deps"]:
            if any(kind["kind"] != "dev" for kind in dependency["dep_kinds"]):
                pending.append(dependency["pkg"])
    return seen - ours


def notice_files(package):
    folder = Path(package["manifest_path"]).parent.resolve()
    files = {
        path
        for path in folder.rglob("*")
        if path.is_file()
        and any(part.upper().startswith(NOTICE_PREFIXES) for part in path.relative_to(folder).parts)
    }
    if package.get("license_file"):
        files.add(folder / package["license_file"])
    return folder, sorted(files)


def read_notice(path, folder, crate):
    if path.is_symlink() or not path.resolve().is_relative_to(folder):
        raise SystemExit(f"{crate}: a license file points outside the crate.")
    data = path.read_bytes()
    if len(data) > MAX_NOTICE_BYTES:
        raise SystemExit(f"{crate}: a license file is too large to be one.")
    return data


def pinned_notice(pinned, crate):
    """A license text we keep ourselves, for a crate that ships none."""
    data = (LICENSES / pinned["file"]).read_bytes()
    if sha256(data) != pinned["sha256"]:
        raise SystemExit(f"{crate}: the kept license text doesn't match its checksum.")
    return data


def generate(target):
    """Returns the notices text and the DEPENDENCIES.json text for a target."""
    metadata = cargo_metadata(target)
    included = built_crates(metadata)
    pinned_by_crate = json.loads((LICENSES / "sources.json").read_text())

    sections = [
        "Hoista third-party notices",
        f"Target: {target}",
        "The crates Hoista is built from, build-time crates included.",
        "Each crate's license and notice files follow, with paths inside that crate.",
    ]
    packages = []
    missing = []

    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        if package["id"] not in included:
            continue
        crate = f'{package["name"]}@{package["version"]}'
        folder, files = notice_files(package)
        record = {key: package.get(key) for key in ("name", "version", "license", "source", "repository")}
        record["notices"] = []

        source = package.get("repository") or package.get("source")
        sections.append(f'\n{RULE}\n{crate}\nDeclared license: {package.get("license")}\nSource: {source}')

        pinned = pinned_by_crate.get(crate)
        if not files and pinned:
            text = pinned_notice(pinned, crate).decode("utf-8")
            sections.append(
                f'\nSelected license: {pinned["selected_license"]}\nUpstream: {pinned["source"]}\n\n{text}'
            )
            record["notices"].append(pinned)
        elif not files:
            missing.append(crate)

        for path in files:
            data = read_notice(path, folder, crate)
            relative = path.relative_to(folder).as_posix()
            sections.append(f'\n--- {relative} ---\n{data.decode("utf-8")}')
            record["notices"].append({"file": relative, "sha256": sha256(data)})
        packages.append(record)

    if missing:
        raise SystemExit(
            "No license text found for: "
            + ", ".join(missing)
            + ". Add a checked copy to scripts/release/licenses and list it in sources.json."
        )

    dependencies = {
        "format_version": 1,
        "target": target,
        "cargo_lock_sha256": sha256((ROOT / "Cargo.lock").read_bytes()),
        "scope": "The crates Cargo builds, build-time ones included; not system libraries",
        "packages": packages,
    }
    notices = "\n\n".join(sections) + "\n"
    return notices, json.dumps(dependencies, indent=2, sort_keys=True) + "\n"


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--target", required=True, help="Rust target triple")
    parser.add_argument("--output", required=True, type=Path, help="folder to write both files to")
    args = parser.parse_args()

    notices, dependencies = generate(args.target)
    args.output.mkdir(parents=True, exist_ok=True)
    (args.output / "THIRD-PARTY-NOTICES.txt").write_text(notices, encoding="utf-8")
    (args.output / "DEPENDENCIES.json").write_text(dependencies, encoding="utf-8")
    print(f"Wrote notices for {len(json.loads(dependencies)['packages'])} crates.")


if __name__ == "__main__":
    main()
