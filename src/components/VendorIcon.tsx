// 厂商品牌图标：优先品牌专属样式（OpenAI / Anthropic / Google / xAI），
// 其余厂家用「首字母 + 稳定配色」的瓷砖，保证 14 个厂家都有可辨识标识。
// icon 参数来自服务端 vendors[].icon（如 "Claude.Color"、"Gemini.Color"），
// 目前只用于兜底判断，视觉仍以厂家名为准。

const SIZE = "h-[18px] w-[18px] shrink-0 rounded-[5px]";

const GENERIC_COLORS = [
  "bg-[#4F46E5] text-white",
  "bg-[#0EA5E9] text-white",
  "bg-[#059669] text-white",
  "bg-[#D97706] text-white",
  "bg-[#DC2626] text-white",
  "bg-[#7C3AED] text-white",
  "bg-[#0F766E] text-white",
  "bg-[#475569] text-white",
];

/** 同名厂家稳定取色：同名必同色，跨会话一致 */
function colorFor(name: string): string {
  let hash = 0;
  for (let i = 0; i < name.length; i += 1) hash = (hash * 31 + name.charCodeAt(i)) % 9973;
  return GENERIC_COLORS[hash % GENERIC_COLORS.length];
}

function labelFor(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "·";
  // 中文厂家取首字，英文取首字母
  return trimmed.slice(0, 1).toUpperCase();
}

export function VendorIcon({ vendor, size }: { vendor: string; size?: number }) {
  const dimension = size ? { width: size, height: size } : undefined;
  const key = vendor.toLowerCase();

  if (key.includes("openai")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#10A37F] text-[10px] font-bold text-white`} style={dimension}>
        O
      </span>
    );
  }
  if (key.includes("anthropic") || key.includes("claude")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#CC785C] text-[10px] font-bold text-white`} style={dimension}>
        A
      </span>
    );
  }
  if (key.includes("google") || key.includes("gemini") || key.includes("vertex")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-white`} style={dimension}>
        <svg viewBox="0 0 18 18" className="h-[13px] w-[13px]">
          <path fill="#4285F4" d="M17.64 9.2c0-.64-.06-1.25-.16-1.84H9v3.48h4.84a4.14 4.14 0 0 1-1.8 2.72v2.26h2.92c1.7-1.57 2.68-3.88 2.68-6.62Z" />
          <path fill="#34A853" d="M9 18c2.43 0 4.47-.8 5.96-2.18l-2.92-2.26c-.8.54-1.84.86-3.04.86-2.34 0-4.32-1.58-5.03-3.7H.96v2.32A9 9 0 0 0 9 18Z" />
          <path fill="#FBBC05" d="M3.97 10.72a5.41 5.41 0 0 1 0-3.44V4.96H.96a9 9 0 0 0 0 8.08l3.01-2.32Z" />
          <path fill="#EA4335" d="M9 3.58c1.32 0 2.5.45 3.44 1.35l2.58-2.58C13.46.9 11.42 0 9 0A9 9 0 0 0 .96 4.96l3.01 2.32C4.68 5.16 6.66 3.58 9 3.58Z" />
        </svg>
      </span>
    );
  }
  if (key === "xai" || key.includes("grok")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-gray-900 text-[10px] font-bold text-white dark:bg-white dark:text-gray-900`} style={dimension}>
        𝕏
      </span>
    );
  }
  if (key.includes("deepseek")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#4D6BFE] text-[10px] font-bold text-white`} style={dimension}>
        D
      </span>
    );
  }
  if (key.includes("moonshot") || key.includes("kimi")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#111827] text-[10px] font-bold text-white`} style={dimension}>
        K
      </span>
    );
  }
  if (key.includes("智谱") || key.includes("zhipu") || key.includes("glm")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#3859FF] text-[10px] font-bold text-white`} style={dimension}>
        智
      </span>
    );
  }
  if (key.includes("讯飞") || key.includes("spark")) {
    return (
      <span aria-hidden="true" className={`${SIZE} flex items-center justify-center bg-[#1E64FF] text-[10px] font-bold text-white`} style={dimension}>
        讯
      </span>
    );
  }

  return (
    <span
      aria-hidden="true"
      className={`${SIZE} flex items-center justify-center text-[10px] font-bold ${colorFor(vendor)}`}
      style={dimension}
    >
      {labelFor(vendor)}
    </span>
  );
}
