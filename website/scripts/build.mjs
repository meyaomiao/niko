import { cpSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import sharp from "sharp";

const here = dirname(fileURLToPath(import.meta.url));
const website = resolve(here, "..");
const repository = resolve(website, "..");
const brand = join(repository, "brand/niko-logo-kit-c");
const source = join(website, "src");
const output = join(website, "dist");

rmSync(output, { recursive: true, force: true });
cpSync(source, output, { recursive: true });
renameSync(join(output, "styles.css"), join(output, "styles-wordmark.css"));
mkdirSync(join(output, "assets"), { recursive: true });

for (const [from, to] of [
  ["logo/niko-mark-full-color.svg", "assets/niko-mark.svg"],
  ["logo/niko-wordmark.svg", "assets/niko-wordmark.svg"],
  ["logo/niko-lockup-horizontal.svg", "assets/niko-lockup-horizontal.svg"],
  ["icons/niko-app-icon-1024.png", "assets/niko-app-icon.png"],
  ["icons/niko-web-icon-192.png", "assets/niko-web-icon-192.png"],
  ["icons/niko-web-icon-512.png", "assets/niko-web-icon-512.png"],
  ["icons/favicon.svg", "favicon.svg"],
  ["icons/favicon.ico", "favicon.ico"],
  ["icons/favicon-16.png", "favicon-16.png"],
  ["icons/favicon-32.png", "favicon-32.png"],
  ["icons/favicon.png", "favicon.png"]
]) {
  const brandAsset = join(brand, from);
  if (!existsSync(brandAsset)) {
    throw new Error(`Missing Niko Logo Kit C asset: ${from}`);
  }
  cpSync(brandAsset, join(output, to));
}

await sharp(join(brand, "variants/niko-primary-on-light.png"))
  .resize(900, 323, { fit: "fill" })
  .extend({
    top: 153,
    bottom: 154,
    left: 150,
    right: 150,
    background: "#FFF8F0"
  })
  .png({ compressionLevel: 9, adaptiveFiltering: true })
  .toFile(join(output, "og.png"));

for (const filename of [
  "index.html",
  "login/index.html",
  "register/index.html",
  "account/index.html",
  "payment/return/index.html",
  "styles-wordmark.css",
  "account.css",
  "js/api.js",
  "js/auth.js",
  "js/account.js",
  "js/site-session.js",
  "js/payment-return.js",
  "robots.txt",
  "sitemap.xml",
  "manifest.webmanifest",
  "og.png"
]) {
  if (!existsSync(join(output, filename))) {
    throw new Error(`Missing required website file: ${filename}`);
  }
}

const html = readFileSync(join(output, "index.html"), "utf8");
for (const required of [
  'src="/assets/niko-wordmark.svg"',
  'href="/login/"',
  'href="/account/"',
  "https://niko-ai.cc/",
  "github.com/meyaomiao/niko/releases"
]) {
  if (!html.includes(required)) {
    throw new Error(`Website is missing required content: ${required}`);
  }
}

console.log("Built Niko website in website/dist");
