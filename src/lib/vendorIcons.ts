// Curated local brand assets. Provenance and licenses: public/vendor-icons/CREDITS.md.
// The server supplies an icon identifier, never an image URL or SVG markup.
export const VENDOR_ICON_ASSETS = {
  ai302: { file: "302-official.png", monochrome: false },
  google: { file: "google-color.svg", monochrome: false },
  gemini: { file: "gemini-color.svg", monochrome: false },
  nanogpt: { file: "nanogpt.png", monochrome: false },
  vertexai: { file: "vertexai-color.svg", monochrome: false },
  opencode: { file: "opencode.svg", monochrome: true },
  deepseek: { file: "deepseek-color.svg", monochrome: false },
  xai: { file: "xai.svg", monochrome: true },
  minimax: { file: "minimax-color.svg", monochrome: false },
  anthropic: { file: "anthropic.svg", monochrome: true },
  claude: { file: "claude-color.svg", monochrome: false },
  openai: { file: "openai.svg", monochrome: true },
  spark: { file: "spark-color.svg", monochrome: false },
  sap: { file: "sap.svg", monochrome: false },
  zhipu: { file: "zhipu-color.svg", monochrome: false },
  moonshot: { file: "moonshot.svg", monochrome: true },
  zai: { file: "zai.svg", monochrome: true },
  doubao: { file: "doubao-color.svg", monochrome: false },
  qwen: { file: "qwen-color.svg", monochrome: false },
} as const;

export type VendorIconId = keyof typeof VENDOR_ICON_ASSETS;
export interface ResolvedVendorIcon {
  id: VendorIconId;
  src: string;
  monochrome: boolean;
}

const SERVER_ICON_IDS = new Map<string, VendorIconId>([
  ["ai302", "ai302"], ["ai302.color", "ai302"],
  ["google", "google"], ["google.color", "google"],
  ["gemini", "gemini"], ["gemini.color", "gemini"],
  ["nanogpt", "nanogpt"],
  ["vertexai", "vertexai"], ["vertexai.color", "vertexai"],
  ["opencode", "opencode"],
  ["deepseek", "deepseek"], ["deepseek.color", "deepseek"],
  ["xai", "xai"],
  ["minimax", "minimax"], ["minimax.color", "minimax"],
  ["anthropic", "anthropic"],
  ["claude", "claude"], ["claude.color", "claude"],
  ["openai", "openai"],
  ["spark", "spark"], ["spark.color", "spark"],
  ["sap", "sap"],
  ["zhipu", "zhipu"], ["zhipu.color", "zhipu"],
  ["moonshot", "moonshot"],
  ["zai", "zai"],
  ["doubao", "doubao"], ["doubao.color", "doubao"],
  ["qwen", "qwen"], ["qwen.color", "qwen"],
]);

function normalizedVendor(value: string): string {
  return value.trim().toLowerCase().replace(/[\s_-]+/g, "");
}

const ALIAS_GROUPS: ReadonlyArray<readonly [VendorIconId, readonly string[]]> = [
  ["ai302", ["302.AI", "302AI", "AI302"]],
  ["google", ["Google"]],
  ["gemini", ["Gemini", "Google Gemini"]],
  ["nanogpt", ["NanoGPT", "Nano GPT", "Nano-GPT"]],
  ["vertexai", ["Vertex", "Vertex AI", "Google Vertex AI"]],
  ["opencode", ["OpenCode", "OpenCode Zen"]],
  ["deepseek", ["DeepSeek", "深度求索"]],
  ["xai", ["xAI", "x.AI", "Grok"]],
  ["minimax", ["MiniMax"]],
  ["anthropic", ["Anthropic"]],
  ["claude", ["Claude"]],
  ["openai", ["OpenAI", "ChatGPT"]],
  ["spark", ["讯飞", "科大讯飞", "讯飞星火", "iFlytek", "iFlytek Spark", "Spark"]],
  ["sap", ["SAP", "SAP AI Core"]],
  ["zhipu", ["智谱", "智谱AI", "Zhipu", "Zhipu AI", "GLM", "ChatGLM"]],
  ["moonshot", ["Moonshot", "Moonshot AI", "MoonshotAI", "月之暗面", "Kimi"]],
  ["zai", ["Z.AI", "ZAI", "Z AI"]],
  ["doubao", ["字节跳动", "ByteDance", "Doubao", "豆包"]],
  ["qwen", ["阿里巴巴", "Alibaba", "Qwen", "通义千问", "千问"]],
];
const VENDOR_ALIASES = new Map<string, VendorIconId>(
  ALIAS_GROUPS.flatMap(([id, aliases]) => aliases.map((name) => [normalizedVendor(name), id] as const)),
);

/** Known server identifiers win; unknown identifiers fall back to exact brand aliases. */
export function resolveVendorIcon(vendor: string, icon?: string | null): ResolvedVendorIcon | null {
  const serverId = typeof icon === "string" ? SERVER_ICON_IDS.get(icon.trim().toLowerCase()) : undefined;
  const id = serverId ?? VENDOR_ALIASES.get(normalizedVendor(vendor));
  if (!id) return null;
  const asset = VENDOR_ICON_ASSETS[id];
  return { id, src: `/vendor-icons/${asset.file}`, monochrome: asset.monochrome };
}
