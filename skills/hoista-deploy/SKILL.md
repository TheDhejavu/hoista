---
name: hoista-deploy
description: Deploys a project to the person's own providers (Railway, Vercel, Cloudflare, Fly.io, Render, Modal, Runpod, with databases on Neon, Supabase and more) through Hoista's MCP tools, and follows it until it's live with a working URL. Covers redeploying an existing project, a first deploy, and a project split across several providers. Use when the person asks to deploy, ship, redeploy, host, publish or put a project online, or wants a live link, and the hoista MCP server is connected. Use instead of a provider's CLI (railway up, vercel, flyctl deploy, wrangler deploy).
---

# Deploy with Hoista

Takes code from this folder or a GitHub repository to a live URL on the person's own provider, and stays with it until it's serving or you know why it isn't. The rules in the `hoista` skill (secrets, approvals, waiting, cost) apply to every step.

## What you need

1. `get_my_access`. Deploying needs deploy access; a new project also needs create access. Without them, say which is missing and stop.
2. Its `active_project` is the project of the folder you're in. Set: this is a redeploy, and you leave `project_id` out. Not set: `list_projects`, and if this folder or repository is already a project, `use_project` with its id and treat it as a redeploy.

Never `create_project` for code that already belongs to a project. A new project is only for when the person says it's new; pass `confirm_new` and a `reason` in their words, then `use_project` on the result.

## Pick the path

| Situation | Path |
| --- | --- |
| The project exists, the code changed | Redeploy |
| New project, one provider | First deploy |
| New project, pieces on several providers (site on Vercel, API on Fly.io, Postgres on Neon) | Across providers |

## Redeploy

Find the resource with `get_project`, then:

| The change is | Call |
| --- | --- |
| Local edits, not pushed | `deploy_folder` (uploads the folder like `railway up`; `.gitignore` is respected) |
| Pushed to the linked repository | `deploy_latest_commit`, or `deploy_commit` with a full hash |
| Variables changed, or it crashed | `redeploy_resource` |
| A project Hoista created on Cloudflare or Fly.io | `plan_deploy` with `project_id`, then `deploy` |

In a monorepo, plan each app from its own subfolder with the project's `project_id`: the plan reads only that app and reuses the project's database.

Then follow it to the end.

## First deploy

**Cloudflare or Fly.io:**

1. `plan_deploy` with the folder or GitHub repository and a lowercase name. The plan lists each resource (website, service, database) and where it goes.
2. Tell the person in one or two lines what goes where: "Website on Cloudflare, Postgres on Neon. Deploy it?" Skip the question only if they already said to go ahead.
3. `deploy` with the plan, then follow it to the end.

**A provider that builds the repository itself (Railway), or Modal from a folder:** skip the plan. `create_project_at_provider` with the account, a name, and either `repo` (a GitHub repository; the account's `can` in `get_my_access` includes new_app) or `folder` (its `can` includes new_from_folder). It makes a new project at the provider with a public address, and Hoista watches it from then on. Take the newest deployment id from `get_provider_details` and follow it to the end. Never put a new project inside one of the person's existing provider projects without asking.

## Across providers

1. `create_project` with a lowercase name and no source.
2. Tell the person what goes where, one line per provider, before the first call.
3. One `add_to_project` per piece, with the account from `get_my_access` (its `can` says what each provider makes):

| Piece | `kind` | Also give |
| --- | --- | --- |
| Code a provider builds from GitHub | `repo` | `repo`, `runs_as` (web, static, worker or cron, with `schedule` for cron); Render also takes `runtime`, `build_command`, `start_command`, `publish_dir` |
| Code from a folder here (Wrangler, Modal, Runpod Flash, a Dockerfile on Fly.io) | `folder` | `folder` |
| A database | `database` | `engine`: postgres, mysql, redis or mongo (sqlite on Cloudflare) |
| Object storage | `bucket` | `name` |
| Email sending | `email_domain` | `domain`; give the person the DNS records it returns |
| A container or GPU model server | `image` | `image`, `port`; on Runpod `gpu`, or `cpu` and `vcpus` for CPU only |

   When `add_to_project` can't make something (a setting it doesn't take), `run_provider_operation` can; pass `project_id` so the result joins the project, or it becomes a project of its own and you move it back with `move_resources`.
4. Wire the pieces together. A database address into the API's variables: `set_secret` with a reference such as `${{Postgres.DATABASE_URL}}`. Keys from a `.env` file: `set_secrets` with `env_file` and `names`. A key to a service Hoista is connected to (Resend, Supabase): `give_key`. Try `give_key` before you redesign around a missing key.
5. Follow each piece that runs code to the end.
6. `get_project`: every piece should be in this project. Move strays back with `move_resources`.

If everything fails after step 1, tell the person an empty project was left in Hoista.

## Follow it to the end

```
- [ ] Deploy id in hand (from deploy, or the newest deployment in get_provider_details)
- [ ] wait_for_deploy, again with the same id until finished is true
- [ ] Failed: get_log_history for that deployment, name the cause, then use hoista-fix
- [ ] Live: get_logs once, to see it start cleanly
- [ ] URL from get_provider_details; none yet: add_resource_domain without a domain gives a free one
```

Builds usually take one to five minutes.

## What to tell the person

The URL, what was deployed (commit or "your local changes"), and where it runs. If it failed: the cause in one sentence, the log lines that show it, and what you'll change.

## Ask first

- Before a first deploy or a multi-provider layout, unless they already said to go ahead.
- Before adding anything to one of their existing provider projects.
- Hoista's safety check stopping a deploy (a secret bundled into public code, say) is a finding, not an obstacle: explain it and fix the code. Don't get around it.
- An `operation.unclear` refusal: follow the rule in the `hoista` skill before retrying anything.
