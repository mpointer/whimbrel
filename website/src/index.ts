import { handle, type Env } from "./app";

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
} satisfies ExportedHandler<Env>;
