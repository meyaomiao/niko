// 上游厂商归类：分组名和模型名都按前缀/关键词判断，未匹配统一归「其他」
export const VENDORS = ["OpenAI", "Anthropic", "Google", "其他"] as const;
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
  return "其他";
}

export function vendorOfModel(name: string): Vendor {
  const n = name.toLowerCase();
  if (n.startsWith("claude") || n.startsWith("anthropic")) return "Anthropic";
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
  return "其他";
}
