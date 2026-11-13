# apps/docs

The Hoista documentation site, built with [Mintlify](https://mintlify.com). Pages are `.mdx` files; `docs.json` holds the navigation and theme.

## Preview

From the repository root:

```sh
npm run docs:dev                # http://localhost:3001
npm run docs:validate           # Mintlify's own check: navigation and links
```

Run `npm run docs:validate` before publishing.

## Pages that follow the code

Update these when the code they describe changes:

| Page | Follows |
| --- | --- |
| `cli/*.mdx` (the CLI tab) | `hoist --help` |
| `reference/mcp-tools.mdx`, `assets/mcp-catalog.json` | The MCP tools in `crates/server/src/mcp.rs` |

## Writing

- Describe what the code does today. Anything not built yet stays out.
- Plain words, the providers' own terms, short pages. Example data uses names like `my-app` and Sam Rivera.
- Show the desktop step and the CLI step where both exist, and link to the CLI tab (`/cli/...`) instead of repeating flags.

## Pages

Each page follows one shape, so they all read alike:

1. A sentence or two saying what the page covers, before any heading.
2. Sections with plain headings. Settings, flags and commands go in tables.
3. Every picture in a `<Frame>` with a one-sentence caption saying what it shows.
4. `<Tip>`, `<Note>` and `<Warning>` sparingly, for what the reader would otherwise miss.
5. `## Related` last, with three cards.

The home page and the CLI and MCP tool pages are the exceptions. Put a sentence between two pictures; never show them back to back.

## Diagrams

Diagrams live in `assets/diagrams/` as a `-light.svg` and `-dark.svg` pair, shown through a snippet in `snippets/diagrams/`. They share one style: a titled sheet with the hoista.dev mark, column headings, white cards with an icon tile, `hoistd` as the dark-bordered card in a dashed boundary, and thin right-angle connectors with open arrowheads and labels on chips.

Each diagram is two SVG files, `assets/diagrams/<name>-light.svg` and `-dark.svg`, plus a snippet in `snippets/diagrams/` that shows the right one with its caption. Change both files together. A page imports the snippet:

```mdx
import ProjectResources from '/snippets/diagrams/project-resources.mdx';

<ProjectResources />
```

## Screenshots

The screenshots in `assets/screens/` are of the desktop app on its sample data (Sam Rivera's projects), in light and dark, saved as WebP at twice the size shown. Show both, so the reader sees the one matching their theme:

```mdx
<Frame caption="What the screenshot shows.">
  <img className="shot-light" src="/assets/screens/projects-light.webp" alt="..." />
  <img className="shot-dark" src="/assets/screens/projects-dark.webp" alt="..." />
</Frame>
```

Retake a screenshot after changing a screen the docs show.

## Publish

Connect the repository to a Mintlify project with `apps/docs` as the documentation folder, and add `docs.hoista.dev` as its custom domain (a CNAME record to Mintlify, as its dashboard shows).
