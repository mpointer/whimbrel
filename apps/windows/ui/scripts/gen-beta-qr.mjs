// Writes src/assets/mobile-beta-qr.svg: the phone beta page as a QR code for Settings → Sync & Devices'
// phone card (the Mac draws the same one with CoreImage). The URL never changes, so the SVG is generated
// once and committed; the app ships no QR library. Run: node scripts/gen-beta-qr.mjs
import { writeFileSync } from "node:fs";
import QRCode from "qrcode";

const BETA_URL = "https://whimbrelapp.com/mobile-beta/";
// Dark modules on white with a two-module quiet zone, so it scans on a dark theme too.
const svg = await QRCode.toString(BETA_URL, { type: "svg", errorCorrectionLevel: "M", margin: 2, color: { dark: "#000000", light: "#ffffff" } });
writeFileSync(new URL("../src/assets/mobile-beta-qr.svg", import.meta.url), svg);
console.log("wrote src/assets/mobile-beta-qr.svg");
