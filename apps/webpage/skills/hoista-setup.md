---
name: hoista-setup
description: Gets the person's hosting into Hoista through its MCP tools - connecting a provider account, bringing in projects that already run on Railway, Vercel, Cloudflare, Fly.io, Render, Neon, Supabase and the rest, grouping them into projects, and linking a project to its GitHub repository or local folder. Use when the person wants to connect or add a provider, import or bring in existing projects, set Hoista up for this repo, or when get_my_access shows no accounts, and the hoista MCP server is connected.
---

# Set up with Hoista

Getting someone from "I have apps on three providers" to "Hoista sees all of them, grouped the way I think about them". Nothing here changes anything at a provider: connecting reads, importing only starts watching.

## What you need

`get_my_access` first. It lists connected accounts (with `can`, what each provider does) and whether this connection may create. Importing and linking need change access; with View only, say so and stop.

## Connect a provider

1. Call `connect_account` with the provider. It opens a guided connection in the Hoista app on this computer: the person signs in on the provider's page or pastes a token there. Connecting an account can need the person's approval first; if the call returns pending, say so and retry with the `approval_id` once they allow it.
2. Tell them it's waiting for them in Hoista. Never ask for a token in chat and never put one in a call: the token goes into Hoista, not into this conversation.
3. When they say it's done, call `get_my_access` again and check the account is there with what you need in its `can`.

## Bring in what already runs

1. `list_import_candidates` lists resources Hoista found on connected accounts that aren't in a project yet.
2. `suggest_imports` proposes groupings from shared names and hostnames, such as a web service and the Postgres it uses. Proposals marked ambiguous need the person's choice; ask, don't pick.
3. Show the person the grouping in a few lines, one project per line with its resources and providers, and wait for an OK unless they already said to go ahead.
4. `import_resources` with the account and the resource ids. With `project_id` it adds them to an existing project.
5. Pieces found by name rather than by `suggest_imports`: `resolve_resources` with a description (names, hostnames, ids) lists the candidates. Check each one's account, then group them with `link_resources`: the same `description` with `project_id`, the `resource_ids` you chose and the `review_token` from the first call.
6. To record that one resource uses another (the API and the database it reads, at different providers), `link_resources` with `from_resource`, `to_resource` and `via` (such as `DATABASE_URL`).
7. Something they don't want to see at all: `set_resource_ignored`. It stays at the provider, untouched.

Importing never changes anything at the provider. Say so if the person seems worried about it.

## Link a project to its code

So the person and later agents know which code a project runs:

| The code is | Call |
| --- | --- |
| In a GitHub repository | `list_repositories` with the GitHub account (from `get_my_access`), then `link_repository` with the repository and branch |
| In a folder on this computer | `link_folder` with its absolute path |
| The folder you're working in | `use_project` as well, so the next session in this folder starts on this project |

A watched resource that should build from GitHub from now on: `connect_repo` with `owner/name` and the branch.

## Environments

`list_environments` shows Hoista's environments (default, production, staging). They're Hoista's own grouping, separate from a provider's environments. `move_project` moves a project into another one, which changes which agents can reach it.

## Check it worked

- `list_projects` shows each project the person expected, with the providers you imported from.
- `get_project` on one of them lists every resource you added.
- Anything left in `list_import_candidates` is something you meant to leave out.

## What to tell the person

One line per project: its name, what it's made of, and where each piece runs. Then what's left unimported, if anything, and why.

## Ask first

- Before importing, when the grouping wasn't spelled out by the person.
- Before choosing between ambiguous groupings.
- Before moving a project to another environment: it changes which agents can see it.
