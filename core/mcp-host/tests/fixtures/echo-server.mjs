import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";

const server = new McpServer({ name: "echo-fixture", version: "0.0.1" });

server.registerTool(
  "echo",
  {
    description: "Echoes back the given message, prefixed with an env var if MCP_HOST_TEST_ENV_VAR is set",
    inputSchema: { message: z.string() },
  },
  async ({ message }) => {
    if (message === "fail") {
      throw new Error("intentional failure for testing");
    }
    const prefix = process.env.MCP_HOST_TEST_ENV_VAR ? `${process.env.MCP_HOST_TEST_ENV_VAR}:` : "";
    return { content: [{ type: "text", text: `${prefix}${message}` }] };
  },
);

const transport = new StdioServerTransport();
await server.connect(transport);
