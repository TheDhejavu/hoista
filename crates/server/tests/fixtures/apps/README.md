# Required pipeline samples

These are deliberately small backend acceptance fixtures. The regular `pipeline_fixtures` test uses these folders and the existing sibling `tanstack-ssr` / `next-ssr` fixtures to cover all seven TRD samples through actual inspection, deterministic planning, bundled pricing metadata, environment-name collection and leak scanning. It prepares and scans the actual FastAPI Docker build context; Express/Prisma context preparation is opt-in because it installs migration tooling. It runs the deliberate leaked-key fixture through the real LocalBuilder rejection path.

| Sample | Location | Additional execution evidence |
| --- | --- | --- |
| Vite SPA | `vite-spa` | Opt-in real locked Vite build in `worker_build` |
| TanStack Start | `../tanstack-ssr` | Existing real build and packaged Worker SSR test |
| Next.js | `../next-ssr` | Existing real OpenNext build and packaged Worker SSR test |
| Express + Prisma | `express-prisma` | Checked-in Postgres schema/migration and opt-in prepared Docker context; image execution/live database migration remain separate |
| FastAPI | `fastapi` | Prepared Docker context and Python environment-name detection; container execution remains separate |
| Supabase-backed SPA | `supabase-spa` | Real locked Vite build; existing Supabase backend is preserved, no Neon provision or migration |
| Deliberately leaked key | `leaked-key` | Fake, unusable Stripe-shaped value must be rejected with a redacted error |

`worker_build::real_vite_and_supabase_spas_build_public_assets_without_backend_migration` installs the checked-in npm lockfiles in disposable build workspaces. It supplies clearly fictional public configuration, verifies compiled assets and keeps provider accounts out of scope. These tests do not contact Supabase. Private/service-role keys must never be substituted for the public fixture value.

The Python requirements pin direct dependencies; they are not a fully hashed transitive lock. Docker base tags are fixture inputs, not a claim of reproducible production images. Planning with no usage history keeps recent workload prices unavailable instead of fabricating free operation. This test inventory does not establish every provider cost or the remaining live/platform acceptance requirements.
