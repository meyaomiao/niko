// 上游厂商归类：分组名和模型名都按前缀/关键词判断，未匹配统一归「其他」
// 国产厂家命名对齐米云 /api/pricing 的 vendors 表（DeepSeek / 智谱 / Moonshot AI / Z.AI），
// 服务端没下发 vendor_id 的模型由本地启发式归入这些厂家，不再全部堆进「其他」
export const VENDORS = [
  "OpenAI",
  "Anthropic",
  "Google",
  "xAI",
  "DeepSeek",
  "智谱",
  "Moonshot AI",
  "MiniMax",
  "其他",
] as const;
export type Vendor = (typeof VENDORS)[number];

export function vendorOfGroup(name: string): Vendor {
  const n = name.toLowerCase();
  if (n.startsWith("gpt") || n.includes("openai") || n.startsWith("codex")) return "OpenAI";
  if (
    n.startsWith("claude") ||
    n.includes("kiro") ||
    n.includes("cursor") ||
    n.includes("copilot") ||
    n.startsWith("cc")
  )
    return "Anthropic";
  if (n.startsWith("gemini") || n.includes("google")) return "Google";
  if (n.startsWith("grok") || n.includes("xai")) return "xAI";
  // 分组名没有匹配时，按该分组的模型名启发式（deepseek/glm/kimi…）归类
  return vendorOfModel(name);
}

export function vendorOfModel(name: string): Vendor {
  const n = name.toLowerCase();
  if (n.startsWith("claude") || n.startsWith("anthropic")) return "Anthropic";
  if (n.startsWith("grok")) return "xAI";
  if (n.startsWith("gemini") || n.startsWith("imagen") || n.startsWith("veo") || n.startsWith("text-embedding-00"))
    return "Google";
  if (
    n.startsWith("gpt") ||
    n.startsWith("o1") ||
    n.startsWith("o3") ||
    n.startsWith("o4") ||
    n.startsWith("codex") ||
    n.startsWith("chatgpt") ||
    n.startsWith("dall-e") ||
    n.startsWith("whisper") ||
    n.startsWith("tts-") ||
    n.startsWith("text-embedding-")
  )
    return "OpenAI";
  // 国产厂家：按模型名前缀拆分，命名与米云 vendors 表一致，不再全部堆进「其他」
  if (n.startsWith("deepseek")) return "DeepSeek";
  if (n.startsWith("glm") || n.startsWith("chatglm") || n.startsWith("z-")) return "智谱";
  if (n.startsWith("kimi") || n.startsWith("moonshot")) return "Moonshot AI";
  if (n.startsWith("minimax") || n.startsWith("abab")) return "MiniMax";
  return "其他";
}
