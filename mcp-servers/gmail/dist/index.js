import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { createDraft, getThread, searchThreads, sendEmail, syncMessages } from "./gmail.js";
const server = new McpServer({ name: "whimbrel-gmail", version: "0.0.1" });
server.registerTool("send_email", {
    description: "Send an email via Gmail.",
    inputSchema: { to: z.string(), subject: z.string(), body: z.string() },
}, async ({ to, subject, body }) => {
    const token = process.env.GMAIL_ACCESS_TOKEN;
    if (!token) {
        throw new Error("GMAIL_ACCESS_TOKEN is not set");
    }
    const result = await sendEmail(to, subject, body, token);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
server.registerTool("search_threads", {
    description: "Search Gmail threads using Gmail's own search query syntax (e.g. \"from:alice newer_than:7d\"). Returns up to max_results threads with subject/from/date, plus a total estimate.",
    inputSchema: { q: z.string(), max_results: z.number().optional() },
}, async ({ q, max_results }) => {
    const token = process.env.GMAIL_ACCESS_TOKEN;
    if (!token) {
        throw new Error("GMAIL_ACCESS_TOKEN is not set");
    }
    const result = await searchThreads(q, max_results, token);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
server.registerTool("get_thread", {
    description: "Get a full Gmail thread by id -- every message's headers, plain-text body, and attachment metadata (filenames/types/sizes only, no attachment content).",
    inputSchema: { thread_id: z.string() },
}, async ({ thread_id }) => {
    const token = process.env.GMAIL_ACCESS_TOKEN;
    if (!token) {
        throw new Error("GMAIL_ACCESS_TOKEN is not set");
    }
    const result = await getThread(thread_id, token);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
server.registerTool("create_draft", {
    description: "Create a reply draft in a Gmail thread. Never sends anything -- the draft sits in the account's Drafts folder until the user sends it.",
    inputSchema: { thread_id: z.string(), to: z.string(), subject: z.string(), body: z.string() },
}, async ({ thread_id, to, subject, body }) => {
    const token = process.env.GMAIL_ACCESS_TOKEN;
    if (!token) {
        throw new Error("GMAIL_ACCESS_TOKEN is not set");
    }
    const result = await createDraft(thread_id, to, subject, body, token);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
server.registerTool("sync_messages", {
    description: "Incrementally sync Gmail messages. Pass the history_id from a previous call to get only what changed since; omit it to establish a baseline with no backfill (nothing older than the moment of connection is ever returned). If the response has needsFullResync: true, the history_id expired -- retry with no history_id.",
    inputSchema: { history_id: z.string().optional() },
}, async ({ history_id }) => {
    const token = process.env.GMAIL_ACCESS_TOKEN;
    if (!token) {
        throw new Error("GMAIL_ACCESS_TOKEN is not set");
    }
    const result = await syncMessages(history_id, token);
    return { content: [{ type: "text", text: JSON.stringify(result) }] };
});
const transport = new StdioServerTransport();
await server.connect(transport);
