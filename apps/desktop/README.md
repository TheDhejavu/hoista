# apps/desktop

The Hoista desktop app: the main window and the panel under the menu bar icon. It's built on [Tauri](https://tauri.app). The screens are React in the system webview (WebKit on macOS, WebView2 on Windows, WebKitGTK on Linux), and the Rust side handles everything around them.

It doesn't do any hosting work itself. It's a client of `hoistd`, like the CLI and coding agents.

## Layout

```
src/
  main.tsx      starts a window: asks Rust for what it needs, sets up window.hoista, renders
  ui/           the screens, components and their data hooks (React, TanStack Query)
  adapter/      shapes hoistd's API into the routes the screens use
  native/       the only code that calls Rust: requests, remotes, menus, notifications
  shared/       types for the screens' API, provider entries
src-tauri/src/
  lib.rs        plugins, state, windows and the list of commands
  commands/     the commands the page can call, one file per area; thin wrappers
  engine/       hoistd over its socket, the event stream, live log tails
  remotes/      your remotes: the list, keys in the keychain, sign-in, switching, moves
  loopback.rs   the one-off 127.0.0.1 address a browser sign-in comes back to
  window.rs     the main window; links open in the browser
  panel.rs      the panel under the menu bar icon
  tray.rs       the icon's picture and the Dock badge
  menu.rs       the app menu, the icon's menu and the right-click menu
```

## How a request travels

A screen asks for `GET /projects/summaries`. The adapter (`src/adapter`) turns that into the `hoistd` calls behind it and shapes the answer. Each of those calls is a Rust command, `engine_request`. Rust sends it to whichever Hoista is shown: this computer's `hoistd` over its socket, or a remote over HTTPS with this computer's key. The screens never know which one answered.

Rust follows the shown Hoista's live events and passes each one to every window as `engine-event`. The screens refresh from those, so nothing polls.

The `/remotes` routes are answered by Rust (`src/native/remotes.ts` → `remotes::Remotes`). Remote keys stay in Rust and the keychain and never reach the page.

## Run it

From the repository root, with the engine running:

```sh
npm install
npm run desktop
```

`scripts/dev.sh` does the same and also rebuilds and restarts the engine. `npm run demo` opens the app on the demo engine: Hoista's own engine on Sam Rivera's example projects, kept in `~/.hoista-demo`, with providers answered locally (`bin/hoista-demo`). `npm run demo -- --fresh` starts it over from the seed.

The first run compiles the Rust side, which takes a few minutes. `HOISTA_SOCKET=/some/dir/hoistd.sock` points the app at a test engine. `HOISTA_START_AT=welcome` opens it on a given screen.

## Checks

```sh
npm run desktop:check
cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

The first checks the page: formatting, lint, types, tests and a build. The second checks the Rust side.

## Rules the checks enforce

- No `node:` imports in the page. Anything that needs the computer is a Rust command.
- Provider names appear only in `src/shared/providers/` (`scripts/check-providers.mjs`). What a provider can do comes from the engine.
- A Rust command checks what it's given: engine paths stay on the engine (no host, no `..`), the window opens only its own screens, and links open in the browser only if they're http(s).

## Known gaps

- Clicking a notification shows the app but not the screen it's about. Tauri isn't told which notification was clicked.
- The app menu doesn't show keyboard shortcuts. The window answers them itself, and a menu shortcut would take the key first.
- Dropping a file on the window doesn't give its path, so the screens ask you to choose the file instead.
- Not yet: auto-updates (the Tauri updater, with its own signing key), signing and notarizing the macOS build, and Open at login.
