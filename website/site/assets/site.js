// Progressive enhancement only: every page and the registration form work without this.
(function () {
  "use strict";

  var params = new URLSearchParams(location.search);

  var header = document.querySelector(".site-header");
  var toggle = document.querySelector(".nav-toggle");
  if (header && toggle) {
    toggle.addEventListener("click", function () {
      var open = !header.hasAttribute("data-open");
      header.toggleAttribute("data-open", open);
      toggle.setAttribute("aria-expanded", String(open));
    });
  }

  function store(get, key, value) {
    try {
      if (get) return localStorage.getItem(key);
      localStorage.setItem(key, value);
    } catch (e) {}
    return null;
  }

  // First-touch attribution: remember where this visitor originally came from, so a
  // registration on a later page view is still credited to the campaign that brought them.
  var ATTR_KEY = "whimbrel_attribution";
  var attribution = null;
  try { attribution = JSON.parse(store(true, ATTR_KEY) || "null"); } catch (e) {}
  if (!attribution) {
    var ref = document.referrer && new URL(document.referrer).host !== location.host ? document.referrer : "";
    attribution = {
      utm_source: params.get("utm_source") || "",
      utm_medium: params.get("utm_medium") || "",
      utm_campaign: params.get("utm_campaign") || "",
      referrer: ref,
      landing_page: location.pathname + location.search,
    };
    store(false, ATTR_KEY, JSON.stringify(attribution));
  }

  function detectPlatform() {
    var p = params.get("platform");
    if (p === "mac" || p === "windows") return p;
    var ua = (navigator.userAgentData && navigator.userAgentData.platform) || navigator.userAgent || "";
    if (/win/i.test(ua)) return "windows";
    if (/mac/i.test(ua)) return "mac";
    return null;
  }
  var platform = detectPlatform();

  // Put the visitor's own platform first wherever both are offered.
  if (platform === "windows") {
    document.querySelectorAll(".cta-actions, .hero-actions, .download-buttons").forEach(function (group) {
      var win = group.querySelector('[data-platform-link="windows"]');
      var mac = group.querySelector('[data-platform-link="mac"]');
      if (win && mac) {
        win.classList.replace("btn-ghost", "btn-primary");
        mac.classList.replace("btn-primary", "btn-ghost");
        group.insertBefore(win, mac);
      }
    });
  }

  // Current version labels.
  var versionEls = document.querySelectorAll("[data-version]");
  if (versionEls.length) {
    fetch("/api/releases").then(function (r) { return r.ok ? r.json() : null; }).then(function (data) {
      if (!data) return;
      versionEls.forEach(function (el) {
        var rel = data[el.getAttribute("data-version")];
        if (rel && rel.version) el.textContent = "Version " + rel.version;
      });
    }).catch(function () {});
  }

  // Thank-you page: start the download for the chosen platform automatically.
  var autoDownload = document.querySelector("[data-auto-download]");
  if (autoDownload && platform) {
    document.querySelectorAll("[data-platform-name]").forEach(function (el) {
      el.textContent = platform === "mac" ? "Mac" : "Windows";
    });
    setTimeout(function () { location.href = "/api/download/" + platform; }, 900);
  }

  // The form's messages on the translated sites (site/pages/es, site/pages/fr).
  var lang = document.documentElement.lang;
  var localePrefix = lang === "es" ? "/es" : lang === "fr-CA" ? "/fr" : "";
  var messages = {
    es: {
      "Enter your name.": "Escribe tu nombre.",
      "Enter a valid email address.": "Escribe un correo electrónico válido.",
      "Choose Mac or Windows.": "Elige Mac o Windows.",
      "Registering…": "Registrando…",
      "Something went wrong. Please try again.": "Algo salió mal. Inténtalo de nuevo.",
      "Network error. Check your connection and try again.": "Error de red. Revisa tu conexión e inténtalo de nuevo.",
    },
    "fr-CA": {
      "Enter your name.": "Entrez votre nom.",
      "Enter a valid email address.": "Entrez une adresse courriel valide.",
      "Choose Mac or Windows.": "Choisissez Mac ou Windows.",
      "Registering…": "Inscription…",
      "Something went wrong. Please try again.": "Une erreur s’est produite. Veuillez réessayer.",
      "Network error. Check your connection and try again.": "Erreur réseau. Vérifiez votre connexion et réessayez.",
    },
  };
  function text(s) { return (messages[lang] && messages[lang][s]) || s; }

  var form = document.getElementById("register-form");
  if (!form) return;

  Object.keys(attribution).forEach(function (k) {
    var input = form.elements.namedItem(k);
    if (input) input.value = attribution[k];
  });

  if (platform) {
    var radio = form.querySelector('input[name="platform"][value="' + platform + '"]');
    if (radio) radio.checked = true;
  }

  var alertBox = form.querySelector(".form-alert");
  if (params.get("error")) {
    alertBox.textContent = "Please check the form and try again.";
    alertBox.hidden = false;
  }

  var siteKey = form.getAttribute("data-turnstile-sitekey");
  if (siteKey) {
    var holder = form.querySelector(".turnstile");
    holder.className = "turnstile cf-turnstile";
    holder.setAttribute("data-sitekey", siteKey);
    var s = document.createElement("script");
    s.src = "https://challenges.cloudflare.com/turnstile/v0/api.js";
    s.async = true;
    document.head.appendChild(s);
  }

  function showErrors(errors) {
    form.querySelectorAll(".field-error").forEach(function (el) { el.textContent = ""; });
    form.querySelectorAll("[aria-invalid]").forEach(function (el) { el.removeAttribute("aria-invalid"); });
    alertBox.hidden = true;
    var first = null;
    Object.keys(errors).forEach(function (key) {
      var slot = form.querySelector('.field-error[data-for="' + key + '"]');
      var input = form.elements.namedItem(key);
      if (slot) slot.textContent = errors[key];
      else { alertBox.textContent = errors[key]; alertBox.hidden = false; }
      if (input && input.setAttribute) { input.setAttribute("aria-invalid", "true"); first = first || input; }
      else if (input && input[0]) { first = first || input[0]; }
    });
    if (first && first.focus) first.focus();
  }

  function validate(data) {
    var errors = {};
    if (!String(data.name || "").trim()) errors.name = text("Enter your name.");
    if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(String(data.email || "").trim())) errors.email = text("Enter a valid email address.");
    if (data.platform !== "mac" && data.platform !== "windows") errors.platform = text("Choose Mac or Windows.");
    return errors;
  }

  form.addEventListener("submit", function (event) {
    event.preventDefault();
    var data = {};
    new FormData(form).forEach(function (v, k) { data[k] = v; });
    data.signup_page = location.pathname;
    var errors = validate(data);
    if (Object.keys(errors).length) { showErrors(errors); return; }
    showErrors({});

    var button = form.querySelector('button[type="submit"]');
    var label = button.textContent;
    button.setAttribute("aria-busy", "true");
    button.textContent = text("Registering…");

    fetch("/api/register", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(data),
      credentials: "same-origin",
    })
      .then(function (r) { return r.json().catch(function () { return { ok: false, errors: { form: text("Something went wrong. Please try again.") } }; }); })
      .then(function (res) {
        if (res.ok) {
          if (window.gtag) window.gtag("event", "generate_lead", { platform: data.platform });
          location.href = localePrefix + res.next;
          return;
        }
        showErrors(res.errors || { form: text("Something went wrong. Please try again.") });
        if (window.turnstile) window.turnstile.reset();
      })
      .catch(function () { showErrors({ form: text("Network error. Check your connection and try again.") }); })
      .finally(function () {
        button.removeAttribute("aria-busy");
        button.textContent = label;
      });
  });
})();
