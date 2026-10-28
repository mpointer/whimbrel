import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { createEvent, listEvents, syncEvents } from "./calendar.js";

const server = new McpServer({ name: "whimbrel-google-calendar", version: "0.0.1" });

server.registerTool(
  "list_events",
  {
    description: "List events on the primary Google Calendar between two ISO 8601 timestamps",
    inputSchema: { time_min: z.string(), time_max: z.string() },
  },
  async ({ time_min, time_max }) => {
    const token = process.env.GOOGLE_CALENDAR_ACCESS_TOKEN;
    if (!token) {
      throw new Error("GOOGLE_CALENDAR_ACCESS_TOKEN is not set");
    }
    const events = await listEvents(time_min, time_max, token);
    return { content: [{ type: "text" as const, text: JSON.stringify(events) }] };
  },
);

server.registerTool(
  "create_event",
  {
    description: "Create an event on the primary Google Calendar. start_iso/end_iso are full ISO 8601 datetimes with timezone offset.",
    inputSchema: { title: z.string(), start_iso: z.string(), end_iso: z.string() },
  },
  async ({ title, start_iso, end_iso }) => {
    const token = process.env.GOOGLE_CALENDAR_ACCESS_TOKEN;
    if (!token) {
      throw new Error("GOOGLE_CALENDAR_ACCESS_TOKEN is not set");
    }
    const event = await createEvent(title, start_iso, end_iso, token);
    return { content: [{ type: "text" as const, text: JSON.stringify(event) }] };
  },
);

server.registerTool(
  "sync_events",
  {
    description:
      "Incrementally sync events on the primary Google Calendar. Pass the sync_token from a previous call to get only what changed since; omit it for an initial full sync. If the response has needsFullResync: true, the token expired -- retry with no sync_token.",
    inputSchema: { sync_token: z.string().optional() },
  },
  async ({ sync_token }) => {
    const token = process.env.GOOGLE_CALENDAR_ACCESS_TOKEN;
    if (!token) {
      throw new Error("GOOGLE_CALENDAR_ACCESS_TOKEN is not set");
    }
    const result = await syncEvents(sync_token, token);
    return { content: [{ type: "text" as const, text: JSON.stringify(result) }] };
  },
);

const transport = new StdioServerTransport();
await server.connect(transport);
