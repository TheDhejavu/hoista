# The engine

For people working on Hoista's Rust side: the engine crates, the `hoistd` daemon and the `hoist` command line. How to use Hoista is in the docs site (`apps/docs`, published at https://docs.hoista.dev).

## The crates

| Crate or program | What it is |
| --- | --- |
| `crates/domain` | Data types and rules with no I/O: projects, resources, deploys, plans, secrets, the list of providers |
| `crates/system` | Talking to the operating system: running programs, finding them, the state folder, reading files safely |
| `crates/app` | What Hoista does, one folder per area (`accounts/`, `projects/`, `deploys/`, …), written against the traits in `ports/` |
| `crates/providers` | One adapter per provider: its API calls and its command-line tool |
| `crates/store` | What Hoista keeps on this computer: SQLite (`migrations/`), the keychain, backups, webhooks, sandboxed builds |
| `crates/server` | How the outside reaches Hoista: the HTTP API, the MCP server, the client the CLI uses, and server mode (`cloud/`) |
| `bin/hoistd` | The daemon. Builds the engine and serves it on a local socket, or over HTTPS on a server |
| `bin/hoist` | The command line. A client of `hoistd`, which it starts when a command needs it |

The desktop app (`apps/desktop`) and coding agents are clients of the same daemon.

Each crate only uses the ones before it:

`domain` → `system` → `app` → `providers` → `store` → `server` → `hoist`, `hoistd`

`app` is written against traits that `providers` and `store` implement, and `crates/server/src/wiring.rs` is the one place that plugs the real ones in. `crates/server/tests/architecture.rs` fails if that slips.

## Adding a provider

See [adding-a-provider.md](adding-a-provider.md). Every address a provider adapter may call is listed in [network-hosts.txt](network-hosts.txt), and a test keeps the two in step.

## The API types

The desktop app's types for the engine API, [api/types.ts](api/types.ts) and [api/schema.json](api/schema.json), are generated from the Rust types. A test fails when they're out of date. After changing a type, run:

```sh
cargo run -p hoist -- schema > docs/backend/api/schema.json
cargo run -p hoist -- schema --typescript > docs/backend/api/types.ts
```

## Running the checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`scripts/tests/` has slower checks CI also runs: `local_daemon.py` runs the built `hoist` and `hoistd` together, and `installer.py` runs the installer against a fake release.

## Conventions

- Every module has a `//!` line saying what it's for, and every public item a `///`. Comments say why, not what.
- Typed errors in the engine; `anyhow` only in the two programs. No `unwrap` outside tests.
- Secrets never reach logs, errors, `Debug` output or SQLite. Use the `Secret` type.
- Every provider call has a timeout, and every spawned task an owner that can stop it.
- Never edit a database migration that has shipped; add a new one.
- Tests use fixtures and no network. Tests against real provider accounts run only in the `live` workflow, started by hand.
