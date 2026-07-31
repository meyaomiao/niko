// E7-2: 兼容等级基线表 + 实测结果合并
// 四级定义见 docs/niko-design.md §9

export type CompatLevel = "native" | "good" | "limited" | "unsupported";

export const COMPAT_LABEL: Record<CompatLevel, string> = {
  native: "原生兼容",
  good: "服务适配",
  limited: "部分受限",
  unsupported: "不建议",
};

export const COMPAT_STYLE: Record<CompatLevel, string> = {
  native: "bg-green-500/10 text-green-700 dark:text-green-400",
  good: "bg-blue-500/10 text-blue-700 dark:text-blue-400",
  limited: "bg-yellow-500/10 text-yellow-700 dark:text-yellow-500",
  unsupported: "bg-black/5 text-gray-500 dark:bg-white/10 dark:text-gray-400",
};

type Family = "claude" | "gpt" | "gemini" | "other";

export function modelFamily(model: string): Family {
  const m = model.toLowerCase();
  if (m.includes("claude") || m.includes("opus") || m.includes("sonnet") || m.includes("haiku")) return "claude";
  if (m.startsWith("gpt") || m.startsWith("o1") || m.startsWith("o3") || m.startsWith("o4") || m.includes("codex")) return "gpt";
  if (m.includes("gemini")) return "gemini";
  return "other";
}

// 目标应用 × 模型族 的基线等级
const MATRIX: Record<string, Record<Family, CompatLevel>> = {
  "codex": { claude: "good", gpt: "native", gemini: "limited", other: "limited" },
  "claude-desktop": { claude: "native", gpt: "good", gemini: "limited", other: "limited" },
};

const NOTES: Record<string, Record<Family, string>> = {
  "codex": {
    claude: "通过服务适配接入，对话与工具调用正常",
    gpt: "可直接使用，原生能力全部可用",
    gemini: "图片理解和深度思考等能力有差异，部分能力可能不可用",
    other: "暂未在该应用验证，部分能力可能不可用",
  },
  "claude-desktop": {
    claude: "可在内置 Claude Code 面板使用，原生能力全部可用",
    gpt: "通过服务适配接入内置 Claude Code 面板，深度思考不可用",
    gemini: "图片理解和深度思考等能力差异较大，目前仅支持基础对话",
    other: "暂未在该应用验证，部分能力可能不可用",
  },
};

export function baselineFor(targetId: string, model: string): { level: CompatLevel; note: string } {
  const family = modelFamily(model);
  const level = MATRIX[targetId]?.[family] ?? "unsupported";
  const note = NOTES[targetId]?.[family] ?? "暂未收录该组合，兼容性未知";
  return { level, note };
}

// probe_compat 的返回结构
export interface CompatProbe {
  target_id: string;
  model: string;
  ok: boolean;
  level: CompatLevel;
  latency_ms?: number;
  error_kind?: string;
  detail?: string;
  checked_at: number;
}

export function formatCheckedAt(ts: number): string {
  if (!ts) return "";
  return new Date(ts * 1000).toLocaleTimeString("zh-CN", { hour12: false });
}

// 目标应用 → 原生匹配的上游厂商，用于「先选应用、再推荐模型」的默认选中
export const NATIVE_VENDOR: Record<string, string> = {
  codex: "OpenAI",
  "claude-desktop": "Anthropic",
};
