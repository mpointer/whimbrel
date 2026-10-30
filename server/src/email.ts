/** One plain-text transactional email, sent through Resend's HTTP API. The
 * sender must be on a domain verified in the Resend account. */
export interface Email {
  from: string;
  to: string;
  replyTo?: string;
  subject: string;
  text: string;
}

export const EMAIL_FROM = "Whimbrel <whimbrel@futuremeanswered.com>";
export const EMAIL_REPLY_TO = "hello@whimbrelapp.com";

/** True only when Resend accepted the email. Never throws. */
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
      }),
    });
    // Status only: the body can echo the recipient and message.
    if (!res.ok) console.error(`email: Resend refused the send: ${res.status}`);
    return res.ok;
  } catch (err) {
    console.error(`email: send failed: ${err}`);
    return false;
  }
}
