import type { UsageDayBucket, UsageLogItem, UsageQuery } from "../api/client.ts";

export class UsageLoadError extends Error {}

export function localDate(ts: number): string {
  const date = new Date(ts * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export type UsageRange = "today" | "7d" | "30d" | "all" | "custom";

export function usageRange(range: UsageRange, from: string, to: string, now = new Date()): [number, number] {
  if (range === "all") return [0, Math.floor(now.getTime() / 1000)];
  const start = range === "custom" ? new Date(`${from}T00:00:00`) : new Date(now);
  const end = range === "custom" ? new Date(`${to}T00:00:00`) : new Date(now);
  start.setHours(0, 0, 0, 0);
  end.setHours(0, 0, 0, 0);
  if (range !== "custom") start.setDate(start.getDate() - (range === "7d" ? 6 : range === "30d" ? 29 : 0));
  if (!Number.isFinite(start.getTime()) || !Number.isFinite(end.getTime()) || start > end) {
    throw new UsageLoadError("请选择有效的开始和结束日期，开始日期不能晚于结束日期。");
  }
  end.setDate(end.getDate() + 1);
  return [Math.floor(start.getTime() / 1000), Math.floor(end.getTime() / 1000) - 1];
}

export function usageTrend(raw: UsageDayBucket[], start: number, end: number): UsageDayBucket[] {
  if (!start || !end) return raw;
  // 长区间保留所有实际日桶，不静默截断后半段；短区间补齐无消费日期。
  if (end - start > 366 * 86400) return raw;
  const byDate = new Map(raw.map((bucket) => [bucket.date, bucket]));
  const result: UsageDayBucket[] = [];
  const date = new Date(start * 1000);
  date.setHours(0, 0, 0, 0);
  while (date.getTime() / 1000 <= end) {
    const key = localDate(date.getTime() / 1000);
    result.push(byDate.get(key) ?? { date: key, quota: 0, tokens: 0, requests: 0 });
    date.setDate(date.getDate() + 1);
  }
  return result;
}

export function usageUnit(value: unknown): number {
  const unit = typeof value === "number" || typeof value === "string" ? Number(value) : NaN;
  if (!Number.isFinite(unit) || unit <= 0) throw new UsageLoadError("无法确认站点的金额单位，请刷新重试。暂不展示估算金额。");
  return unit;
}

export interface UsagePage { items: UsageLogItem[] | null; total?: number }

/** IDs in new-api user logs can be page ordinals, not persistent ledger IDs. Never deduplicate them. */
export async function collectUsage(
  fetchPage: (query: UsageQuery) => Promise<UsagePage>,
  query: UsageQuery,
  signal?: AbortSignal,
): Promise<UsageLogItem[]> {
  const result: UsageLogItem[] = [];
  let expected: number | undefined;
  // A bounded failure is safer than presenting an incomplete total as a complete bill.
  for (let page = 1; page <= 1000; page += 1) {
    signal?.throwIfAborted();
    const response = await fetchPage({ ...query, models: undefined, page, pageSize: 100 });
    signal?.throwIfAborted();
    const items = response.items ?? [];
    if (!Array.isArray(items)) throw new UsageLoadError("站点返回的用量记录格式无效。");
    if (response.total !== undefined) {
      if (!Number.isSafeInteger(response.total) || response.total < 0) throw new UsageLoadError("站点返回的记录总数无效。");
      if (expected !== undefined && expected !== response.total) throw new UsageLoadError("读取期间账单发生变化，请刷新后重新核对。");
      expected = response.total;
    }
    for (const item of items) {
      if (![item.created_at, item.quota, item.prompt_tokens, item.completion_tokens].every(n => Number.isFinite(n) && n >= 0)) {
        throw new UsageLoadError("站点返回的用量数值无效，无法准确统计。");
      }
    }
    result.push(...items);
    if (expected !== undefined && result.length > expected) throw new UsageLoadError("站点分页记录与总数不一致，请刷新重试。");
    if (expected !== undefined && result.length === expected) break;
    if (items.length === 0) {
      if (expected !== undefined && result.length < expected) throw new UsageLoadError("用量记录尚未读全，请刷新重试。");
      break;
    }
    if (page === 1000) throw new UsageLoadError("所选范围记录过多，请缩小时间范围后查看完整统计。");
  }
  return result.filter(item =>
    (!query.startTimestamp || item.created_at >= query.startTimestamp) &&
    (!query.endTimestamp || item.created_at <= query.endTimestamp) &&
    (!query.group || item.group === query.group) &&
    (!query.modelName || item.model_name === query.modelName) &&
    (query.models === undefined || query.models.includes(item.model_name)),
  );
}
