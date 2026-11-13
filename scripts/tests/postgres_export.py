#!/usr/bin/env python3
"""Tests database export and restore against a throwaway PostgreSQL server.

Starts a new cluster in a temporary folder, on a free port on 127.0.0.1 with
TLS, fills a database with a few awkward values (bytes, Unicode, a view, a
large object), then runs the ignored `native_postgres_archive` test against
it. It never connects to a server it didn't start, and stops and deletes the
cluster when done.

Needs the PostgreSQL programs (initdb, pg_ctl, pg_dump, ...) and openssl.

Run: python3 scripts/tests/postgres_export.py --pg-bin /path/to/postgres/bin [--work-dir DIR]
"""

import argparse
import os
import shutil
import socket
import subprocess
import tempfile
from pathlib import Path

REPOSITORY = Path(__file__).resolve().parents[2]
POSTGRES_PROGRAMS = ["initdb", "pg_ctl", "pg_dump", "pg_restore", "psql", "createdb"]
USER = "fixture"
PASSWORD = "fixture-password"
SOURCE_DATABASE = "hoista_source_fixture"
RESTORE_DATABASE = "hoista_restore_fixture"

SAMPLE_DATA = """
CREATE TABLE notes (id bigserial PRIMARY KEY, body text NOT NULL, payload bytea);
INSERT INTO notes (body, payload) VALUES
  ('Hello, Lagos', decode('00', 'hex')),
  ('Unicode: àṣẹ', decode('010203ff', 'hex'));
CREATE VIEW note_count AS SELECT count(*) AS count FROM notes;
CREATE TABLE objects (oid oid);
INSERT INTO objects SELECT lo_from_bytea(0, decode('aabbccdd', 'hex'));
"""


def free_port():
    with socket.socket() as probe:
        probe.bind(("127.0.0.1", 0))
        return probe.getsockname()[1]


class Cluster:
    """A PostgreSQL cluster of our own, in a temporary folder."""

    def __init__(self, programs, folder):
        self.programs = programs
        self.folder = folder
        self.data = folder / "cluster"
        self.log = folder / "server.log"
        self.port = free_port()
        # Our own settings only: nothing from PG* variables the caller has set.
        self.env = {key: value for key, value in os.environ.items() if not key.startswith("PG")}
        self.env["PATH"] = f"{programs}{os.pathsep}{self.env.get('PATH', '')}"

    def run(self, program, *arguments, input=None):
        command = [str(self.programs / program), *arguments]
        return subprocess.run(
            command, input=input, env=self.env, text=True, check=True, capture_output=True, timeout=60
        )

    def start(self):
        self.run("initdb", "-D", str(self.data), "-U", USER, "--auth=trust", "--no-locale", "-E", "UTF8")
        self.make_certificate()
        options = f"-h 127.0.0.1 -p {self.port} -c unix_socket_directories='' -c ssl=on"
        self.run("pg_ctl", "-D", str(self.data), "-l", str(self.log), "-w", "-t", "20", "-o", options, "start")
        self.env.update(
            PGHOST="127.0.0.1",
            PGPORT=str(self.port),
            PGUSER=USER,
            PGPASSWORD=PASSWORD,
            PGSSLMODE="require",
        )

    def make_certificate(self):
        key = self.data / "server.key"
        subprocess.run(
            [
                shutil.which("openssl") or "openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                "-days", "1", "-subj", "/CN=localhost",
                "-keyout", str(key), "-out", str(self.data / "server.crt"),
            ],
            check=True, capture_output=True, timeout=60,
        )
        key.chmod(0o600)

    def fill(self):
        for database in (SOURCE_DATABASE, RESTORE_DATABASE):
            self.run("createdb", "--no-password", database)
        self.env["PGDATABASE"] = SOURCE_DATABASE
        self.run("psql", "-X", "--no-password", "-v", "ON_ERROR_STOP=1", input=SAMPLE_DATA)

    def stop(self):
        if not (self.data / "postmaster.pid").exists():
            return
        stopped = subprocess.run(
            [str(self.programs / "pg_ctl"), "-D", str(self.data), "-m", "immediate", "-w", "-t", "20", "stop"],
            env=self.env, capture_output=True, timeout=25,
        )
        if stopped.returncode:
            raise SystemExit("Couldn't stop the test PostgreSQL server, so the test isn't finished.")

    def url(self):
        return f"postgresql://{USER}:{PASSWORD}@127.0.0.1:{self.port}/{SOURCE_DATABASE}?sslmode=require"


def run_export_test(cluster):
    env = dict(
        cluster.env,
        HOISTA_TEST_DATABASE_URL=cluster.url(),
        HOISTA_TEST_POSTGRES_BIN=str(cluster.programs),
        CARGO_INCREMENTAL="0",
        SQLX_OFFLINE="true",
    )
    command = [
        "cargo", "test", "-p", "hoista-server", "--test", "database_export",
        "native_postgres_archive", "--locked", "--", "--ignored",
    ]
    result = subprocess.run(command, cwd=REPOSITORY, env=env, timeout=180)
    if result.returncode:
        raise SystemExit(result.returncode)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--pg-bin", required=True, type=Path, help="folder with the PostgreSQL programs")
    parser.add_argument("--work-dir", type=Path, help="where to make the temporary cluster folder")
    args = parser.parse_args()

    programs = args.pg_bin.resolve(strict=True)
    for name in POSTGRES_PROGRAMS:
        if not (programs / name).is_file():
            raise SystemExit(f"{name} isn't in {programs}.")

    with tempfile.TemporaryDirectory(prefix="hoista-postgres-export-", dir=args.work_dir) as folder:
        cluster = Cluster(programs, Path(folder))
        try:
            cluster.start()
            cluster.fill()
            run_export_test(cluster)
        except subprocess.CalledProcessError as error:
            program = Path(error.cmd[0]).name
            if program == "pg_ctl" and cluster.log.exists():
                print(cluster.log.read_text()[-2000:])
            raise SystemExit(f"Setting up the test server failed at {program} (exit {error.returncode}).") from None
        finally:
            cluster.stop()

    print("PostgreSQL export and restore passed; the test server is stopped and deleted.")


if __name__ == "__main__":
    main()
