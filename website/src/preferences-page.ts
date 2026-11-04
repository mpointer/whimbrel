// /email/preferences/: shows a lead's product-email answer and switches it, from a signed
// link, with no login (design §3). Served by the Worker in the lead's signup language.
import type { Language, ProductEmails } from "./preferences";

type State =
  | { kind: "show"; value: ProductEmails; token: string; saved: boolean; confirm: boolean }
  | { kind: "expired" }
  | { kind: "link_sent" };

interface Copy {
  title: string;
  confirm: string;
  on: string;
  off: string;
  turnOn: string;
  turnOff: string;
  saved: string;
  transactional: string;
  expired: string;
  emailLabel: string;
  sendLink: string;
  linkSent: string;
}

const COPY: Record<Language, Copy> = {
  en: {
    title: "Email preferences",
    confirm: "You asked for Whimbrel release notes and product news. Confirm below to start getting them.",
    on: "Product emails are on: you'll get the occasional release note and product news.",
    off: "Product emails are off: you won't get release notes or product news.",
    turnOn: "Turn product emails on",
    turnOff: "Turn product emails off",
    saved: "Saved.",
    transactional: "Either way, you'll still get mail you need, like your download links and replies from Mike.",
    expired: "This link has expired or isn't valid. Enter your email address and we'll send you a new one.",
    emailLabel: "Email address",
    sendLink: "Send me a new link",
    linkSent: "If that address is registered, a new link is on its way.",
  },
  es: {
    title: "Preferencias de correo",
    confirm: "Pediste recibir las notas de versión y las novedades de Whimbrel. Confírmalo abajo para empezar a recibirlas.",
    on: "Los correos de producto están activados: recibirás de vez en cuando notas de versión y novedades del producto.",
    off: "Los correos de producto están desactivados: no recibirás notas de versión ni novedades del producto.",
    turnOn: "Activar los correos de producto",
    turnOff: "Desactivar los correos de producto",
    saved: "Guardado.",
    transactional: "En cualquier caso, seguirás recibiendo los correos que necesitas, como tus enlaces de descarga y las respuestas de Mike.",
    expired: "Este enlace venció o no es válido. Escribe tu correo y te enviaremos uno nuevo.",
    emailLabel: "Correo electrónico",
    sendLink: "Enviarme un enlace nuevo",
    linkSent: "Si ese correo está registrado, te enviamos un enlace nuevo.",
  },
  fr: {
    title: "Préférences de courriel",
    confirm: "Vous avez demandé les notes de version et les nouvelles de Whimbrel. Confirmez ci-dessous pour commencer à les recevoir.",
    on: "Les courriels sur le produit sont activés : vous recevrez à l'occasion des notes de version et des nouvelles du produit.",
    off: "Les courriels sur le produit sont désactivés : vous ne recevrez ni notes de version ni nouvelles du produit.",
    turnOn: "Activer les courriels sur le produit",
    turnOff: "Désactiver les courriels sur le produit",
    saved: "Enregistré.",
    transactional: "Dans tous les cas, vous recevrez toujours les courriels utiles, comme vos liens de téléchargement et les réponses de Mike.",
    expired: "Ce lien a expiré ou n'est pas valide. Entrez votre adresse courriel et nous vous en enverrons un nouveau.",
    emailLabel: "Adresse courriel",
    sendLink: "M'envoyer un nouveau lien",
    linkSent: "Si cette adresse est inscrite, un nouveau lien est en route.",
  },
};

const escapeHtml = (s: string) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

export function preferencesPage(lang: Language, state: State): Response {
  const c = COPY[lang];
  let body: string;
  if (state.kind === "show") {
    const next = state.value === "opted_in" ? "opted_out" : "opted_in";
    body = `${state.saved ? `<p class="saved" role="status">${c.saved}</p>` : ""}${state.confirm && state.value === "opted_out" ? `<p><strong>${c.confirm}</strong></p>` : ""}
<p id="state">${state.value === "opted_in" ? c.on : c.off}</p>
<form method="post" action="/email/preferences/">
<input type="hidden" name="t" value="${escapeHtml(state.token)}">
<input type="hidden" name="value" value="${next}">
<button type="submit">${next === "opted_in" ? c.turnOn : c.turnOff}</button>
</form>
<p class="muted">${c.transactional}</p>`;
  } else if (state.kind === "expired") {
    body = `<p>${c.expired}</p>
<form method="post" action="/email/preferences/">
<input type="hidden" name="lang" value="${lang}">
<label>${c.emailLabel} <input type="email" name="email" required autocomplete="email"></label>
<button type="submit">${c.sendLink}</button>
</form>`;
  } else {
    body = `<p role="status">${c.linkSent}</p>`;
  }
  const html = `<!doctype html><html lang="${lang === "fr" ? "fr-CA" : lang}"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex"><title>${c.title} · Whimbrel</title>
<style>body{margin:0;background:#F7F2E7;color:#241E19;font:16px/1.5 -apple-system,'Segoe UI',sans-serif}main{max-width:520px;margin:0 auto;padding:48px 16px}h1{font-size:24px}button{background:#A8502E;color:#fff;border:0;border-radius:10px;padding:12px 20px;font:inherit;font-weight:600;cursor:pointer}input[type=email]{display:block;width:100%;box-sizing:border-box;margin:6px 0 12px;padding:10px;border:1px solid #E4D9C2;border-radius:8px;font:inherit}.muted{color:#6B5F4F;font-size:14px}.saved{color:#2F6B3A;font-weight:600}a{color:#A8502E}</style></head>
<body><main><p><a href="/">Whimbrel</a></p><h1>${c.title}</h1>
${body}
</main></body></html>`;
  return new Response(html, { headers: { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "x-robots-tag": "noindex" } });
}
