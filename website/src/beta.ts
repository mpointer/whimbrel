// The mobile beta sign-up (docs/superpowers/specs/2026-11-06-mobile-beta-design.md):
// POST /api/beta saves a request and alerts Mike; /beta/review is the signed page his
// Approve and Decline links open. A GET only shows the request; only a POST changes it.
import { readBody, redirect, saveLead, spamCheck, type Env } from "./app";
import { send, type Email } from "./email";
import { oneOf, parseContact } from "./leads";
import { b64url, fromB64url, hmacKey, language, leadById, type Language } from "./preferences";

export const PHONES = ["iphone", "android", "both"] as const;
export const COMPUTERS = ["mac", "windows", "none"] as const;
type Phones = (typeof PHONES)[number];
type Action = "approve" | "decline";

const DEFAULT_ANDROID_RELEASE_URL = "https://github.com/mpointer/whimbrel/releases/latest";
const THANKS = "/mobile-beta/thanks/";

interface BetaRow {
  id: string;
  lead_id: string;
  email: string;
  phones: Phones;
  computer: string;
  status: "pending" | "approved" | "declined";
  created_at: string;
  decided_at: string | null;
  install_email_sent_at: string | null;
}

const esc = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
const PHONE_LABEL: Record<Phones, string> = { iphone: "iPhone", android: "Android", both: "iPhone and Android" };
const COMPUTER_LABEL: Record<string, string> = { mac: "Mac", windows: "Windows", none: "None" };

// ---- POST /api/beta ---------------------------------------------------------------------

export async function handleBeta(request: Request, env: Env, ctx: ExecutionContext, fetcher: typeof fetch): Promise<Response> {
  const parsedBody = await readBody(request);
  if (!parsedBody) return Response.json({ ok: false, errors: { form: "Unreadable request." } }, { status: 400 });
  const { body, isJson } = parsedBody;
  const fail = (status: number, errors: Record<string, string>) =>
    isJson ? Response.json({ ok: false, errors }, { status }) : redirect("/mobile-beta/?error=1#beta");
  const done = () => (isJson ? Response.json({ ok: true, next: THANKS }, { headers: { "cache-control": "no-store" } }) : redirect(THANKS));

  const { contact, errors } = parseContact(body);
  const phones = oneOf(body.phones, PHONES);
  const computer = oneOf(body.computer, COMPUTERS);
  if (!phones) errors.phones = "Choose iPhone, Android or both.";
  if (!computer) errors.computer = "Choose Mac, Windows or none.";
  if (Object.keys(errors).length > 0 || !phones || !computer) return fail(400, errors);

  const spam = await spamCheck(request, env, contact, fetcher);
  if (spam === "bot") return done();
  if (spam === "refused") return fail(403, { form: "Verification failed. Refresh the page and try again." });

  const { id: leadId, isNew, now, countryCode } = await saveLead(request, env, ctx, { ...contact, platform: computer === "none" ? null : computer }, fetcher);
  // A lead that came only for the phone beta never got desktop download links, so the
  // cron's next-day "your download links" note would not make sense to them.
  if (isNew) await env.DB.prepare("UPDATE leads SET followup_sent_at = ?2 WHERE id = ?1").bind(leadId, now).run();

  const row = await env.DB.prepare(
    `INSERT INTO beta_signups (id, lead_id, email, phones, computer, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
     ON CONFLICT(email) WHERE status = 'pending' DO UPDATE SET lead_id = excluded.lead_id, phones = excluded.phones, computer = excluded.computer
     RETURNING id, created_at`,
  )
    .bind(crypto.randomUUID(), leadId, contact.email, phones, computer, now)
    .first<{ id: string; created_at: string }>();
  if (!row) throw new Error("beta upsert returned no row");
  const isNewRequest = row.created_at === now;

  // Email never blocks or fails the sign-up. The tester's "we got it" goes once per request,
  // so the form can't be used to flood an address; Mike hears about every submission.
  if (env.EMAIL_FROM) {
    const origin = new URL(request.url).origin;
    const nowMs = Date.parse(now);
    const first = contact.name.split(/\s+/)[0] || contact.name;
    if (isNewRequest) {
      const email = receivedEmail(contact.email, contact.signupLanguage, first, env.EMAIL_FROM, env.EMAIL_REPLY_TO);
      ctx.waitUntil(send(env, "transactional", email, { leadId, origin, nowMs }, fetcher));
    }
    if (env.LEAD_ALERT_EMAIL) {
      const links = env.BETA_REVIEW_SECRET
        ? { approve: await reviewUrl(env.BETA_REVIEW_SECRET, origin, row.id, "approve"), decline: await reviewUrl(env.BETA_REVIEW_SECRET, origin, row.id, "decline") }
        : null;
      const alert = alertEmail({ ...contact, phones, computer, country: countryCode, isNewRequest, links }, env.EMAIL_FROM, env.LEAD_ALERT_EMAIL);
      ctx.waitUntil(send(env, "transactional", alert, { origin, nowMs }, fetcher));
    }
  }
  return done();
}

// ---- signed review links -----------------------------------------------------------------
// The signature is HMAC-SHA256 over "<id>.<action>" with BETA_REVIEW_SECRET. No expiry:
// the links only ever reach Mike's inbox, and a decided request can't be decided twice.

async function sign(secret: string, id: string, action: Action): Promise<string> {
  return b64url(await crypto.subtle.sign("HMAC", await hmacKey(secret), new TextEncoder().encode(`${id}.${action}`)));
}

async function verified(secret: string | undefined, id: unknown, action: unknown, sig: unknown): Promise<{ id: string; action: Action } | null> {
  if (!secret || typeof id !== "string" || typeof sig !== "string" || (action !== "approve" && action !== "decline")) return null;
  const bytes = fromB64url(sig);
  if (!bytes || !sig) return null;
  const ok = await crypto.subtle.verify("HMAC", await hmacKey(secret), bytes, new TextEncoder().encode(`${id}.${action}`));
  return ok ? { id, action } : null;
}

export async function reviewUrl(secret: string, origin: string, id: string, action: Action): Promise<string> {
  return `${origin}/beta/review?id=${encodeURIComponent(id)}&action=${action}&sig=${await sign(secret, id, action)}`;
}

// ---- GET and POST /beta/review -------------------------------------------------------------

export async function handleReview(request: Request, env: Env, fetcher: typeof fetch): Promise<Response> {
  const isPost = request.method === "POST";
  const fields = isPost
    ? Object.fromEntries(((await request.formData().catch(() => null)) ?? new FormData()).entries())
    : Object.fromEntries(new URL(request.url).searchParams.entries());
  const ok = await verified(env.BETA_REVIEW_SECRET, fields.id, fields.action, fields.sig);
  const row = ok ? await env.DB.prepare("SELECT * FROM beta_signups WHERE id = ?1").bind(ok.id).first<BetaRow>() : null;
  if (!ok || !row) return page(403, "Link not valid", "<p>This review link isn't valid. Use the Approve or Decline link from the alert email.</p>");

  if (!isPost) return page(200, ok.action === "approve" ? "Approve this request?" : "Decline this request?", confirmForm(row, ok.action, fields.sig as string));

  const now = new Date().toISOString();
  if (ok.action === "approve") return page(200, "Approve", await approve(env, row, now, new URL(request.url).origin, fetcher));

  const changed = await env.DB.prepare("UPDATE beta_signups SET status = 'declined', decided_at = ?2 WHERE id = ?1 AND status = 'pending'").bind(row.id, now).run();
  if (!changed.meta.changes) return page(200, "Decline", `<p>Nothing changed. This request is already ${esc(row.status)}.</p>`);
  if (fields.tell !== "1") return page(200, "Decline", "<p>Declined. Nothing was sent to them.</p>");
  const lead = await leadById(env.DB, row.lead_id);
  const sent =
    lead && env.EMAIL_FROM
      ? (await send(env, "transactional", declineEmail(row.email, language(lead.signup_language), firstName(lead.name), env.EMAIL_FROM, env.EMAIL_REPLY_TO), { leadId: lead.id, origin: new URL(request.url).origin, nowMs: Date.parse(now) }, fetcher)).sent
      : false;
  return page(200, "Decline", sent ? "<p>Declined. We sent them a short note.</p>" : "<p>Declined. The note to them did not send; email them yourself if you want them to know.</p>");
}

/** Marks the request approved and sends the install email once. When an iPhone is asked
 * for and TESTFLIGHT_PUBLIC_URL is empty, the email says the link follows and is not
 * marked sent, so approving again after setting the link sends the full email. */
async function approve(env: Env, row: BetaRow, now: string, origin: string, fetcher: typeof fetch): Promise<string> {
  await env.DB.prepare("UPDATE beta_signups SET status = 'approved', decided_at = IIF(status = 'approved', decided_at, ?2) WHERE id = ?1").bind(row.id, now).run();
  if (row.install_email_sent_at) return `<p>Already approved. The install email went out on ${esc(row.install_email_sent_at.slice(0, 10))}. Nothing more was sent.</p>`;
  const lead = await leadById(env.DB, row.lead_id);
  if (!lead || !env.EMAIL_FROM) return "<p>Approved, but email is off, so no install email was sent.</p>";

  const testflight = env.TESTFLIGHT_PUBLIC_URL || null;
  const complete = row.phones === "android" || testflight !== null;
  if (complete) {
    const claim = await env.DB.prepare("UPDATE beta_signups SET install_email_sent_at = ?2 WHERE id = ?1 AND install_email_sent_at IS NULL").bind(row.id, now).run();
    if (!claim.meta.changes) return "<p>Already approved. Nothing more was sent.</p>";
  }
  const email = installEmail(row.email, language(lead.signup_language), firstName(lead.name), row.phones, testflight, env.ANDROID_RELEASE_URL || DEFAULT_ANDROID_RELEASE_URL, env.EMAIL_FROM, env.EMAIL_REPLY_TO);
  const result = await send(env, "transactional", email, { leadId: lead.id, origin, nowMs: Date.parse(now) }, fetcher);
  if (!result.sent) {
    if (complete) await env.DB.prepare("UPDATE beta_signups SET install_email_sent_at = NULL WHERE id = ?1").bind(row.id).run();
    return "<p>Approved, but the install email did not send. Open the Approve link again to retry.</p>";
  }
  return complete
    ? "<p>Approved. The install email is on its way.</p>"
    : "<p>Approved. The email told them their TestFlight link comes in a follow-up. Set TESTFLIGHT_PUBLIC_URL, deploy, then approve again to send it.</p>";
}

const firstName = (name: string) => name.split(/\s+/)[0] || name;

function confirmForm(row: BetaRow, action: Action, sig: string): string {
  const details = `<table>${[
    ["Email", row.email],
    ["Phones", PHONE_LABEL[row.phones]],
    ["Computer", COMPUTER_LABEL[row.computer] ?? row.computer],
    ["Status", row.status],
    ["Asked", row.created_at.slice(0, 16).replace("T", " ") + " UTC"],
  ]
    .map(([k, v]) => `<tr><td>${k}</td><td>${esc(v)}</td></tr>`)
    .join("")}</table>`;
  const hidden = `<input type="hidden" name="id" value="${esc(row.id)}"><input type="hidden" name="action" value="${action}"><input type="hidden" name="sig" value="${esc(sig)}">`;
  const buttons =
    action === "approve"
      ? `<button type="submit">Approve and send the install email</button>`
      : `<button type="submit">Decline</button> <button type="submit" name="tell" value="1" class="ghost">Decline and tell them</button>`;
  return `${details}<form method="post" action="/beta/review">${hidden}${buttons}</form>`;
}

function page(status: number, title: string, body: string): Response {
  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><meta name="robots" content="noindex"><title>${esc(title)} | Whimbrel beta</title>
<style>body{margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19}main{max-width:560px;margin:0 auto;padding:32px 16px;line-height:1.5}td{padding:4px 12px 4px 0;vertical-align:top}td:first-child{color:#6B5F4F}form{margin-top:24px}button{font:inherit;font-weight:600;padding:12px 20px;border-radius:10px;border:0;background:#A8502E;color:#fff;cursor:pointer;margin:0 8px 8px 0}button.ghost{background:#fff;color:#241E19;border:1px solid #E4D9C2}</style></head>
<body><main><h1>${esc(title)}</h1>${body}</main></body></html>`;
  return new Response(html, { status, headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "x-robots-tag": "noindex" } });
}

// ---- email -------------------------------------------------------------------------------

/** The body wrapper `send` adds the preferences footer to. */
function wrap(paragraphs: string[]): string {
  return `<!doctype html><html><body style="margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19">
<div style="max-width:560px;margin:0 auto;padding:32px 20px">
${paragraphs.join("\n")}
</div></body></html>`;
}
const para = (html: string) => `<p style="font-size:16px;line-height:1.5">${html}</p>`;
const heading = (text: string) => `<h2 style="font-size:18px;margin:28px 0 8px">${esc(text)}</h2>`;
const steps = (items: string[]) => `<ol style="font-size:16px;line-height:1.5;padding-left:22px">${items.map((i) => `<li>${i}</li>`).join("")}</ol>`;
const link = (href: string, label: string) => `<a href="${esc(href)}" style="color:#A8502E">${esc(label)}</a>`;

const RECEIVED: Record<Language, { subject: string; hi: (f: string) => string; body: string; reads: string; sign: string }> = {
  en: {
    subject: "We got your Whimbrel beta request",
    hi: (f) => `Hi ${f},`,
    body: "Thanks for asking to try the Whimbrel phone app. We'll email you when your spot is ready.",
    reads: "The phone app shows the meetings Whimbrel records on your Mac or Windows PC, so you'll need Whimbrel running on one of them.",
    sign: "Mike",
  },
  es: {
    subject: "Recibimos tu solicitud para la beta de Whimbrel",
    hi: (f) => `Hola, ${f}:`,
    body: "Gracias por pedir probar la app de Whimbrel para el teléfono. Te escribiremos cuando tu lugar esté listo.",
    reads: "La app del teléfono muestra las reuniones que Whimbrel graba en tu Mac o PC con Windows, así que necesitarás Whimbrel funcionando en una de ellas.",
    sign: "Mike",
  },
  fr: {
    subject: "Nous avons reçu votre demande pour la bêta de Whimbrel",
    hi: (f) => `Bonjour ${f},`,
    body: "Merci de vouloir essayer l'application Whimbrel pour téléphone. Nous vous écrirons quand votre place sera prête.",
    reads: "L'application pour téléphone affiche les réunions que Whimbrel enregistre sur votre Mac ou votre PC Windows. Il vous faudra donc Whimbrel sur l'un des deux.",
    sign: "Mike",
  },
};

function receivedEmail(to: string, lang: Language, first: string, from: string, replyTo: string | undefined): Email {
  const c = RECEIVED[lang];
  const lines = [c.hi(first), c.body, c.reads, c.sign];
  return { from, to, replyTo, subject: c.subject, text: lines.join("\n\n"), html: wrap(lines.map((l) => para(esc(l)))) };
}

const DECLINE: Record<Language, { subject: string; hi: (f: string) => string; body: string; desktop: string; sign: string }> = {
  en: {
    subject: "About your Whimbrel beta request",
    hi: (f) => `Hi ${f},`,
    body: "Thanks for asking to join the Whimbrel phone beta. The beta is small, and we can't add you right now.",
    desktop: "Whimbrel for Mac and Windows is free to download at https://whimbrelapp.com/download/.",
    sign: "Mike",
  },
  es: {
    subject: "Sobre tu solicitud para la beta de Whimbrel",
    hi: (f) => `Hola, ${f}:`,
    body: "Gracias por pedir unirte a la beta de Whimbrel para el teléfono. La beta es pequeña y por ahora no podemos sumarte.",
    desktop: "Whimbrel para Mac y Windows se descarga gratis en https://whimbrelapp.com/es/download/.",
    sign: "Mike",
  },
  fr: {
    subject: "À propos de votre demande pour la bêta de Whimbrel",
    hi: (f) => `Bonjour ${f},`,
    body: "Merci de vouloir rejoindre la bêta de Whimbrel pour téléphone. La bêta est petite et nous ne pouvons pas vous ajouter pour le moment.",
    desktop: "Whimbrel pour Mac et Windows se télécharge gratuitement sur https://whimbrelapp.com/fr/download/.",
    sign: "Mike",
  },
};

function declineEmail(to: string, lang: Language, first: string, from: string, replyTo: string | undefined): Email {
  const c = DECLINE[lang];
  const lines = [c.hi(first), c.body, c.desktop, c.sign];
  return { from, to, replyTo, subject: c.subject, text: lines.join("\n\n"), html: wrap(lines.map((l) => para(esc(l)))) };
}

interface InstallCopy {
  subject: string;
  hi: (f: string) => string;
  ready: string;
  iphone: string;
  iphoneSteps: (link: string) => string[];
  iphoneLater: string;
  android: string;
  androidSteps: (link: string) => string[];
  updates: string;
  iphoneUpdates: string;
  androidUpdates: string;
  problems: (mike: string) => string;
  sign: string;
}

const INSTALL: Record<Language, InstallCopy> = {
  en: {
    subject: "Your Whimbrel phone beta is ready",
    hi: (f) => `Hi ${f},`,
    ready: "Your spot in the Whimbrel phone beta is ready. The phone app shows the meetings Whimbrel records on your Mac or Windows PC, so keep Whimbrel running there.",
    iphone: "iPhone",
    iphoneSteps: (l) => [
      "Install TestFlight from the App Store.",
      `On your iPhone, open this link: ${l}`,
      "Install Whimbrel from TestFlight and open it.",
      "Sign in with the Google account you use in Whimbrel on your computer.",
    ],
    iphoneLater: "Your TestFlight link comes in a follow-up email. While you wait, install TestFlight from the App Store.",
    android: "Android",
    androidSteps: (l) => [
      `On your Android phone, open the release page: ${l}`,
      "Under Assets, tap the Whimbrel .apk file to download it.",
      "Open the file. If Android asks, allow installs from your browser, then install.",
      "Open Whimbrel and sign in with the Google account you use in Whimbrel on your computer.",
      "Approve the phone on your computer when Whimbrel asks.",
    ],
    updates: "Staying up to date",
    iphoneUpdates: "On iPhone, TestFlight installs new versions for you.",
    androidUpdates: "On Android, Whimbrel shows a notice when a new version is out. Tap it and install the new file the same way.",
    problems: (m) => `If something doesn't work, reply to this email or write to ${m}.`,
    sign: "Mike",
  },
  es: {
    subject: "Tu beta de Whimbrel para el teléfono está lista",
    hi: (f) => `Hola, ${f}:`,
    ready: "Tu lugar en la beta de Whimbrel para el teléfono está listo. La app del teléfono muestra las reuniones que Whimbrel graba en tu Mac o PC con Windows, así que mantén Whimbrel funcionando ahí.",
    iphone: "iPhone",
    iphoneSteps: (l) => [
      "Instala TestFlight desde el App Store.",
      `En tu iPhone, abre este enlace: ${l}`,
      "Instala Whimbrel desde TestFlight y ábrelo.",
      "Inicia sesión con la cuenta de Google que usas en Whimbrel en tu computadora.",
    ],
    iphoneLater: "Tu enlace de TestFlight llegará en otro correo. Mientras tanto, instala TestFlight desde el App Store.",
    android: "Android",
    androidSteps: (l) => [
      `En tu teléfono Android, abre la página de la versión: ${l}`,
      "En Assets, toca el archivo .apk de Whimbrel para descargarlo.",
      "Abre el archivo. Si Android lo pide, permite instalar desde tu navegador y luego instala.",
      "Abre Whimbrel e inicia sesión con la cuenta de Google que usas en Whimbrel en tu computadora.",
      "Aprueba el teléfono en tu computadora cuando Whimbrel lo pida.",
    ],
    updates: "Cómo mantenerla al día",
    iphoneUpdates: "En iPhone, TestFlight instala las versiones nuevas por ti.",
    androidUpdates: "En Android, Whimbrel muestra un aviso cuando sale una versión nueva. Tócalo e instala el archivo nuevo de la misma forma.",
    problems: (m) => `Si algo no funciona, responde a este correo o escribe a ${m}.`,
    sign: "Mike",
  },
  fr: {
    subject: "Votre bêta de Whimbrel pour téléphone est prête",
    hi: (f) => `Bonjour ${f},`,
    ready: "Votre place dans la bêta de Whimbrel pour téléphone est prête. L'application pour téléphone affiche les réunions que Whimbrel enregistre sur votre Mac ou votre PC Windows. Gardez donc Whimbrel ouvert sur votre ordinateur.",
    iphone: "iPhone",
    iphoneSteps: (l) => [
      "Installez TestFlight depuis l'App Store.",
      `Sur votre iPhone, ouvrez ce lien : ${l}`,
      "Installez Whimbrel depuis TestFlight et ouvrez-le.",
      "Connectez-vous avec le compte Google que vous utilisez dans Whimbrel sur votre ordinateur.",
    ],
    iphoneLater: "Votre lien TestFlight arrivera dans un autre courriel. En attendant, installez TestFlight depuis l'App Store.",
    android: "Android",
    androidSteps: (l) => [
      `Sur votre téléphone Android, ouvrez la page de la version : ${l}`,
      "Sous Assets, touchez le fichier .apk de Whimbrel pour le télécharger.",
      "Ouvrez le fichier. Si Android le demande, autorisez les installations depuis votre navigateur, puis installez.",
      "Ouvrez Whimbrel et connectez-vous avec le compte Google que vous utilisez dans Whimbrel sur votre ordinateur.",
      "Approuvez le téléphone sur votre ordinateur quand Whimbrel le demande.",
    ],
    updates: "Rester à jour",
    iphoneUpdates: "Sur iPhone, TestFlight installe les nouvelles versions pour vous.",
    androidUpdates: "Sur Android, Whimbrel affiche un avis quand une nouvelle version sort. Touchez-le et installez le nouveau fichier de la même façon.",
    problems: (m) => `Si quelque chose ne fonctionne pas, répondez à ce courriel ou écrivez à ${m}.`,
    sign: "Mike",
  },
};

/** Only the sections for the phones asked for. */
export function installEmail(
  to: string,
  lang: Language,
  first: string,
  phones: Phones,
  testflight: string | null,
  androidUrl: string,
  from: string,
  replyTo: string | undefined,
): Email {
  const c = INSTALL[lang];
  const iphone = phones !== "android";
  const android = phones !== "iphone";
  const mike = replyTo ?? "hello@whimbrelapp.com";
  const text: string[] = [c.hi(first), c.ready];
  const html: string[] = [para(esc(c.hi(first))), para(esc(c.ready))];
  if (iphone) {
    text.push(c.iphone.toUpperCase());
    html.push(heading(c.iphone));
    if (testflight) {
      text.push(c.iphoneSteps(testflight).map((s, i) => `${i + 1}. ${s}`).join("\n"));
      html.push(steps(c.iphoneSteps("\u0000").map((s) => esc(s).replace("\u0000", link(testflight, testflight)))));
    } else {
      text.push(c.iphoneLater);
      html.push(para(esc(c.iphoneLater)));
    }
  }
  if (android) {
    text.push(c.android.toUpperCase(), c.androidSteps(androidUrl).map((s, i) => `${i + 1}. ${s}`).join("\n"));
    html.push(heading(c.android), steps(c.androidSteps("\u0000").map((s) => esc(s).replace("\u0000", link(androidUrl, androidUrl)))));
  }
  const updates = [iphone ? c.iphoneUpdates : null, android ? c.androidUpdates : null].filter((s): s is string => s !== null).join(" ");
  text.push(c.updates.toUpperCase(), updates, c.problems(mike), c.sign);
  html.push(heading(c.updates), para(esc(updates)), para(esc(c.problems(mike))), para(esc(c.sign)));
  return { from, to, replyTo, subject: c.subject, text: text.join("\n\n"), html: wrap(html) };
}

function alertEmail(
  r: { name: string; email: string; phones: Phones; computer: string; country: string | null; marketingOptIn: boolean; signupLanguage: string; isNewRequest: boolean; links: { approve: string; decline: string } | null },
  from: string,
  to: string,
): Email {
  const rows: [string, string][] = [
    ["Name", r.name],
    ["Email", r.email],
    ["Phones", PHONE_LABEL[r.phones]],
    ["Computer", COMPUTER_LABEL[r.computer] ?? r.computer],
    ["Country", r.country ?? "unknown"],
    ["Language", r.signupLanguage],
    ["Product emails", r.marketingOptIn ? "asked (confirmation sent)" : "no"],
  ];
  const subject = `${r.isNewRequest ? "Beta request" : "Updated beta request"}: ${r.name}, ${PHONE_LABEL[r.phones]}`;
  const noLinks = "Review links are unavailable: the BETA_REVIEW_SECRET secret is not set.";
  const text = [
    rows.map(([k, v]) => `${k}: ${v}`).join("\n"),
    r.links ? `Approve: ${r.links.approve}\nDecline: ${r.links.decline}` : noLinks,
  ].join("\n\n");
  const table = `<table style="font-family:-apple-system,'Segoe UI',sans-serif;font-size:14px;border-collapse:collapse">${rows
    .map(([k, v]) => `<tr><td style="padding:4px 12px 4px 0;color:#6B5F4F">${k}</td><td style="padding:4px 0">${esc(v)}</td></tr>`)
    .join("")}</table>`;
  const actions = r.links
    ? `<p style="font-family:-apple-system,'Segoe UI',sans-serif;font-size:14px"><a href="${esc(r.links.approve)}">Approve</a> &nbsp; <a href="${esc(r.links.decline)}">Decline</a></p>`
    : `<p style="font-family:-apple-system,'Segoe UI',sans-serif;font-size:14px">${noLinks}</p>`;
  return { from, to, replyTo: r.email, subject, text, html: table + actions };
}
