import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { listEvents, readMail } from "./graph.js";

const server = new McpServer({ name: "whimbrel-outlook365", version: "0.0.1" });

server.registerTool(
  "list_events",
  {
    description: "List events on the signed-in user's Outlook calendar between two ISO 8601 timestamps",
    inputSchema: { time_min: z.string(), time_max: z.string() },
  },
  async ({ time_min, time_max }) => {
    const token = process.env.MICROSOFT_ACCESS_TOKEN;
    if (!token) {
      throw new Error("MICROSOFT_ACCESS_TOKEN is not set");
    }
    const events = await listEvents(time_min, time_max, token);
    return { content: [{ type: "text" as const, text: JSON.stringify(events) }] };
  },
);

server.registerTool(
  "read_mail",
  {
    description: "Read the signed-in user's most recent Outlook mail messages",
    inputSchema: {},
  },
  async () => {
    const token = process.env.MICROSOFT_ACCESS_TOKEN;
    if (!token) {
      throw new Error("MICROSOFT_ACCESS_TOKEN is not set");
    }
    const mail = await readMail(token);
    return { content: [{ type: "text" as const, text: JSON.stringify(mail) }] };
  },
);

const transport = new StdioServerTransport();
await server.connect(transport);
