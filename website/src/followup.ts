// The personal next-day note (design §4) and the hourly cron that sends it and keeps the
// Resend contacts in step with the stored answers.
import { downloadLinks, send, type Email, type MailEnv } from "./email";
import { language, markSynced, syncContact, type Language, type LeadRow } from "./preferences";

export interface CronEnv extends MailEnv {
  EMAIL_FROM?: string;
  EMAIL_REPLY_TO?: string;
  CANONICAL_HOST?: string;
  RESEND_PRODUCT_TOPIC_ID?: string;
}

/** Sent 20 to 72 hours after the first registration. Wider than a day so a missed or
 * failed run is retried; older leads are left alone rather than greeted late. */
export const FOLLOWUP_MIN_HOURS = 20;
export const FOLLOWUP_MAX_HOURS = 72;
const BATCH = 50;

interface Copy {
  subject: string;
  hi: (first: string) => string;
  thanks: string;
  linkAgain: string;
  mac: string;
  windows: string;
  updates: string;
  quickstartIntro: string;
  steps: (platform: string) => string[];
  more: string;
  promiseIn: string;
  promiseOut: string;
  promisePending: string;
  reply: string;
}

const COPY: Record<Language, Copy> = {
  en: {
    subject: "Whimbrel: thanks for trying it",
    hi: (f) => `Hi ${f},`,
    thanks: `Thanks for registering for Whimbrel yesterday. Your download links came in an email titled "Your Whimbrel download links", and here's yours again:`,
    linkAgain: "Download Whimbrel",
    mac: "It needs a Mac with Apple silicon and macOS 14 or later.",
    windows: "The Windows build is early access and needs Windows 11, 64-bit. If SmartScreen warns that the installer is unrecognized, choose More info, then Run anyway.",
    updates: "Once it's installed you won't need to download it again: Whimbrel checks for updates and installs them itself.",
    quickstartIntro: "Getting started takes a few minutes:",
    steps: (p) => [
      p === "mac" ? "Allow microphone and screen recording when macOS asks. That's how Whimbrel hears your meetings and remembers what was on screen." : "Allow microphone access if Windows asks. That's how Whimbrel hears your meetings.",
      "Connect your calendar in Whimbrel's Settings, so your meetings start recording on their own.",
      "After a meeting, open it to read the summary, the decisions and the action items.",
      'Ask Whimbrel anything about it, like "What did we agree about pricing?"',
    ],
    more: "More on how meetings work",
    promiseIn: "That's the last of these setup notes. You asked for product news when you registered, so you'll get the occasional release note, never a flood, and you can turn it off below.",
    promiseOut: "That's it from us. You won't get product or marketing emails, because you didn't ask for them when you registered. If you'd like release notes, you can turn them on below.",
    promisePending: "That's the last of these setup notes. You asked for product news when you registered; it starts once you confirm with the link we emailed you. It'll never be a flood, and you can change it below.",
    reply: "I'd really like to hear how the first days go, what worked and what didn't. Just reply to this email.",
  },
  es: {
    subject: "Whimbrel: gracias por probarlo",
    hi: (f) => `Hola, ${f}:`,
    thanks: "Gracias por registrarte ayer en Whimbrel. Tus enlaces de descarga llegaron en un correo titulado «Your Whimbrel download links», y aquí tienes el tuyo otra vez:",
    linkAgain: "Descargar Whimbrel",
    mac: "Necesita una Mac con Apple silicon y macOS 14 o posterior.",
    windows: "La versión para Windows es de acceso anticipado y necesita Windows 11 de 64 bits. Si SmartScreen avisa que el instalador no es reconocido, elige Más información y luego Ejecutar de todas formas.",
    updates: "Una vez instalado, no tendrás que volver a descargarlo: Whimbrel busca actualizaciones y las instala solo.",
    quickstartIntro: "Empezar toma unos minutos:",
    steps: (p) => [
      p === "mac" ? "Permite el acceso al micrófono y a la grabación de pantalla cuando macOS lo pida. Así Whimbrel escucha tus reuniones y recuerda lo que había en pantalla." : "Permite el acceso al micrófono si Windows lo pide. Así Whimbrel escucha tus reuniones.",
      "Conecta tu calendario en los Ajustes de Whimbrel para que tus reuniones se graben solas.",
      "Después de una reunión, ábrela para leer el resumen, las decisiones y las tareas pendientes.",
      "Pregúntale a Whimbrel lo que quieras sobre ella, por ejemplo: «¿Qué acordamos sobre el precio?»",
    ],
    more: "Más sobre cómo funcionan las reuniones",
    promiseIn: "Esta es la última de estas notas de inicio. Pediste noticias del producto al registrarte, así que recibirás alguna nota de versión de vez en cuando, nunca una avalancha, y puedes desactivarlas abajo.",
    promiseOut: "Eso es todo de nuestra parte. No recibirás correos de producto ni de marketing, porque no los pediste al registrarte. Si quieres recibir las notas de versión, puedes activarlas abajo.",
    promisePending: "Esta es la última de estas notas de inicio. Pediste noticias del producto al registrarte; empezarán cuando lo confirmes con el enlace que te enviamos. Nunca será una avalancha, y puedes cambiarlo abajo.",
    reply: "Me encantaría saber cómo te va en los primeros días, qué funcionó y qué no. Solo responde a este correo.",
  },
  fr: {
    subject: "Whimbrel : merci de l'essayer",
    hi: (f) => `Bonjour ${f},`,
    thanks: "Merci de vous être inscrit à Whimbrel hier. Vos liens de téléchargement sont arrivés dans un courriel intitulé « Your Whimbrel download links ». Le voici de nouveau :",
    linkAgain: "Télécharger Whimbrel",
    mac: "Il faut un Mac avec puce Apple et macOS 14 ou une version ultérieure.",
    windows: "La version Windows est en accès anticipé et nécessite Windows 11, 64 bits. Si SmartScreen indique que le programme d'installation n'est pas reconnu, choisissez Informations complémentaires, puis Exécuter quand même.",
    updates: "Une fois l'app installée, vous n'aurez plus à la télécharger : Whimbrel recherche les mises à jour et les installe lui-même.",
    quickstartIntro: "La prise en main prend quelques minutes :",
    steps: (p) => [
      p === "mac" ? "Autorisez l'accès au micro et l'enregistrement de l'écran quand macOS le demande. C'est ainsi que Whimbrel entend vos réunions et se souvient de ce qui était à l'écran." : "Autorisez l'accès au micro si Windows le demande. C'est ainsi que Whimbrel entend vos réunions.",
      "Connectez votre calendrier dans les Réglages de Whimbrel pour que vos réunions s'enregistrent d'elles-mêmes.",
      "Après une réunion, ouvrez-la pour lire le résumé, les décisions et les tâches à faire.",
      "Posez à Whimbrel n'importe quelle question à son sujet, par exemple : « Qu'a-t-on décidé pour le prix ? »",
    ],
    more: "En savoir plus sur les réunions",
    promiseIn: "C'est la dernière de ces notes de démarrage. Vous avez demandé les nouvelles du produit à l'inscription : vous recevrez à l'occasion une note de version, jamais un déluge, et vous pouvez les désactiver ci-dessous.",
    promiseOut: "C'est tout de notre part. Vous ne recevrez pas de courriels de produit ni de marketing, puisque vous ne les avez pas demandés à l'inscription. Si vous souhaitez recevoir les notes de version, vous pouvez les activer ci-dessous.",
    promisePending: "C'est la dernière de ces notes de démarrage. Vous avez demandé les nouvelles du produit à l'inscription; elles commenceront quand vous aurez confirmé avec le lien que nous vous avons envoyé. Jamais un déluge, et vous pouvez changer cela ci-dessous.",
    reply: "J'aimerais beaucoup savoir comment se passent vos premiers jours, ce qui a fonctionné et ce qui n'a pas fonctionné. Répondez simplement à ce courriel.",
  },
};

const escapeHtml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export function followupEmail(origin: string, lead: LeadRow, from: string, replyTo: string | undefined): Email {
  const lang = language(lead.signup_language);
  const c = COPY[lang];
  const first = lead.name.split(/\s+/)[0] || lead.name;
  const link = downloadLinks(origin, lead.id)[lead.platform === "windows" ? "windows" : "mac"];
  const platformNote = lead.platform === "windows" ? c.windows : c.mac;
  const steps = c.steps(lead.platform);
  const more = `${origin}${lang === "en" ? "" : `/${lang}`}/features/meeting-notes/`;
  const promise = lead.product_emails === "opted_in" ? c.promiseIn : lead.product_emails_confirm_sent_at ? c.promisePending : c.promiseOut;
  const text = [
    c.hi(first),
    "",
    `${c.thanks} ${link}`,
    "",
    `${platformNote} ${c.updates}`,
    "",
    c.quickstartIntro,
    ...steps.map((s, i) => `${i + 1}. ${s}`),
    `${c.more}: ${more}`,
    "",
    promise,
    "",
    c.reply,
    "",
    "Mike",
    "Whimbrel",
  ].join("\n");
  const p = (s: string) => `<p style="font-size:16px;line-height:1.5">${s}</p>`;
  const html = `<!doctype html><html><body style="margin:0;background:#F7F2E7;font-family:-apple-system,'Segoe UI',sans-serif;color:#241E19">
<div style="max-width:560px;margin:0 auto;padding:32px 20px">
${p(escapeHtml(c.hi(first)))}
${p(`${escapeHtml(c.thanks)} <a href="${escapeHtml(link)}" style="color:#A8502E;font-weight:600">${escapeHtml(c.linkAgain)}</a>`)}
${p(`${escapeHtml(platformNote)} ${escapeHtml(c.updates)}`)}
${p(escapeHtml(c.quickstartIntro))}
<ol style="font-size:16px;line-height:1.5;padding-left:22px">${steps.map((s) => `<li style="margin-bottom:6px">${escapeHtml(s)}</li>`).join("")}</ol>
${p(`<a href="${escapeHtml(more)}" style="color:#A8502E">${escapeHtml(c.more)}</a>`)}
${p(escapeHtml(promise))}
${p(escapeHtml(c.reply))}
${p("Mike<br>Whimbrel")}
</div></body></html>`;
  return { from, to: lead.email, replyTo, subject: c.subject, text, html };
}

/** The hourly cron: follow-ups due, then Resend contacts out of step. */
export async function runScheduled(env: CronEnv, nowMs: number, fetcher: typeof fetch = fetch): Promise<{ followups: number; synced: number }> {
  const origin = `https://${env.CANONICAL_HOST ?? "whimbrelapp.com"}`;
  const hoursAgo = (h: number) => new Date(nowMs - h * 3600_000).toISOString();
  let followups = 0;
  if (env.RESEND_API_KEY && env.EMAIL_FROM) {
    const due = await env.DB.prepare(
      `SELECT * FROM leads WHERE followup_sent_at IS NULL AND created_at <= ?1 AND created_at >= ?2 ORDER BY created_at LIMIT ${BATCH}`,
    )
      .bind(hoursAgo(FOLLOWUP_MIN_HOURS), hoursAgo(FOLLOWUP_MAX_HOURS))
      .all<LeadRow>();
    for (const lead of due.results) {
      const result = await send(env, "transactional", followupEmail(origin, lead, env.EMAIL_FROM, env.EMAIL_REPLY_TO), { leadId: lead.id, origin, nowMs }, fetcher);
      if (!result.sent) continue; // retried next hour while still in the window
      await env.DB.prepare("UPDATE leads SET followup_sent_at = ?2 WHERE id = ?1 AND followup_sent_at IS NULL")
        .bind(lead.id, new Date(nowMs).toISOString())
        .run();
      followups++;
    }
  }
  let synced = 0;
  if (env.RESEND_API_KEY && env.RESEND_PRODUCT_TOPIC_ID) {
    const stale = await env.DB.prepare(
      `SELECT * FROM leads WHERE resend_synced_at IS NULL OR resend_synced_at < product_emails_set_at ORDER BY created_at LIMIT ${BATCH}`,
    ).all<LeadRow>();
    for (const lead of stale.results) {
      if (!(await syncContact(env.RESEND_API_KEY, env.RESEND_PRODUCT_TOPIC_ID, lead, fetcher))) continue;
      await markSynced(env.DB, lead.id, new Date(nowMs).toISOString());
      synced++;
    }
  }
  return { followups, synced };
}
