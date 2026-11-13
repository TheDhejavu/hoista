# Adding a provider

Everything about a provider lives in one file in the engine and one in the desktop app. If adding one needs a change anywhere else, the shared code is missing something: change that, not the provider.

## In the engine

1. **One row** in `crates/domain/src/provider.rs`, in the `providers!` list: the variant, its slug (a compatibility contract, never renamed), the name as the provider writes it, what the provider calls the account a token belongs to, and its pages for making an account and a token. The CLI and the desktop app read those pages from here (the desktop app through `GET /providers`).

   ```rust
   /// Web services, workers and cron jobs.
   Render = "render", "Render", "workspace ID",
       signup: "https://dashboard.render.com/register",
       tokens: "https://dashboard.render.com/u/settings?add-api-key";
   ```

2. **One adapter file**, `crates/providers/src/<provider>.rs`, with:
   - its `ENTRY` (a `ProviderEntry`):
     - `capabilities`: what the adapter can do (see the table below), declared once; the adapter's `capabilities()` returns these;
     - `cli`: its command-line tool, when the adapter runs one: the program, the variable it reads the token from, the rest of its environment, and the commands the credential keeper may run on a server. `None` otherwise;
     - `api`: the base URL, every API host it may call, whether the provider's refusal messages are safe to pass on, and how its token signs a request in;
     - `accounts`: how a token lists its accounts (the request, where the rows are, the id field, how it pages);
     - `adapter`: how to build it.
   - a type implementing `ProviderAdapter`. Only `provider`, `capabilities`, `account_choices`, `verify` and `discover` are required; everything else defaults to "not supported".
   - `capabilities` saying exactly what it implements. The desktop app and agents read these and show nothing else, and the engine's generic code decides by them, never by the provider's name.
   - every address it calls declared once, as a named constant in the file (or the entry's `base`), and pages on the provider's site built by one function each.
   - its `FAKE`: a fake of the provider for the conformance suite. It needs only the account it reaches, one resource it finds (its reference and kind), and the few answers to list accounts, verify and find that resource. Everything else is answered for you (see Checks).

   | Flag                                                               | Set when the adapter implements                                                                                                                                                                                                                                                                                  |
   | ------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
   | `discover`                                                         | `discover`                                                                                                                                                                                                                                                                                                       |
   | `deploy`, `rollback`, `pause`, `domains`, `secrets`                | Hoista-managed deploys to this provider                                                                                                                                                                                                                                                                          |
   | `logs`, `metrics`, `usage`                                         | `log_history` / `open_logs`, provider-reported metrics and traffic, `usage`                                                                                                                                                                                                                                      |
   | `provider_details`, `provider_deploys`, `provider_variables`       | `provider_details`, the provider's own deploy history and actions, provider variables                                                                                                                                                                                                                            |
   | `provider_operations`                                              | `provider_operations` / `provider_operation` (its API catalog)                                                                                                                                                                                                                                                   |
   | `upload`, `new_project`, `new_from_folder`, `bills`                | `provider_upload`, `provider_new_project`, `provider_new_from_folder`, `provider_bill`                                                                                                                                                                                                                           |
   | `new_database`, `new_bucket`, `new_email_domain`, `new_from_image` | it makes that kind of resource (`provider_add` / `provider_create`): a database of its own, an object storage bucket, a domain to send email from, a resource running a container image                                                                                                                          |
   | `delete`                                                           | `delete`: it deletes a resource at the provider once the person confirms it by name                                                                                                                                                                                                                              |
   | `server`                                                           | `provider_new_hoista_server` (it can run Hoista as a remote)                                                                                                                                                                                                                                                     |
   | `browser_sign_in`                                                  | `oauth` (Hoista has an OAuth app there, set by `HOISTA_<PROVIDER>_CLIENT_ID` at build time, such as `HOISTA_VERCEL_CLIENT_ID`; `ProviderOAuth.logo` is its mark). The exchange is `oauth::public_token`. Register it at the provider as a public client (PKCE, no secret) and put its id in `.cargo/config.toml` |
   | `plan_deploy`                                                      | Hoista builds a folder and deploys it there from a plan (`plan_deploy`)                                                                                                                                                                                                                                          |
   | `containers`                                                       | it runs deploys as long-running containers: services plan there, and the builder writes a Dockerfile for a Node project without one                                                                                                                                                                              |
   | `cron`                                                             | it runs a web or job resource on exact UTC cron schedules                                                                                                                                                                                                                                                        |
   | `project_database`                                                 | Hoista creates a planned project's Postgres there                                                                                                                                                                                                                                                                |
   | `dns_zones`                                                        | it creates the DNS zone for a domain it adds                                                                                                                                                                                                                                                                     |

   Rules that depend on one resource, or on how the provider behaves, are adapter methods with defaults: `reports_bindings` and `keeps_known_role` (by reference), `provider_project` (the provider project a reference belongs to), `removes_services_by_action` and `names_added_databases`.

   Providers that can run Hoista and don't hide secrets on their own also implement `provider_hide_server_key`, or `server_key_steps` for the person to do it.

3. **One line** in `ENTRIES` in `crates/providers/src/lib.rs`, and `pub mod <provider>;` above it.
4. Add its API hosts to `docs/backend/network-hosts.txt` (a test checks the list matches).
5. Regenerate the types: `hoist schema > docs/backend/api/schema.json` and `hoist schema --typescript > docs/backend/api/types.ts`.

Nothing else in the engine names a provider. The journal forwards every adapter call, the keeper allows exactly the providers the entries declare, MCP lists connected providers and their capabilities in `get_my_access`, and `GET /providers` hands the capabilities to the desktop app.

That includes Hoista's own deploy planner (`crates/app/src/deploys/planning.rs`): it takes the providers and their capabilities (`Engine::providers`) and picks the first provider, in the order of `Provider::ALL`, that can run the project. A provider Hoista should deploy to sets `plan_deploy` (and `containers` or `cron` where it has them).

## In the desktop app

1. **One file**, `apps/desktop/src/shared/providers/<provider>.ts`, exporting its `ProviderEntry`. It holds:
   - the provider's name and logo;
   - its catalogue line;
   - how to make a token (the steps; the pages themselves come from the engine), and what to call the account;
   - its Accounts tile;
   - its step-by-step guide pages;
   - its own words: product names, compute and storage labels, and what a redeploy does;
   - its console links;
   - how its projects group resources;
   - its provider-tab sections, built from its operation catalog;
   - which of its operations give request logs.
2. **One line** in `PROVIDER_ENTRIES` in `shared/providers/entries.ts`.

`ProviderId` comes from the engine's `Provider`, plus the providers still coming soon (`shared/providers/soon.ts`). Move a provider out of `soon.ts` once its adapter ships.

## Checks

- `npm run lint` runs `scripts/check-providers.mjs`. It fails if a provider's id or name appears in desktop app code outside `src/shared/providers/`. Tests and comments are exempt. If a line has a real reason to name a provider, mark it with a `provider-name-ok` comment on the line before.
- The engine test `every_provider_has_one_provider_entry_in_order` fails if a provider has no `ENTRY` in `ENTRIES`, or an entry's base URL isn't one of its own API hosts.
- **The conformance suite** (`crates/providers/src/conformance.rs`, test `every_provider_adapter_passes`) runs every adapter in `ENTRIES` against its `FAKE` and checks that it:
  1. connects and finds projects: lists its account, verifies, and finds the fake's resource;
  2. refuses what it doesn't support: every capability it doesn't claim is refused as not supported, before anything is sent;
  3. shows only what its capabilities allow: every capability it claims is implemented;
  4. never repeats an unclear change: the fake fails every change it doesn't know after receiving it, and no change may be sent twice;
  5. never sends a token anywhere but the provider's API: every request goes to one of its entry's API hosts and carries exactly its token, and no error repeats the token.

  A second test, `the_suite_catches_an_adapter_that_breaks_the_rules`, runs a deliberately broken adapter to show each check works. When writing a `FAKE`, `CONFORMANCE_TRACE=1 cargo test -p hoista-server --lib conformance` prints every request the adapter makes while connecting and finding projects.
