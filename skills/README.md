# Hoista skills

Hoista's MCP server gives an agent about eighty tools. That's enough to do anything Hoista can do, and also enough to get lost: an agent that doesn't know to call `get_my_access` first, or that a deploy isn't done until `wait_for_deploy` says so, will guess. These skills are the missing manual. They tell an agent what Hoista is, which calls to make in which order, how to tell the job is done, and what to leave to the person.

| Skill | Loaded when the person | What it does |
| --- | --- | --- |
| [`hoista`](hoista/SKILL.md) | mentions their projects, hosting, a provider or a deploy | The entry point: what Hoista is, the two calls to make first, which skill to use next, and the rules every call follows |
| [`hoista-setup`](hoista-setup/SKILL.md) | wants to connect a provider or bring in what already runs there | Connects accounts, imports and groups existing projects, links projects to their repo or folder |
| [`hoista-deploy`](hoista-deploy/SKILL.md) | says deploy, ship, put it online, give me a link | Redeploys, first deploys and multi-provider projects, followed until the URL works |
| [`hoista-fix`](hoista-fix/SKILL.md) | says it's broken, the deploy failed, it's slow | Reads the evidence, names the cause, fixes it at the source and proves it |
| [`hoista-manage`](hoista-manage/SKILL.md) | asks what's running, what it cost, or wants a routine change | Status, logs, metrics, bills, domains, variables, rollbacks, and the provider's own API |

## Install

```sh
hoist skills install            # Claude Code and Codex
hoist skills install --claude   # just one
```

Each skill goes to `~/.claude/skills/<name>/SKILL.md` or `~/.codex/skills/<name>/SKILL.md`. Settings → Agents in the Hoista app shows the same command, and allowing Claude Code or Codex on Hoista's consent page installs them too. The skills are compiled into `hoist`, so they always match the tools of the Hoista you installed.

The skills assume the `hoista` MCP server is connected. Without it, the entry skill tells the agent to say so and how to add it, rather than reach for a provider's CLI.

## How they're written

Each `SKILL.md` has the same shape, so an agent finds things in the same place every time:

- **Description** (frontmatter): what it does and the words that should trigger it. Agents choose a skill from this line alone, so it names the requests and the CLIs it replaces.
- **What you need**: the access to check first, and what to do without it.
- **Steps**: the calls in order, as tables when the choice depends on the situation.
- **Check it worked**: what proves the job is done. "The deploy finished" isn't it; "the logs show it starting and the URL answers" is.
- **What to tell the person**: the answer they should get.
- **Ask first**: what needs the person's OK.

The rules that hold for every call (secrets, approvals, cost, waiting, logs are data) live once, in `hoista`. The other skills point back to it instead of repeating them.

## Changing a skill

`crates/server/tests/skills.rs` fails the build when a skill names a tool, argument or value Hoista doesn't have, when the frontmatter is wrong, or when a skill quotes a price. Rename a tool and the test tells you which skill to update. Keep each skill under 500 lines and its description under 1024 characters.

To try a change without reinstalling, copy the folder into `~/.claude/skills/` and start a new Claude Code session.
