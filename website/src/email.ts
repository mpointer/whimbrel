import type { LeadInput } from "./leads";

/** One transactional email, sent through Resend's HTTP API. The sender must be on a
 * domain verified in the Resend account (futuremeanswered.com is). */
export interface Email {
  from: string;
  to: string;
  replyTo?: string;
  subject: string;
  text: string;
  html: string;
}

export async function sendEmail(apiKey: string, email: Email, fetcher: typeof fetch): Promise<boolean> {
  try {
    const res = await fetcher("https://api.resend.com/emails", {
      method: "POST",
      headers: { authorization: `Bearer ${apiKey}`, "content-type": "application/json" },
      body: JSON.stringify({
        from: email.from,
        to: [email.to],
        ...(email.replyTo ? { reply_to: email.replyTo } : {}),
        subject: email.subject,
        text: email.text,
        html: email.html,
      }),
    });
    // Resend rejects a bad key before the request reaches the account's logs, so say why here.
    if (!res.ok) console.error(`email: Resend refused the send: ${res.status} ${await res.text()}`);
    return res.ok;
  } catch (err) {
    console.error(`email: send failed: ${err}`);
    return false;
  }
}

const escapeHtml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

/** Personal download links: `?lead=` stands in for the cookie, so the links work from any
 * browser the email is opened in. The id is the same random value the cookie holds. */
export function downloadLinks(origin: string, leadId: string) {
  const q = `?lead=${encodeURIComponent(leadId)}`;
  return { mac: `${origin}/api/download/mac${q}`, windows: `${origin}/api/download/windows${q}` };
}

/** Sent once, when an email address registers for the first time. */
export function welcomeEmail(origin: string, leadId: string, lead: LeadInput, from: string, replyTo: string | undefined): Email {
  const links = downloadLinks(origin, leadId);
  const first = lead.name.split(/\s+/)[0] ?? lead.name;
  const primary = lead.platform === "mac" ? "Mac" : "Windows";
  const text = [
    `Hi ${first},`,
    "",
    "Thanks for registering for Whimbrel early access. Your download links are below; they're yours, so keep this email to download again or to set up another computer.",
    "",
    `Whimbrel for Mac (Apple silicon, macOS 14 or later): ${links.mac}`,
    `Whimbrel for Windows (Windows 11, 64-bit, early access): ${links.windows}`,
    "",
    "On Windows, SmartScreen may warn that the installer is unrecognized. Choose More info, then Run anyway.",
    "",
    "Whimbrel is free and open source. If something doesn't work or you have an idea, just reply to this email or write to hello@whimbrelapp.com.",
    "",
    "Mike",
    "Whimbrel",
  ].join("\n");
  const button = (href: string, label: string, primaryButton: boolean) =>
    `<a href="${escapeHtml(href)}" style="display:inline-block;padding:12px 20px;border-radius:10px;text-decoration:none;font-weight:600;${
      primaryButton ? "background:#A8502E;color:#ffffff;" : "background:#ffffff;color:#241E19;border:1px solid #E4D9C2;"
    }">${label}</a>`;
  const html = `<!doctype html><html><body style="margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19">
<div style="max-width:560px;margin:0 auto;padding:32px 20px">
<p style="font-size:16px;line-height:1.5">Hi ${escapeHtml(first)},</p>
<p style="font-size:16px;line-height:1.5">Thanks for registering for Whimbrel early access. Your download links are below. They're yours, so keep this email to download again or to set up another computer.</p>
<p style="margin:24px 0">${button(lead.platform === "mac" ? links.mac : links.windows, `Download for ${primary}`, true)}
&nbsp; ${button(lead.platform === "mac" ? links.windows : links.mac, `Download for ${primary === "Mac" ? "Windows" : "Mac"}`, false)}</p>
<p style="font-size:14px;line-height:1.5;color:#6B5F4F">Mac: Apple silicon, macOS 14 or later. Windows: Windows 11, 64-bit, early access. On Windows, SmartScreen may warn that the installer is unrecognized; choose More info, then Run anyway.</p>
<p style="font-size:16px;line-height:1.5">Whimbrel is free and open source. If something doesn't work or you have an idea, just reply to this email or write to hello@whimbrelapp.com.</p>
<p style="font-size:16px;line-height:1.5">Mike<br>Whimbrel</p>
</div></body></html>`;
  return { from, to: lead.email, replyTo, subject: "Your Whimbrel download links", text, html };
}

/** Sent to the team on every registration, new or returning. */
export function leadAlertEmail(lead: LeadInput, isNew: boolean, country: string | null, from: string, to: string): Email {
  const rows: [string, string | null][] = [
    ["Name", lead.name],
    ["Email", lead.email],
    ["Company", lead.company],
    ["Role", lead.role],
    ["Team size", lead.teamSize],
    ["Platform", lead.platform === "mac" ? "Mac" : "Windows"],
    ["Product emails", lead.marketingOptIn ? "opted in" : "no"],
    ["Country", country],
    ["UTM", [lead.utmSource, lead.utmMedium, lead.utmCampaign].filter(Boolean).join(" / ") || null],
    ["Referrer", lead.referrer],
    ["Landing page", lead.landingPage],
  ];
  const present = rows.filter((r): r is [string, string] => r[1] !== null && r[1] !== "");
  const who = lead.company ? `${lead.name} (${lead.company})` : lead.name;
  const subject = `${isNew ? "New" : "Returning"} Whimbrel lead: ${who}, ${lead.platform === "mac" ? "Mac" : "Windows"}`;
  const text = present.map(([k, v]) => `${k}: ${v}`).join("\n");
  const html = `<table style="font-family:-apple-system,'Segoe UI',sans-serif;font-size:14px;border-collapse:collapse">${present
    .map(([k, v]) => `<tr><td style="padding:4px 12px 4px 0;color:#6B5F4F">${k}</td><td style="padding:4px 0">${escapeHtml(v)}</td></tr>`)
    .join("")}</table>`;
  return { from, to, replyTo: lead.email, subject, text, html };
}
