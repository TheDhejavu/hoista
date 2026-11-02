# apps/web

The hoista.ai website: plain HTML, CSS and JavaScript with no build step.

| File | Purpose |
| --- | --- |
| `index.html` | The landing page |
| `styles.css` | Layout and the dark and light themes, as colour tokens |
| `theme.js` | Applies the saved or system theme before the page paints |
| `main.js` | Install tabs, copy button, download picker and the hero animation |
| `assets/` | Icons, favicon and the desktop app screenshot |

Run it locally with `python3 -m http.server 8080 --directory apps/web` from the repository root. To deploy, point Cloudflare Pages at `apps/web` with no build command.

Before publishing a change, look at the page at desktop and phone widths in both themes and check the browser console for errors.
