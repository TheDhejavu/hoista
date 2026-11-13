#!/usr/bin/env python3
"""Tests scripts/install/install.sh offline.

Stand-ins for curl, cosign and uname serve a fake release from a folder, so
nothing is downloaded, no real signature is checked and nothing is set to
start at login. Each case installs into a fresh folder whose name has a space
and a $ in it, to catch quoting mistakes.

Run: python3 scripts/tests/installer.py
"""

import io
import os
import subprocess
import tarfile
import tempfile
from pathlib import Path

INSTALLER = Path(__file__).resolve().parents[1] / "install" / "install.sh"
RELEASE_FILES = ["hoist", "hoistd", "LICENSE", "THIRD-PARTY-NOTICES.txt", "DEPENDENCIES.json"]

# The fake hoist and hoistd: each records its arguments, then exits with
# FAKE_SERVICE_STATUS so a case can make `hoist service install` fail.
FAKE_PROGRAM = b'#!/bin/sh\nprintf "%s\\n" "$*" >> "$FAKE_CALLS"\nexit "${FAKE_SERVICE_STATUS:-0}"\n'

FAKE_UNAME = '#!/bin/sh\ncase "$1" in -s) echo Linux;; -m) echo x86_64;; *) exit 1;; esac\n'
FAKE_COSIGN = '#!/bin/sh\nexit "${FAKE_SIGNATURE_STATUS:-0}"\n'
# Hands out the fake release, or an empty signature bundle for *.sigstore.json.
FAKE_CURL = """#!/bin/sh
while [ "$#" -gt 0 ]; do
  case "$1" in
    -o) output=$2; shift 2;;
    *.sigstore.json) signature=yes; shift;;
    *) shift;;
  esac
done
if [ "${signature:-no}" = yes ]; then printf '{}' > "$output"; else cp "$FAKE_RELEASE" "$output"; fi
"""


class Sandbox:
    def __init__(self, root):
        self.root = root
        self.release = root / "release.tar.gz"
        self.calls = root / "calls"
        tools = root / "tools"
        tools.mkdir()
        for name, source in [("uname", FAKE_UNAME), ("cosign", FAKE_COSIGN), ("curl", FAKE_CURL)]:
            (tools / name).write_text(source)
            (tools / name).chmod(0o755)
        self.env = dict(
            os.environ,
            PATH=f"{tools}{os.pathsep}{os.environ['PATH']}",
            FAKE_RELEASE=str(self.release),
            FAKE_CALLS=str(self.calls),
        )

    def pack_release(self, hoist_is_symlink=False):
        with tarfile.open(self.release, "w:gz") as archive:
            for name in RELEASE_FILES:
                info = tarfile.TarInfo(name)
                info.mode = 0o755
                if hoist_is_symlink and name == "hoist":
                    info.type = tarfile.SYMTYPE
                    info.linkname = "/does-not-exist"
                    archive.addfile(info)
                    continue
                content = FAKE_PROGRAM if name in ("hoist", "hoistd") else b"Notice\n"
                info.size = len(content)
                archive.addfile(info, io.BytesIO(content))

    def install(self, case, *options, **env):
        destination = self.root / f"{case} spaces $literal"
        result = subprocess.run(
            ["sh", str(INSTALLER), "v0.1.0", str(destination), *options],
            env={**self.env, **env},
            capture_output=True,
            text=True,
            timeout=10,
        )
        return destination, result

    def take_calls(self):
        """What the fake programs were asked to do since the last look."""
        if not self.calls.exists():
            return ""
        calls = self.calls.read_text()
        self.calls.unlink()
        return calls


def installs_and_starts_at_login(sandbox):
    destination, result = sandbox.install("default")
    assert result.returncode == 0, result.stderr
    assert (destination / "hoist").read_bytes() == FAKE_PROGRAM
    assert (destination / "hoistd").is_file()
    assert sandbox.take_calls() == "service install\n"
    assert "hoist accounts add" in result.stdout and "hoist mcp config" in result.stdout


def no_service_skips_login_startup(sandbox):
    _, result = sandbox.install("no-service", "--no-service")
    assert result.returncode == 0, result.stderr
    assert sandbox.take_calls() == ""
    assert "won’t start at login" in result.stdout


def says_so_when_login_startup_fails(sandbox):
    destination, result = sandbox.install("service-fails", FAKE_SERVICE_STATUS="1")
    assert result.returncode != 0
    assert (destination / "hoist").is_file(), "The programs stay installed"
    assert "could not be set to start at login" in result.stderr
    sandbox.take_calls()


def refuses_a_bad_signature(sandbox):
    destination, result = sandbox.install("bad-signature", FAKE_SIGNATURE_STATUS="1")
    assert result.returncode != 0
    assert not destination.exists()
    assert sandbox.take_calls() == ""


def refuses_an_unknown_option(sandbox):
    destination, result = sandbox.install("unknown-option", "--unrecognized")
    assert result.returncode != 0
    assert not destination.exists()


def refuses_a_symlink_in_the_release(sandbox):
    sandbox.pack_release(hoist_is_symlink=True)
    destination, result = sandbox.install("symlink")
    assert result.returncode != 0
    assert not destination.exists()
    assert "regular files" in result.stderr


CASES = [
    installs_and_starts_at_login,
    no_service_skips_login_startup,
    says_so_when_login_startup_fails,
    refuses_a_bad_signature,
    refuses_an_unknown_option,
    # Last: it replaces the release with a broken one.
    refuses_a_symlink_in_the_release,
]


def main():
    with tempfile.TemporaryDirectory(prefix="hoista-installer-") as directory:
        sandbox = Sandbox(Path(directory))
        sandbox.pack_release()
        for case in CASES:
            case(sandbox)
            print(f"ok  {case.__name__.replace('_', ' ')}")


if __name__ == "__main__":
    main()
