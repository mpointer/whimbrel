import type { R2Bucket } from "@cloudflare/workers-types";

// `cloudflare:test`'s exported `env` (used in test/index.test.ts) is typed as
// `Cloudflare.Env`. `wrangler types` would normally generate this namespace
// augmentation (plus a large runtime-types bundle we don't need here, since
// `@cloudflare/workers-types` already covers our types) — this is the
// minimal hand-written equivalent, kept in sync with wrangler.jsonc's
// r2_buckets binding. `declare global` is required because this file has a
// top-level import, which makes it a module rather than an ambient script.
declare global {
  namespace Cloudflare {
    interface Env {
      UPDATES_BUCKET: R2Bucket;
    }
  }
}
