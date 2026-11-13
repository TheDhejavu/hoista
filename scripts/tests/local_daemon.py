#!/usr/bin/env python3
"""Runs the built hoist and hoistd together against a throwaway state folder.

Checks the parts unit tests can't: the hidden token prompt on a real
terminal, the engine's socket and its permissions, shutting down and
restarting (including when the client that asked goes away), several
`hoist start`s at once settling on one engine, the import and link commands
end to end, and an agent starting Hoista from cold while agents are paused.

Uses no provider accounts and never touches the keychain: the only account is
written straight into the database as disconnected.

Run after `cargo build`: python3 scripts/tests/local_daemon.py [--bin-dir target/debug]
"""

import argparse
import http.client
import json
import os
import select
import signal
import socket
import sqlite3
import stat
import subprocess
import sys
import tempfile
import time
from pathlib import Path

TIMEOUT = 10
FAKE_TOKEN = b"hidden-fixture"
BACKSPACE = b"\x7f"


def wait_until(condition, what, timeout=TIMEOUT):
    deadline = time.monotonic() + timeout
    while not condition():
        if time.monotonic() > deadline:
            raise TimeoutError(what)
        time.sleep(0.05)


class Hoista:
    """The built programs, pointed at one state folder."""

    def __init__(self, programs, state):
        self.programs = programs
        self.state = state
        self.socket = state / "hoistd.sock"

    def command(self, *arguments):
        return [str(self.programs / "hoist"), *arguments, "--state-dir", str(self.state)]

    def cli(self, *arguments):
        """Runs a hoist command and returns its JSON answer."""
        result = subprocess.run(
            self.command(*arguments, "--json", "--no-input"),
            check=True, capture_output=True, text=True, timeout=TIMEOUT,
        )
        return json.loads(result.stdout)

    def api(self, method, path, body=None, expected=200):
        """Calls the engine's HTTP API over its socket."""
        connection = http.client.HTTPConnection("localhost", timeout=5)
        connection.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        connection.sock.settimeout(5)
        connection.sock.connect(str(self.socket))
        try:
            payload = json.dumps(body) if body is not None else None
            connection.request(method, path, body=payload, headers={"Content-Type": "application/json"})
            response = connection.getresponse()
            assert response.status == expected, f"{method} {path}: {response.status}"
            return json.loads(response.read(1024 * 1024))
        finally:
            connection.close()

    def running_pid(self):
        """The engine's process id, or None when nothing answers."""
        try:
            return self.api("GET", "/health")["pid"]
        except (OSError, AssertionError, http.client.HTTPException):
            return None


def stop_process(pid):
    os.kill(pid, signal.SIGTERM)

    def gone():
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return True
        return False

    wait_until(gone, "The engine didn't exit")


# ---------------------------------------------------------------------------
# Before an engine is running
# ---------------------------------------------------------------------------


def token_prompt_never_shows_the_token(hoista, _scratch):
    """Types a token at the real hidden prompt, then rubs it out.

    The empty token is refused before the engine, a provider or the keychain
    is touched.
    """
    import pty
    import termios

    pid, terminal = pty.fork()
    if pid == 0:
        os.execv(str(hoista.programs / "hoist"), hoista.command("accounts", "add", "fly", "--account", "fixture-org"))

    transcript = b""
    typed = False
    exited = False
    try:
        deadline = time.monotonic() + TIMEOUT
        while time.monotonic() < deadline:
            readable, _, _ = select.select([terminal], [], [], 0.1)
            if not readable:
                continue
            try:
                chunk = os.read(terminal, 65536)
            except OSError:
                break
            if not chunk:
                break
            transcript += chunk
            if b"Fly.io token (hidden):" in transcript and not typed:
                # The prompt is printed just before echo is switched off; wait for that.
                wait_until(lambda: not termios.tcgetattr(terminal)[3] & termios.ECHO, "Echo stayed on", timeout=2)
                os.write(terminal, FAKE_TOKEN + BACKSPACE * len(FAKE_TOKEN) + b"\n")
                typed = True

        found, status = os.waitpid(pid, os.WNOHANG)
        if not found:
            time.sleep(0.1)
            found, status = os.waitpid(pid, os.WNOHANG)
        assert found == pid, "The prompt didn't exit"
        exited = True
        assert os.waitstatus_to_exitcode(status) != 0
        assert typed, f"The token prompt never appeared. The terminal showed: {transcript!r}"
        assert FAKE_TOKEN not in transcript
        assert b"control characters" in transcript, repr(transcript)
    finally:
        os.close(terminal)
        if not exited:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)

    assert not hoista.socket.exists()


def status_does_not_start_the_engine(hoista, _scratch):
    status = subprocess.run(hoista.command("daemon", "status"), capture_output=True, timeout=TIMEOUT)
    assert status.returncode != 0, "Status should say no engine is running"
    assert not hoista.socket.exists(), "Status shouldn't start the engine"


def login_item_is_valid_on_macos(hoista, scratch):
    if sys.platform != "darwin":
        return
    preview = subprocess.run(
        hoista.command("service", "install", "--dry-run"), check=True, capture_output=True, timeout=TIMEOUT
    )
    plist = scratch / "service-preview.plist"
    plist.write_bytes(preview.stdout)
    subprocess.run(["/usr/bin/plutil", "-lint", str(plist)], check=True, capture_output=True, timeout=TIMEOUT)


# ---------------------------------------------------------------------------
# One engine, started by hand
# ---------------------------------------------------------------------------


def engine_answers_and_shuts_down(hoista, _scratch):
    engine = subprocess.Popen(
        [str(hoista.programs / "hoistd"), "--state-dir", str(hoista.state)],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    try:

        def listening():
            if engine.poll() is not None:
                raise RuntimeError("The engine exited before it opened its socket")
            return hoista.socket.exists()

        wait_until(listening, "The engine didn't open its socket")

        assert hoista.cli("profile", "set", "--name", "IPC Fixture", "--initials", "IF")["name"] == "IPC Fixture"
        assert hoista.cli("profile")["initials"] == "IF"
        assert hoista.cli("projects") == []
        assert hoista.cli("daemon", "status")["pid"] == engine.pid
        assert stat.S_IMODE(hoista.state.stat().st_mode) == 0o700
        assert stat.S_IMODE(hoista.socket.stat().st_mode) == 0o600

        engine.send_signal(signal.SIGTERM)
        _, stderr = engine.communicate(timeout=TIMEOUT)
        assert engine.returncode == 0, stderr.decode(errors="replace")
    finally:
        if engine.poll() is None:
            engine.kill()
            engine.communicate(timeout=5)

    # The CLI can open the state on its own once the engine has stopped cleanly.
    assert hoista.cli("profile")["name"] == "IPC Fixture"


def add_disconnected_account(hoista, _scratch):
    """One disconnected Cloudflare account with three resources, written while
    no engine is running. Finding resources at a provider has its own tests."""
    account = {
        "id": "fixture-account",
        "provider": "cloudflare",
        "label": "Fixture",
        "external_id": "fixture-remote",
        "scopes": [],
        "status": "disconnected",
        "connected_at": "2026-11-02T00:00:00Z",
    }
    with sqlite3.connect(hoista.state / "hoista.db") as database:
        database.execute(
            "INSERT INTO accounts(id, provider, data) VALUES (?, ?, ?)",
            (account["id"], account["provider"], json.dumps(account)),
        )
        for letter in "abc":
            resource = {
                "id": f"fixture-{letter}",
                "project_id": None,
                "kind": "web",
                "provider": "cloudflare",
                "account_id": account["id"],
                "provider_ref": f"worker:fixture-{letter}",
                "name": f"fixture-{letter}",
                "region": None,
                "status": "unknown",
                "mode": "watching",
                "url": None,
            }
            database.execute(
                "INSERT INTO resources(id, project_id, account_id, provider_ref, data) VALUES (?, NULL, ?, ?, ?)",
                (resource["id"], resource["account_id"], resource["provider_ref"], json.dumps(resource)),
            )


# ---------------------------------------------------------------------------
# The engine in the background, as `hoist start` runs it
# ---------------------------------------------------------------------------


def starts_at_once_share_one_engine(hoista, _scratch):
    starters = [
        subprocess.Popen(hoista.command("start", "--json"), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        for _ in range(3)
    ]
    try:
        pids = []
        for starter in starters:
            stdout, stderr = starter.communicate(timeout=20)
            assert starter.returncode == 0, stderr.decode(errors="replace")
            pids.append(json.loads(stdout)["pid"])
    finally:
        for starter in starters:
            if starter.poll() is None:
                starter.kill()
                starter.communicate(timeout=5)

    assert len(set(pids)) == 1, pids
    assert hoista.cli("start")["pid"] == pids[0]


def restart_brings_up_a_new_engine(hoista, _scratch):
    before = hoista.cli("daemon", "status")
    assert before["supervised"] is False
    after = hoista.cli("daemon", "restart")
    assert after["instance_id"] != before["instance_id"]
    assert not after["restart_requested"] and not after["supervised"]
    assert hoista.cli("profile")["initials"] == "IF"


def restart_finishes_after_the_client_leaves(hoista, _scratch):
    """The engine replaces itself even when the client that asked is gone."""
    current = hoista.api("GET", "/daemon")
    accepted = hoista.api("POST", "/daemon/restart", {"instance_id": current["instance_id"]}, expected=202)
    assert accepted["restart_requested"]

    def replaced():
        try:
            return hoista.api("GET", "/daemon")["instance_id"] != current["instance_id"]
        except (OSError, AssertionError, http.client.HTTPException):
            return False

    wait_until(replaced, "The engine didn't replace itself")


def import_and_link_work(hoista, scratch):
    assert hoista.cli("accounts")[0]["id"] == "fixture-account"
    config = hoista.cli("mcp", "config", "--name", "Fixture tool")
    assert config["mcpServers"]["hoista"]["args"][-1] == "Fixture tool"

    assert len(hoista.cli("import", "--list")) == 3
    imported = hoista.cli("import", "--new", "import-fixture", "fixture-a")
    project = imported["project"]
    assert project["source"] == {"kind": "unlinked"}
    assert project["mode"] == "watching"
    assert imported["resources"][0]["mode"] == "watching"

    added = hoista.cli("import", "--project", "import-fixture", "fixture-b")
    assert added["items"][0]["project_id"] == project["id"]
    managed = hoista.cli("import", "--new", "managed-fixture", "--manage", "fixture-c")
    assert managed["resources"][0]["mode"] == "managed"
    assert hoista.cli("import", "--list") == []

    folder = scratch / "source"
    folder.mkdir()
    (folder / "index.html").write_text("<!doctype html><title>Fixture</title>")
    linked = hoista.cli("link", "import-fixture", "--folder", str(folder))
    assert linked["source"] == {"kind": "folder", "path": str(folder.resolve())}
    assert linked["mode"] == "watching"
    assert hoista.cli("projects", "manage", "import-fixture")["mode"] == "managed"
    assert hoista.cli("projects", "watch", "import-fixture")["mode"] == "watching"


def commands_start_the_engine_when_needed(hoista, _scratch):
    stop_process(hoista.running_pid())
    assert len(hoista.cli("projects")) == 2
    assert hoista.running_pid() is not None


def a_paused_hoista_refuses_agents_with_a_reason(hoista, _scratch):
    """An agent starting Hoista from cold while agents are paused.

    Pausing first means no agent is enrolled, so nothing reaches the keychain.
    """
    assert hoista.cli("agents", "pause")["agents_enabled"] is False
    stop_process(hoista.running_pid())

    requests = [
        {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "fixture"}}},
        {"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "list_projects", "arguments": {}}},
    ]
    session = subprocess.run(
        hoista.command("mcp"),
        input="".join(json.dumps(request) + "\n" for request in requests),
        text=True,
        capture_output=True,
        timeout=20,
    )
    assert hoista.running_pid() is not None

    # The session starts, the tool call is refused at once with the reason,
    # and nothing but JSON-RPC is written to stdout.
    answers = [json.loads(line) for line in session.stdout.splitlines() if line.strip()]
    assert [answer.get("id") for answer in answers] == [1, 2], session.stdout
    assert "result" in answers[0]
    assert "paused" in json.dumps(answers[1]).lower(), answers[1]

    assert hoista.cli("agents", "resume")["agents_enabled"] is True
    assert hoista.cli("start")["pid"] == hoista.running_pid()


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--bin-dir", type=Path, default=Path("target/debug"), help="where hoist and hoistd are")
    args = parser.parse_args()
    if os.name != "posix":
        raise SystemExit("This test needs a Unix terminal and signals.")

    # In /tmp, so the socket's path stays under the system's length limit.
    with tempfile.TemporaryDirectory(prefix="hoista-ipc-", dir="/tmp") as folder:
        scratch = Path(folder)
        hoista = Hoista(args.bin_dir.resolve(), scratch / "state")

        steps = [
            token_prompt_never_shows_the_token,
            status_does_not_start_the_engine,
            login_item_is_valid_on_macos,
            engine_answers_and_shuts_down,
            add_disconnected_account,
            starts_at_once_share_one_engine,
            restart_brings_up_a_new_engine,
            restart_finishes_after_the_client_leaves,
            import_and_link_work,
            commands_start_the_engine_when_needed,
            a_paused_hoista_refuses_agents_with_a_reason,
        ]
        try:
            for step in steps:
                step(hoista, scratch)
                print(f"ok  {step.__name__.replace('_', ' ')}")
        finally:
            pid = hoista.running_pid()
            if pid is not None:
                try:
                    os.kill(pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass

    print("The engine, the CLI and a cold agent start all passed.")


if __name__ == "__main__":
    main()
