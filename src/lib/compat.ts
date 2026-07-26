// E7-2: 兼容等级基线表 + 实测结果合并
// 四级定义见 docs/momo-launcher-design.md §9

export type CompatLevel = "native" | "good" | "limited" | "unsupported";

export const COMPAT_LABEL: Record<CompatLevel, string> = {
  native: "原生兼容",
  good: "转换接入",
  limited: "部分受限",
  unsupported: "不建议",
};

export const COMPAT_STYLE: Record<CompatLevel, string> = {
  native: "bg-green-900/30 text-green-400",
  good: "bg-blue-900/30 text-blue-400",
  limited: "bg-yellow-900/30 text-yellow-400",
  unsupported: "bg-gray-800 text-gray-500",
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
  "claude-code": { claude: "native", gpt: "good", gemini: "limited", other: "limited" },
};

const NOTES: Record<string, Record<Family, string>> = {
  "codex": {
    claude: "经服务端协议转换接入，对话与工具调用正常",
    gpt: "同协议直连，原生能力全部可用",
    gemini: "多模态与思考链存在差异，部分能力缺失",
    other: "未在该目标上验证过的模型，能力可能缺失",
  },
  "claude-desktop": {
    claude: "同协议直连，原生能力全部可用",
    gpt: "经 MCP 通道转换接入，扩展思考不可用",
    gemini: "多模态与思考链差异最大，仅基础对话可用",
    other: "未在该目标上验证过的模型，能力可能缺失",
  },
  "claude-code": {
    claude: "同协议直连，原生能力全部可用",
    gpt: "经服务端协议转换接入，日常对话与工具调用正常",
    gemini: "多模态与思考链存在差异，部分能力缺失",
    other: "未在该目标上验证过的模型，能力可能缺失",
  },
};

export function baselineFor(targetId: string, model: string): { level: CompatLevel; note: string } {
  const family = modelFamily(model);
  const level = MATRIX[targetId]?.[family] ?? "unsupported";
  const note = NOTES[targetId]?.[family] ?? "该组合未收录，等级未知";
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
