// Run: VITE_PREVIEW=1 bun run build -- --base=./ && bun scripts/build-preview.mjs
import { readFileSync, writeFileSync, existsSync } from "node:fs";
import path from "node:path";

const dist = "dist";
let html = readFileSync(path.join(dist, "index.html"), "utf8");

const cssHref = /<link rel="stylesheet" crossorigin href="([^"]+)">/.exec(html)?.[1];
const jsSrc = /<script type="module" crossorigin src="([^"]+)"><\/script>/.exec(html)?.[1];
if (!cssHref || !jsSrc) throw new Error("could not find the built css/js in index.html");

const rel = (p) => path.join(dist, p.replace(/^\.?\//, ""));

let css = readFileSync(rel(cssHref), "utf8");

css = css.replace(/url\(([^)]+)\)/g, (whole, raw) => {
  const url = raw.replace(/["']/g, "").split("?")[0];
  if (!url.includes("assets/")) return whole;
  const file = rel(url);
  if (!existsSync(file)) return whole;
  const ext = path.extname(file).slice(1);
  const mime = ext === "woff2" ? "font/woff2" : ext === "woff" ? "font/woff" : `image/${ext}`;
  return `url(data:${mime};base64,${readFileSync(file).toString("base64")})`;
});

const js = readFileSync(rel(jsSrc), "utf8");

html = html
  .replace(/<link rel="stylesheet" crossorigin href="[^"]+">/, `<style>${css}</style>`)
  .replace(
    /<script type="module" crossorigin src="[^"]+"><\/script>/,
    `<script type="module">${js}</script>`,
  )
  .replace(/<link rel="(icon|apple-touch-icon)"[^>]*>/g, "")
  .replace(/<link rel="manifest"[^>]*>/g, "")
  .replace(/<script id="vite-plugin-pwa:register-sw"[^>]*><\/script>/g, "");

writeFileSync("ferrum-preview.html", html);
console.log("ferrum-preview.html", (html.length / 1024).toFixed(0), "KB");
