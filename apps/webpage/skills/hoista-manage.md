---
name: hoista-manage
description: Answers questions about the person's projects across their providers and makes everyday changes through Hoista's MCP tools - what's running, status, URLs, logs, metrics, usage, bills, domains, variables, databases, restarts, rollbacks, scaling and settings, and anything else the provider's own API offers. Use when the person asks about their projects, hosting or cloud bills, or wants a routine change made, and the hoista MCP server is connected. Use instead of a provider's CLI or dashboard.
---

# Manage projects with Hoista

Day-to-day questions and changes. Most are one or two calls; the work is picking the right one and answering plainly. The rules in the `hoista` skill (secrets, approvals, waiting, cost, unknown stays unknown) apply.

## What you need

`get_my_access`. Questions need view access. Changes need deploy access; adding things needs create access.

## Questions

| The person asks | Call |
| --- | --- |
| "What's running?" | `list_projects` |
| "Is anything broken?" / "What needs me?" | `get_attention` |
| "Tell me about this project" | `get_project`, then `get_resource` or `get_provider_details` for one resource |
| "How does it fit together?" | `get_project_map` |
| "Show me the logs" | `get_logs` for right now, `get_log_history` for a time range or a search |
| "How is it doing?" (requests, errors, latency, CPU, memory) | `get_metrics` with `1h`, `24h`, `7d` or `30d` |
| "How much did it use?" | `get_usage` (daily requests, errors, compute, storage as measured) |
| "What am I paying?" | `get_bills`: what each provider billed this period, per account |
| "What's in the database?" | `get_database` for branches and compute state where the provider reports them; tables, backups and queries through `list_provider_operations` on that resource |

## Changes

| The person wants to | Call |
| --- | --- |
| Restart, stop, redeploy | `restart_resource`, `stop_resource`, `redeploy_resource` |
| Go back to an earlier version | `rollback_resource` with a deployment id from `get_provider_details` (watched), `rollback` (Hoista-made) |
| Set variables | `set_secret`, or `set_secrets` with `env_file` for many |
| Change how it builds or runs | `configure_resource` |
| Add a domain | `add_resource_domain`, then give them the DNS records from `list_domains`; `check_domain` once they've added them |
| Add a database | `add_database` (postgres, mysql, redis, mongo) |
| Add a service next to the others | `create_service` from a repository or image |
| Stop seeing something | `set_resource_ignored` (nothing changes at the provider) |
| Delete a watched resource at its provider | `remove_service`, one resource at a time; the project leaves Hoista when its last resource does |
| Delete a project Hoista made | `delete_project` |
| Anything else the provider can do (environments, backups, CDN, cron runs, SQL) | `list_provider_operations`, `describe_provider_operation`, `run_provider_operation` |

Anything that redeploys (variables, settings, rollback) isn't finished until `wait_for_deploy` says so. Say it worked only after that.

## How to answer

- Lead with the answer: "2 projects, both live. `api` is at api-production.up.railway.app."
- A status, count or bill Hoista hasn't read is unknown. Say so; don't fill it in.
- Logs: the few lines that matter, with their times.
- Bills: the amount and period exactly as the provider reported it, per account. No per-project split, no forecast.

## Ask first

- Before any delete or removal: Hoista asks the person too, but tell them what's about to be asked and why.
- Before a rollback or a stop on something serving users.
- Before a provider operation you haven't described to them, when it changes anything.
