import { handle, type Env } from "./app";
import { runScheduled } from "./followup";

// The entry module may only export handlers, so the routes live in app.ts.
export default {
  async fetch(request: Request, env: Env, ctx: ExecutionContext): Promise<Response> {
    try {
      return await handle(request, env, ctx);
    } catch (err) {
      console.error("unhandled", err);
      return Response.json({ ok: false, errors: { form: "Something went wrong. Please try again." } }, { status: 500 });
    }
  },
  // Hourly (wrangler.jsonc triggers): next-day follow-ups and Resend contact sync.
  async scheduled(controller: ScheduledController, env: Env, ctx: ExecutionContext): Promise<void> {
    ctx.waitUntil(runScheduled(env, controller.scheduledTime).then((r) => console.log(`cron: ${r.followups} follow-ups, ${r.synced} contacts synced`)));
  },
} satisfies ExportedHandler<Env>;
