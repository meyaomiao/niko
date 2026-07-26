// 模型价格换算：与网页端 features/pricing/lib/price.ts 保持同一套公式
// 输入价（美元 / 百万 token）= model_ratio * 2 * 分组倍率
import type { PricingItem } from "../api/client";

export interface ModelPrice {
  /** 按次计费时为 true，此时 input 表示每次调用价格 */
  perRequest: boolean;
  input: number;
  output: number;
  cache?: number;
  createCache?: number;
}

export function buildPricingIndex(list: PricingItem[] | undefined): Map<string, PricingItem> {
  const map = new Map<string, PricingItem>();
  for (const item of list ?? []) map.set(item.model_name, item);
  return map;
}

export function priceOf(item: PricingItem | undefined, groupRatio: number): ModelPrice | null {
  if (!item) return null;
  if (item.quota_type === 1) {
    return { perRequest: true, input: (item.model_price || 0) * groupRatio, output: 0 };
  }
  const base = item.model_ratio * 2 * groupRatio;
  const price: ModelPrice = {
    perRequest: false,
    input: base,
    output: base * (item.completion_ratio || 1),
  };
  if (item.cache_ratio != null) price.cache = base * item.cache_ratio;
  if (item.create_cache_ratio != null) price.createCache = base * item.create_cache_ratio;
  return price;
}

/** 保留有效数字并去掉多余的 0，避免 $0.0000 这类无意义显示 */
export function fmtUSD(value: number): string {
  if (!Number.isFinite(value)) return "—";
  if (value === 0) return "$0";
  const digits = value < 0.01 ? 6 : value < 1 ? 4 : 2;
  return `$${Number(value.toFixed(digits))}`;
}
