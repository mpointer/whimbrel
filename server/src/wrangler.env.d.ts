import type { KVNamespace } from "@cloudflare/workers-types";

declare global {
  namespace Cloudflare {
    interface Env {
      ENROLLMENT_HANDOFF: KVNamespace;
    }
  }
}

export {};
