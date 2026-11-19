// Debug only. gallery.html is not part of the default build (vite.config.ts builds it only in `--mode gallery`),
// so nothing in this folder reaches a release. Dev: npm run dev, then open /gallery.html.
import { createRoot } from "react-dom/client";
import "../../tokens.css";
import { Gallery } from "./Gallery";

createRoot(document.getElementById("root")!).render(<Gallery />);
