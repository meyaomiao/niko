// 模型标签：全部按「厂家内」横向比较，从已有数据推导，不造数据
// - 刚上新：官方发布日期在 14 天内
// - 常用：该厂家内用量排行前 3（服务端 usage/summary 的本人用量；全站热榜服务端暂未下发）
// - 性价比：该厂家内用量排行前 5，且输入价在该厂家内相对位置低于 1/5

import type { UsageSummary } from "../api/client.ts";
import { vendorOfModel } from "./vendor.ts";

export interface ModelTag {
  id: "new" | "value" | "hot";
  label: string;
  className: string;
}

/** 刚上新窗口：两周 */
const NEW_MS = 14 * 86400_000;

/** 常用：厂家内用量前 N */
export const HOT_RANK_LIMIT = 3;
/** 性价比：厂家内用量前 N 且价格分位低于 VALUE_LEVEL_LIMIT */
export const VALUE_RANK_LIMIT = 5;
export const VALUE_LEVEL_LIMIT = 1 / 5;

export function isNewRelease(release: string | undefined, now = Date.now()): boolean {
  if (!release) return false;
  const ts = Date.parse(release);
  if (!Number.isFinite(ts)) return false;
  return now - ts >= 0 && now - ts <= NEW_MS;
}

/**
 * 厂家内用量排行（1-based）：先按 requests 降序排，再在每个厂家内部排名。
 * 返回 model 名 → 名次；没有用量数据的模型不出现在 Map 里。
 */
export function vendorUsageRanks(
  summary: UsageSummary | null | undefined
): Map<string, number> {
  const byModel = summary?.by_model;
  const ranks = new Map<string, number>();
  if (!byModel?.length) return ranks;

  const perVendor = new Map<string, { name: string; requests: number }[]>();
  for (const item of byModel) {
    if (!item.name) continue;
    const vendor = vendorOfModel(item.name);
    const list = perVendor.get(vendor) ?? [];
    list.push({ name: item.name, requests: item.requests ?? 0 });
    perVendor.set(vendor, list);
  }

  for (const list of perVendor.values()) {
    list
      .sort((a, b) => b.requests - a.requests)
      .forEach((entry, index) => ranks.set(entry.name, index + 1));
  }
  return ranks;
}

/**
 * 厂家内价格分位（0~1，1 为最贵）：同厂家、有输入价的模型之间比较。
 * 只有一个有价模型或全部同价时返回 0（视作最便宜），保证标签可判定。
 */
export function vendorPriceLevels(
  models: string[],
  inputOf: (name: string) => number | undefined
): Map<string, number> {
  const perVendor = new Map<string, { name: string; input: number }[]>();
  for (const name of models) {
    const input = inputOf(name);
    if (input === undefined || !Number.isFinite(input)) continue;
    const vendor = vendorOfModel(name);
    const list = perVendor.get(vendor) ?? [];
    list.push({ name, input });
    perVendor.set(vendor, list);
  }

  const levels = new Map<string, number>();
  for (const list of perVendor.values()) {
    const inputs = list.map((entry) => entry.input);
    const min = Math.min(...inputs);
    const max = Math.max(...inputs);
    const span = max > min ? max - min : 0;
    for (const entry of list) {
      levels.set(entry.name, span > 0 ? (entry.input - min) / span : 0);
    }
  }
  return levels;
}

export function computeTags(params: {
  name: string;
  release?: string;
  /** 该模型在所属厂家内的用量名次（1-based），无用量的模型不传 */
  rank?: number;
  /** 输入价在该模型所属厂家内的相对位置 0~1，无价格的不传 */
  level?: number;
  now?: number;
}): ModelTag[] {
  const tags: ModelTag[] = [];
  if (isNewRelease(params.release, params.now)) {
    tags.push({
      id: "new",
      label: "刚上新",
      className: "bg-blue-500/10 text-blue-700 dark:text-blue-400",
    });
  }
  if (params.rank !== undefined && params.rank <= HOT_RANK_LIMIT) {
    tags.push({
      id: "hot",
      label: "常用",
      className: "bg-purple-500/10 text-purple-700 dark:text-purple-400",
    });
  }
  if (
    params.rank !== undefined &&
    params.rank <= VALUE_RANK_LIMIT &&
    params.level !== undefined &&
    params.level < VALUE_LEVEL_LIMIT
  ) {
    tags.push({
      id: "value",
      label: "性价比",
      className: "bg-green-500/10 text-green-700 dark:text-green-400",
    });
  }
  return tags;
}
