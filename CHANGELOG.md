# Changelog

All notable changes to Hoista are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Pre-releases carry a suffix: `0.1.0-beta.1` is the first beta of `0.1.0`. Betas are published on GitHub as pre-releases; each one replaces the last, until `0.1.0` itself is released.

## [0.1.0-beta.2] - 2026-11-13

Replaces `0.1.0-beta.1`, which was published without downloads because its release build failed on Linux and Windows.

### Changed

- This beta has no Windows builds. They return once Hoista's deployment tests pass on Windows.
- The release build frees disk space on the Linux runner before compiling.
- hoista.dev links straight to each installer, says which Mac chip a download is for, and offers the Intel build to Intel Macs.

### Known limitations

- Hoista Desktop is not yet signed with an Apple Developer ID or notarized, so macOS blocks it on first open.
- No Windows builds yet, of Desktop or the CLI.
- Providers connect with API tokens only; one-click sign-in with a provider is turned off for now.
- The beta update channel in Desktop's settings is not available yet. To move to a newer beta, install it from its release page.

## [0.1.0-beta.1] - 2026-11-13

The first public beta of Hoista: the open-source control plane for your infrastructure, built for AI agents.

### Added

- `hoistd`, the engine, and `hoist`, the CLI. The CLI starts the engine when a command needs it, and `hoist service install` starts it at login.
- Hoista Desktop for macOS (Apple Silicon and Intel) and Linux (AppImage and Debian package).
- Providers: Cloudflare, Fly.io, Railway, Vercel, Neon, Supabase, Render, Resend, Runpod, Modal and GitHub. Each is connected with an API token.
- Import of what already runs on your providers, grouped into projects.
- Deploys from a folder, with a plan you see first, live progress and rollback to any earlier deploy.
- Logs, metrics and daily usage as each provider reports them.
- Variables (hidden, or from a `.env` file), custom domains with DNS checks, and environments such as production and staging.
- Billing: what each provider has billed this period, and plan limits. Hoista never estimates cost.
- An MCP server for coding agents and chat apps, over stdio (`hoist mcp`) or HTTP on `127.0.0.1:8788`, with per-agent access (View only, Ask for approval, Full access) and a record of every call.
- Skills for agents, installed with `hoist skills install`.
- Remotes: run Hoista on Railway, Fly.io or Docker so agents can reach it while your computer is off, and move this computer's projects there in one step.
- Backups of Hoista's own state, an encrypted export for moving to another computer, and webhooks.
- Signed releases: every archive is signed with Sigstore from this repository's release workflow, and the installers refuse anything that doesn't verify.

### Known limitations

- This is a beta. Expect rough edges, and please [report them](https://github.com/TheDhejavu/hoista/issues).
- Hoista Desktop is not yet signed with an Apple Developer ID or notarized. macOS blocks it on first open; see [Install](https://docs.hoista.dev/getting-started/install#desktop-app) for how to open it.
- There is no Desktop app for Windows yet. The CLI and engine run on Windows.
- Providers connect with API tokens only; one-click sign-in with a provider is turned off for now.
- The beta update channel in Desktop's settings is not available yet. To move to a newer beta, install it from its release page.

[0.1.0-beta.2]: https://github.com/TheDhejavu/hoista/releases/tag/v0.1.0-beta.2
[0.1.0-beta.1]: https://github.com/TheDhejavu/hoista/releases/tag/v0.1.0-beta.1
