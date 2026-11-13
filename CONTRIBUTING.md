# Contributing to Hoista

Issues and pull requests are welcome. Keep changes small, add tests for what you change, and run the [checks](#checks) before you open a pull request.

Report security problems privately through [GitHub security advisories](https://github.com/TheDhejavu/hoista/security/advisories/new), not in public issues.

## Setup

You need [Rust](https://rustup.rs) 1.97 (pinned in `rust-toolchain.toml`) and Node.js 22. On Linux the desktop app also needs the system webview's libraries: `libwebkit2gtk-4.1-dev`, `libayatana-appindicator3-dev` and `librsvg2-dev` on Debian and Ubuntu.

```sh
git clone https://github.com/TheDhejavu/hoista.git
cd hoista
npm install
scripts/dev.sh
```

`scripts/dev.sh` rebuilds `hoist` and `hoistd`, installs npm packages when the lockfile changed, starts the engine on the new build (or restarts a running one onto it), and opens the desktop app. Ctrl-C closes the app; the engine keeps running.

- `scripts/dev.sh --demo` (or `npm run demo`) opens the app on the demo engine: Hoista's real engine on Sam Rivera's example projects, in `~/.hoista-demo`, with providers answered locally so nothing reaches the internet. Add `--fresh` to start over from the seed.
- A development build is its own channel: it keeps state in `~/.hoista-dev`, its credentials under a separate Keychain name and MCP over HTTP on port 8789. It can run beside an installed release (`~/.hoista`, port 8788) without sharing anything. `HOISTA_CHANNEL=stable` or `dev` overrides the choice.
- `HOISTA_SOCKET=/some/dir/hoistd.sock scripts/dev.sh` runs against a separate state folder, so you don't touch `~/.hoista-dev`.
- `npm run docs:dev` serves the docs at http://localhost:3001, and `npm run webpage:dev` serves hoista.dev at http://localhost:3000.

## Project layout

### Crates

The workspace is split the way the engine is layered. Each crate depends only on the ones above it, so `hoista-domain` and `hoista-app` never use a database, an HTTP framework or a provider's API directly.

| Crate                                  | What it is                                                                                                                                            |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| [`hoista-domain`](crates/domain)       | Core types and rules: projects, resources, deploys, access. No I/O.                                                                                   |
| [`hoista-system`](crates/system)       | The operating system: running programs, finding them, Hoista's folders, reading files safely.                                                         |
| [`hoista-app`](crates/app)             | The use cases, written against traits in its `ports` module. `Engine` and the services it hands out (`engine.deploys()`, `engine.accounts()`, …).     |
| [`hoista-providers`](crates/providers) | One adapter per provider, implementing the provider port.                                                                                             |
| [`hoista-store`](crates/store)         | The other ports' implementations: SQLite through sqlx, the keychain and credential keeper, backups, sandboxed builds.                                 |
| [`hoista-server`](crates/server)       | HTTP API, MCP, the browser pages (askama templates in `templates/`), the client the CLI uses, and `wiring.rs`, which plugs the store into the engine. |
| [`hoist`](bin/hoist)                   | The CLI binary.                                                                                                                                       |
| [`hoistd`](bin/hoistd)                 | The engine daemon binary.                                                                                                                             |
| [`hoista-demo`](bin/hoista-demo)       | The engine on seeded example data, with providers answered locally, for `npm run demo`.                                                               |

Swapping SQLite for another store, or the keychain for another vault, means a new implementation of a port and one line in `wiring.rs`. `crates/server/tests/architecture.rs` keeps the layering honest.

### Everything else

- [`apps/desktop`](apps/desktop): the desktop app, on [Tauri](https://tauri.app): React screens in the system webview, with a Rust side for the windows, the menu bar icon and remotes.
- [`apps/docs`](apps/docs): the docs site (Mintlify), at docs.hoista.dev.
- [`apps/webpage`](apps/webpage): hoista.dev.
- [`skills`](skills): skills for coding agents.
- [`deploy/server`](deploy/server): Hoista as a remote, in Docker.
- [`docs/backend`](docs/backend): notes for people working on the engine.

## Checks

CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
npm run desktop:check
npm run docs:validate
```

The desktop app's Rust side is checked on its own: see [apps/desktop](apps/desktop/README.md#checks).

## Releases

Versions follow [Semantic Versioning](https://semver.org). A beta is a pre-release of the next version: `0.1.0-beta.1`, then `0.1.0-beta.2`, until `0.1.0` itself.

To release:

1. Set the version in `Cargo.toml` (the workspace and its `hoista-*` dependencies), `apps/desktop/package.json`, `apps/desktop/src-tauri/Cargo.toml` and `apps/desktop/src-tauri/tauri.conf.json`, and update both `Cargo.lock` files and `package-lock.json`.
2. Add the version's section to [CHANGELOG.md](CHANGELOG.md), then run `python3 scripts/release/docs_changelog.py` to add it to [the docs' Changelog page](apps/docs/changelog.mdx). The release workflow stops if either is missing it, and uses the CHANGELOG.md section as the release notes.
3. Point the install commands and download links at the new tag: README.md, `apps/webpage` (`index.html`, `main.js`, `llms.txt`) and `apps/docs/getting-started/install.mdx`. In `index.html`, also change the `?v=` on `styles.css` and `main.js`, so browsers load the new files.
4. Merge to `main`, then tag that commit and push the tag (or publish a pre-release for the new tag from the Releases page, which creates the tag):

   ```sh
   git tag -a v0.1.0-beta.1 -m "Hoista 0.1.0 Beta 1"
   git push origin v0.1.0-beta.1
   ```

The [release workflow](.github/workflows/release.yml) checks and builds the CLI, the engine and Desktop for every platform, signs each file with Sigstore, and opens a draft GitHub release. A tag with a suffix such as `-beta.1` becomes a pre-release, which GitHub keeps off `/releases/latest`. Check the draft's files and notes, then publish it.
