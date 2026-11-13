# apps/webpage

hoista.dev: plain HTML, CSS and JavaScript with no build step. The docs are separate, in `apps/docs`, at docs.hoista.dev.

| File            | Purpose                                                                                                                                               |
| --------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------- |
| `index.html`    | The page. Brand logos and the mascot are drawn once in the `<svg class="sprite">` at the top of `<body>` and placed with `<use href="#logo-railway">` |
| `styles.css`    | Colours for light and dark (`data-theme` on `<html>`), then each section in page order, then the tablet and phone changes                             |
| `main.js`       | The light/dark switch (remembered per visitor), the copy buttons, the two menus and the terminal recording                                            |
| `assets/shots/` | Screenshots of Hoista Desktop, each in a light copy (`name.webp`) and a dark one (`name-dark.webp`)                                                   |
| `llms.txt`      | A plain-text summary of Hoista for AI tools (llmstxt.org format)                                                                                      |
| `skills/`       | Hoista's skills as plain markdown, the same as `skills/` at the repository root, linked from `llms.txt`                                               |
| `_redirects`    | Cloudflare Pages redirects: `hoista.dev/install.sh` goes to the installer in the repository                                                           |

To add a logo, add a `<symbol id="logo-name" viewBox="0 0 24 24">` to the sprite and its brand colour as `.logo-name` in `styles.css`.

Open `index.html` directly, or run `npm run webpage:dev` from the repository root to serve it at http://localhost:3000.

## Deploy

On Cloudflare Pages, with no build step. From the repository root, sign in once (it opens Cloudflare in your browser), then deploy:

```sh
npx wrangler login
npx wrangler pages deploy apps/webpage --project-name hoista
```

The first run creates the `hoista` project and prints a `*.pages.dev` address. Add `hoista.dev` under the project's Custom domains in the Cloudflare dashboard. Or connect the GitHub repo in the dashboard instead (Workers & Pages › Create › Pages › Connect to Git), with `apps/webpage` as the build output directory and no build command, so every push deploys.

## Docs

The documentation lives in `apps/docs` and is published by Mintlify at https://docs.hoista.dev. The page links there for the docs and for security. Preview it with `npm run docs:dev` from the repository root.
