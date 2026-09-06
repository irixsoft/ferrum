import sharp from "sharp";
import { readFileSync, writeFileSync } from "node:fs";

// icon.svg's <style> class does not survive re-wrapping, so paths are re-coloured inline.
const raw = readFileSync("public/icon.svg", "utf8");
const paths = [...raw.matchAll(/<path[^>]*d="([^"]+)"[^>]*\/>/g)].map((m) => m[1]);
if (paths.length !== 4) throw new Error(`expected 4 paths in icon.svg, found ${paths.length}`);

const mark = (fill) => paths.map((d) => `<path fill="${fill}" d="${d}"/>`).join("");

const plain = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 500">${mark("#14171A")}</svg>`;

// Maskable icons must keep the mark inside the 80% safe zone, on an opaque field.
const maskable = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 500 500">
  <rect width="500" height="500" fill="#14171A"/>
  <g transform="translate(250 250) scale(0.66) translate(-250 -250)">${mark("#E8EBED")}</g>
</svg>`;

const jobs = [
  ["public/pwa-192.png", plain, 192],
  ["public/pwa-512.png", plain, 512],
  ["public/pwa-maskable-192.png", maskable, 192],
  ["public/pwa-maskable-512.png", maskable, 512],
  ["public/apple-touch-icon.png", maskable, 180],
];

for (const [out, svg, size] of jobs) {
  writeFileSync(out, await sharp(Buffer.from(svg)).resize(size, size).png().toBuffer());
  console.log(out, size);
}
