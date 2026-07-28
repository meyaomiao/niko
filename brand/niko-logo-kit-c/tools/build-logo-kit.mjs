import fs from "node:fs";
import path from "node:path";
import { createRequire } from "node:module";

const root = path.resolve(import.meta.dirname, "..");
const repository = path.resolve(root, "../..");
const require = createRequire(path.join(repository, "website/package.json"));
const sharp = require("sharp");

const dirs = ["logo", "variants", "icons", "png/mark", "guidelines", "previews"];
for (const dir of dirs) fs.mkdirSync(path.join(root, dir), { recursive: true });

const colors = {
  blue: "#78C5DF",
  yellow: "#F3D47F",
  apricot: "#F5AD78",
  coral: "#EE9288",
  ink: "#22304A",
  cream: "#FFF8F0",
  night: "#162038",
  black: "#000000",
  white: "#FFFFFF",
};

// One closed contour: no overlapping primitives, joins, or hidden seams.
const markPath = "M310 152C256 152 210 197 210 260V724C210 789 240 834 286 834C333 834 356 792 355 730C354 651 350 564 348 494C347 442 365 404 394 404C439 404 468 458 503 523L566 638C624 744 678 834 744 834C792 834 824 788 824 722V234C824 179 793 146 750 146C703 146 674 182 675 238C676 301 678 383 679 446C680 496 664 525 635 525C600 525 568 478 534 425L435 270C389 197 350 152 310 152Z";

// Slightly opened counter and simpler three-stop gradient for 16–32 px use.
const smallMarkPath = "M314 166C257 166 216 207 216 265V721C216 785 245 824 289 824C335 824 358 785 357 725L352 500C351 449 369 414 398 414C439 414 470 467 505 530L565 638C622 741 677 824 739 824C784 824 812 782 812 721V244C812 192 785 162 748 162C706 162 681 196 682 243L687 444C688 493 671 520 640 520C607 520 575 476 540 422L438 266C393 196 354 166 314 166Z";

const gradient = (id = "niko-gradient", simple = false) => `
    <linearGradient id="${id}" x1="210" y1="490" x2="824" y2="490" gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="${colors.blue}"/>
      <stop offset="${simple ? "0.5" : "0.42"}" stop-color="${colors.yellow}"/>${simple ? "" : `
      <stop offset="0.70" stop-color="${colors.apricot}"/>`}
      <stop offset="1" stop-color="${colors.coral}"/>
    </linearGradient>`;

const doc = ({ viewBox, title, desc, defs = "", body }) => `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${viewBox}" role="img" aria-labelledby="title desc">
  <title id="title">${title}</title>
  <desc id="desc">${desc}</desc>${defs ? `
  <defs>${defs}
  </defs>` : ""}
  ${body}
</svg>
`;

const mark = ({ fill = "url(#niko-gradient)", transform = "", small = false } = {}) =>
  `<path fill="${fill}"${transform ? ` transform="${transform}"` : ""} d="${small ? smallMarkPath : markPath}"/>`;

const wordmark = (color = colors.ink, transform = "") => `<g${transform ? ` transform="${transform}"` : ""} fill="none" stroke="${color}" stroke-width="38" stroke-linecap="round" stroke-linejoin="round">
    <path d="M60 208V52L210 208V52"/>
    <path d="M286 112V208"/>
    <circle cx="286" cy="62" r="20" fill="${color}" stroke="none"/>
    <path d="M365 52V208M377 150L454 96M377 150L462 210"/>
    <ellipse cx="555" cy="154" rx="63" ry="58"/>
  </g>`;

const svgs = new Map();
const add = (name, svg) => svgs.set(name, svg);

add("logo/niko-mark-full-color.svg", doc({
  viewBox: "0 0 1024 1024",
  title: "Niko full-color mark",
  desc: "A continuous rounded N with a soft blue, warm yellow, apricot, and coral gradient.",
  defs: gradient(),
  body: mark(),
}));

add("logo/niko-wordmark.svg", doc({
  viewBox: "0 0 680 260",
  title: "Niko wordmark",
  desc: "A custom rounded geometric Niko wordmark with no font dependency.",
  body: wordmark(),
}));

const horizontal = (wordColor = colors.ink, background = "") => doc({
  viewBox: "0 0 1280 460",
  title: "Niko horizontal logo",
  desc: "The full-color Niko mark paired with the custom Niko wordmark.",
  defs: gradient(),
  body: `${background}${mark({ transform: "translate(-35 -50) scale(.56)" })}\n  ${wordmark(wordColor, "translate(470 100) scale(1.25)")}`,
});

const primary = horizontal();
add("logo/niko-primary-full-color.svg", primary);
add("logo/niko-lockup-horizontal.svg", primary);

add("logo/niko-lockup-stacked.svg", doc({
  viewBox: "0 0 800 900",
  title: "Niko stacked logo",
  desc: "The full-color Niko mark above the custom wordmark.",
  defs: gradient(),
  body: `${mark({ transform: "translate(38 -15) scale(.70)" })}\n  ${wordmark(colors.ink, "translate(15 620) scale(1.15)")}`,
}));

add("logo/niko-mark-small.svg", doc({
  viewBox: "0 0 1024 1024",
  title: "Niko small-size mark",
  desc: "An optically simplified Niko mark for display at 16 to 32 pixels.",
  defs: gradient("niko-small-gradient", true),
  body: mark({ fill: "url(#niko-small-gradient)", small: true }),
}));

for (const [name, color] of [
  ["black", colors.black],
  ["white", colors.white],
  ["monochrome-coral", colors.coral],
]) {
  add(`variants/niko-mark-${name}.svg`, doc({
    viewBox: "0 0 1024 1024",
    title: `Niko ${name} mark`,
    desc: `Single-color ${name} version of the Niko mark.`,
    body: mark({ fill: color }),
  }));
}

add("variants/niko-primary-black.svg", doc({
  viewBox: "0 0 1280 460",
  title: "Niko black logo",
  desc: "Single-color black Niko horizontal logo.",
  body: `${mark({ fill: colors.black, transform: "translate(-35 -50) scale(.56)" })}\n  ${wordmark(colors.black, "translate(470 100) scale(1.25)")}`,
}));

add("variants/niko-primary-white.svg", doc({
  viewBox: "0 0 1280 460",
  title: "Niko white logo",
  desc: "Single-color white Niko horizontal logo for dark backgrounds.",
  body: `${mark({ fill: colors.white, transform: "translate(-35 -50) scale(.56)" })}\n  ${wordmark(colors.white, "translate(470 100) scale(1.25)")}`,
}));

add("variants/niko-primary-on-light.svg", horizontal(colors.ink, `<rect width="1280" height="460" fill="${colors.cream}"/>\n  `));
add("variants/niko-primary-on-dark.svg", horizontal(colors.white, `<rect width="1280" height="460" fill="${colors.night}"/>\n  `));

add("icons/favicon.svg", doc({
  viewBox: "0 0 1024 1024",
  title: "Niko favicon",
  desc: "Small-size Niko mark for browser tabs.",
  defs: gradient("niko-small-gradient", true),
  body: mark({ fill: "url(#niko-small-gradient)", small: true }),
}));

const appIcon = doc({
  viewBox: "0 0 1024 1024",
  title: "Niko app icon",
  desc: "The Niko mark on a warm cream rounded square.",
  defs: gradient(),
  body: `<rect x="64" y="64" width="896" height="896" rx="220" fill="${colors.cream}"/>\n  ${mark({ transform: "translate(72 96) scale(.85)" })}`,
});
add("icons/niko-app-icon.svg", appIcon);
add("icons/niko-web-icon.svg", appIcon);

add("icons/niko-social-avatar.svg", doc({
  viewBox: "0 0 1024 1024",
  title: "Niko social avatar",
  desc: "The full-color Niko mark centered on a deep navy circle.",
  defs: gradient(),
  body: `<circle cx="512" cy="512" r="448" fill="${colors.night}"/>\n  ${mark({ transform: "translate(88 88) scale(.82)" })}`,
}));

const safeGuide = doc({
  viewBox: "0 0 1800 1400",
  title: "Niko logo usage guide",
  desc: "Clear space, minimum sizes, and incorrect usage guidance.",
  defs: `${gradient()}\n    <filter id="wrong-shadow"><feDropShadow dx="18" dy="18" stdDeviation="12" flood-color="#162038" flood-opacity="0.45"/></filter>`,
  body: `<rect width="1800" height="1400" fill="#F7F9FC"/>
  <text x="90" y="110" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="54" font-weight="700">Niko Logo Usage Guide</text>
  <text x="90" y="160" fill="#667085" font-family="Arial, sans-serif" font-size="24">Clear space · minimum size · incorrect use</text>
  <rect x="90" y="220" width="780" height="640" rx="36" fill="#FFFFFF"/>
  <text x="140" y="290" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="30" font-weight="700">Clear space</text>
  <rect x="225" y="350" width="510" height="420" fill="none" stroke="${colors.blue}" stroke-width="4" stroke-dasharray="14 12"/>
  <path fill="none" stroke="${colors.coral}" stroke-width="3" d="M225 325V350M735 325V350M200 350H225M200 770H225"/>
  <text x="465" y="335" text-anchor="middle" fill="${colors.coral}" font-family="Arial, sans-serif" font-size="22">x</text>
  ${mark({ transform: "translate(205 275) scale(.54)" })}
  <text x="140" y="820" fill="#667085" font-family="Arial, sans-serif" font-size="22">Keep x = ¼ mark width clear on all sides.</text>
  <rect x="920" y="220" width="790" height="300" rx="36" fill="#FFFFFF"/>
  <text x="970" y="290" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="30" font-weight="700">Minimum digital size</text>
  <g transform="translate(975 340)">${mark({ transform: "scale(.0625)", small: true })}</g>
  <text x="1007" y="445" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">16 px small</text>
  <g transform="translate(1160 320)">${mark({ transform: "scale(.09375)", small: true })}</g>
  <text x="1208" y="445" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">24 px mark</text>
  <g transform="translate(1375 295)">${mark({ transform: "scale(.14)" })}</g>
  <text x="1447" y="445" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">32 px preferred</text>
  <text x="970" y="490" fill="#667085" font-family="Arial, sans-serif" font-size="20">Horizontal lockup: 120 px wide · Print mark: 8 mm</text>
  <rect x="920" y="560" width="790" height="650" rx="36" fill="#FFFFFF"/>
  <text x="970" y="630" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="30" font-weight="700">Incorrect use</text>
  <g transform="translate(980 680)">${mark({ transform: "rotate(12 512 512) scale(.19)" })}</g>
  <path d="M980 680L1175 875M1175 680L980 875" stroke="#E5484D" stroke-width="12"/>
  <text x="1078" y="920" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">Do not rotate</text>
  <g transform="translate(1250 695) scale(.28 .17)">${mark()}</g>
  <path d="M1250 680L1535 875M1535 680L1250 875" stroke="#E5484D" stroke-width="12"/>
  <text x="1392" y="920" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">Do not distort</text>
  <g transform="translate(980 940)">${mark({ transform: "scale(.19)", fill: colors.yellow })}</g>
  <path d="M980 940L1175 1135M1175 940L980 1135" stroke="#E5484D" stroke-width="12"/>
  <text x="1078" y="1180" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">Do not recolor</text>
  <g transform="translate(1250 940)" filter="url(#wrong-shadow)">${mark({ transform: "scale(.19)" })}</g>
  <path d="M1250 940L1445 1135M1445 940L1250 1135" stroke="#E5484D" stroke-width="12"/>
  <text x="1348" y="1180" text-anchor="middle" fill="#667085" font-family="Arial, sans-serif" font-size="20">No effects or outlines</text>
  <text x="90" y="1325" fill="#667085" font-family="Arial, sans-serif" font-size="22">Never crop, crowd, rebuild, or place the full-color mark on low-contrast imagery.</text>`,
});
add("guidelines/niko-logo-guidelines.svg", safeGuide);

const overview = doc({
  viewBox: "0 0 1800 1400",
  title: "Niko Logo Kit overview",
  desc: "Overview of the Niko Option C logo system and core color palette.",
  defs: gradient(),
  body: `<rect width="1800" height="1400" fill="#F3F5F8"/>
  <text x="80" y="105" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="58" font-weight="700">Niko Logo Kit</text>
  <text x="80" y="155" fill="#667085" font-family="Arial, sans-serif" font-size="24">Option C · Production master · 2026</text>
  <rect x="80" y="205" width="1080" height="390" rx="40" fill="${colors.cream}"/>
  ${mark({ transform: "translate(25 150) scale(.40)" })}
  ${wordmark(colors.ink, "translate(455 295) scale(1.05)")}
  <text x="120" y="550" fill="#8A7B73" font-family="Arial, sans-serif" font-size="20">PRIMARY · LIGHT</text>
  <rect x="1200" y="205" width="520" height="390" rx="40" fill="${colors.night}"/>
  ${mark({ transform: "translate(1180 155) scale(.46)" })}
  <text x="1240" y="565" fill="#C4CAD6" font-family="Arial, sans-serif" font-size="20">MARK · DARK</text>
  <rect x="80" y="635" width="510" height="405" rx="40" fill="#FFFFFF"/>
  ${mark({ fill: colors.black, transform: "translate(40 570) scale(.48)" })}
  <text x="120" y="995" fill="#667085" font-family="Arial, sans-serif" font-size="20">BLACK</text>
  <rect x="625" y="635" width="510" height="405" rx="40" fill="${colors.night}"/>
  ${mark({ fill: colors.white, transform: "translate(585 570) scale(.48)" })}
  <text x="665" y="995" fill="#C4CAD6" font-family="Arial, sans-serif" font-size="20">WHITE</text>
  <rect x="1170" y="635" width="550" height="405" rx="40" fill="#FFFFFF"/>
  <rect x="1220" y="680" width="250" height="250" rx="62" fill="${colors.cream}"/>
  ${mark({ transform: "translate(1192 660) scale(.31)" })}
  <circle cx="1580" cy="805" r="125" fill="${colors.night}"/>
  ${mark({ transform: "translate(1484 710) scale(.19)" })}
  <text x="1210" y="995" fill="#667085" font-family="Arial, sans-serif" font-size="20">APP ICON · SOCIAL</text>
  <text x="80" y="1125" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="28" font-weight="700">Core palette</text>
  ${[
    [80, colors.blue, "SKY BLUE", "#78C5DF"],
    [410, colors.yellow, "WARM YELLOW", "#F3D47F"],
    [740, colors.apricot, "APRICOT", "#F5AD78"],
    [1070, colors.coral, "CORAL", "#EE9288"],
    [1400, colors.ink, "INK", "#22304A"],
  ].map(([x, color, label, hex]) => `<g transform="translate(${x} 1170)"><rect width="280" height="110" rx="24" fill="${color}"/><text x="0" y="148" fill="${colors.ink}" font-family="Arial, sans-serif" font-size="18" font-weight="700">${label}</text><text x="0" y="175" fill="#667085" font-family="Arial, sans-serif" font-size="17">${hex}</text></g>`).join("\n  ")}`,
});
add("previews/niko-logo-kit-overview.svg", overview);

for (const [name, svg] of svgs) fs.writeFileSync(path.join(root, name), svg);

const render = async (svgName, pngName, width, height) => {
  const svg = svgs.get(svgName) ?? fs.readFileSync(path.join(root, svgName));
  await sharp(Buffer.from(svg), { density: 288 })
    .resize(width, height, { fit: "fill", kernel: sharp.kernel.lanczos3 })
    .png({ compressionLevel: 9, adaptiveFiltering: true })
    .toFile(path.join(root, pngName));
};

await render("logo/niko-mark-full-color.svg", "logo/niko-mark-full-color.png", 1024, 1024);
await render("logo/niko-wordmark.svg", "logo/niko-wordmark.png", 1360, 520);
await render("logo/niko-primary-full-color.svg", "logo/niko-primary-full-color.png", 1280, 460);
await render("logo/niko-lockup-horizontal.svg", "logo/niko-lockup-horizontal.png", 1280, 460);
await render("logo/niko-lockup-stacked.svg", "logo/niko-lockup-stacked.png", 800, 900);
await render("logo/niko-mark-small.svg", "logo/niko-mark-small.png", 256, 256);

for (const name of ["black", "white", "monochrome-coral"]) {
  await render(`variants/niko-mark-${name}.svg`, `variants/niko-mark-${name}.png`, 1024, 1024);
}
for (const name of ["black", "white", "on-light", "on-dark"]) {
  await render(`variants/niko-primary-${name}.svg`, `variants/niko-primary-${name}.png`, 1280, 460);
}

for (const size of [16, 32, 48, 64, 128, 256, 512, 1024]) {
  const source = size <= 32 ? "logo/niko-mark-small.svg" : "logo/niko-mark-full-color.svg";
  await render(source, `png/mark/niko-mark-${size}.png`, size, size);
}

for (const size of [16, 32, 48]) await render("icons/favicon.svg", `icons/favicon-${size}.png`, size, size);
for (const size of [128, 256, 512, 1024]) await render("icons/niko-app-icon.svg", `icons/niko-app-icon-${size}.png`, size, size);
for (const size of [192, 512]) await render("icons/niko-web-icon.svg", `icons/niko-web-icon-${size}.png`, size, size);
for (const size of [512, 1024]) await render("icons/niko-social-avatar.svg", `icons/niko-social-avatar-${size}.png`, size, size);
await render("icons/favicon.svg", "icons/favicon.png", 32, 32);
await render("icons/niko-app-icon.svg", "icons/niko-app-icon.png", 1024, 1024);
await render("icons/niko-web-icon.svg", "icons/niko-web-icon.png", 512, 512);
await render("icons/niko-social-avatar.svg", "icons/niko-social-avatar.png", 1024, 1024);

await render("guidelines/niko-logo-guidelines.svg", "guidelines/niko-logo-guidelines.png", 1800, 1400);
await render("previews/niko-logo-kit-overview.svg", "previews/niko-logo-kit-overview.png", 1800, 1400);

console.log(`Built ${svgs.size} SVG masters and PNG exports in ${root}`);
