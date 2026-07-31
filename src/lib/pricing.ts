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
  if (!Number.isFinite(groupRatio) || groupRatio <= 0) return null;

  const validAmount = (value: unknown): value is number =>
    typeof value === "number" && Number.isFinite(value) && value >= 0;
  const multiplyPrice = (left: number, right: number): number | null => {
    const value = left * right;
    return Number.isFinite(value) ? value : null;
  };

  if (item.quota_type === 1) {
    if (!validAmount(item.model_price)) return null;
    const input = multiplyPrice(item.model_price, groupRatio);
    return input === null ? null : { perRequest: true, input, output: 0 };
  }
  if (item.quota_type !== 0 || !validAmount(item.model_ratio) || !validAmount(item.completion_ratio)) {
    return null;
  }

  const base = multiplyPrice(item.model_ratio * 2, groupRatio);
  if (base === null) return null;
  const completionRatio = item.completion_ratio || 1;
  const output = multiplyPrice(base, completionRatio);
  if (output === null) return null;
  const price: ModelPrice = {
    perRequest: false,
    input: base,
    output,
  };
  if (validAmount(item.cache_ratio)) {
    const cache = multiplyPrice(base, item.cache_ratio);
    if (cache !== null) price.cache = cache;
  }
  if (validAmount(item.create_cache_ratio)) {
    const createCache = multiplyPrice(base, item.create_cache_ratio);
    if (createCache !== null) price.createCache = createCache;
  }
  return price;
}

/** 金额保留可读精度；免费不显示成容易误解的 $0。 */
export function fmtUSD(value: number): string {
  if (!Number.isFinite(value) || value < 0) return "价格暂不可用";
  if (value === 0) return "免费";
  if (value < 0.000001) return "<$0.000001";
  const digits = value < 0.01 ? 6 : 2;
  return `$${value.toFixed(digits)}`;
}
