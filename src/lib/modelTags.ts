// 模型标签：从已有数据推导，不造数据
// - 刚上新：官方发布日期在 60 天内
// - 性价比：输入价位于全目录下 1/3 分位
// - 常用：出现在当前账号用量聚合的前几名（服务端 usage/summary）

import type { UsageSummary } from "../api/client";

export interface ModelTag {
  id: "new" | "value" | "hot";
  label: string;
  className: string;
}

const NEW_MS = 60 * 86400_000;

export function isNewRelease(release: string | undefined, now = Date.now()): boolean {
  if (!release) return false;
  const ts = Date.parse(release);
  if (!Number.isFinite(ts)) return false;
  return now - ts >= 0 && now - ts <= NEW_MS;
}

/** 从用量聚合取用户最常用的模型名（按请求数排序，默认前 3） */
export function topUsedModels(summary: UsageSummary | null | undefined, limit = 3): Set<string> {
  const byModel = summary?.by_model;
  if (!byModel?.length) return new Set();
  return new Set(
    [...byModel]
      .sort((a, b) => b.requests - a.requests)
      .slice(0, limit)
      .map((d) => d.name)
      .filter(Boolean)
  );
}

export function computeTags(params: {
  release?: string;
  /** 输入价在全部有价模型中的分位 0~1；无价格时为 undefined */
  level?: number;
  topUsed?: Set<string>;
  name: string;
}): ModelTag[] {
  const tags: ModelTag[] = [];
  if (isNewRelease(params.release)) {
    tags.push({
      id: "new",
      label: "刚上新",
      className: "bg-blue-500/10 text-blue-700 dark:text-blue-400",
    });
  }
  if (params.topUsed?.has(params.name)) {
    tags.push({
      id: "hot",
      label: "常用",
      className: "bg-purple-500/10 text-purple-700 dark:text-purple-400",
    });
  }
  if (params.level !== undefined && params.level <= 1 / 3) {
    tags.push({
      id: "value",
      label: "性价比",
      className: "bg-green-500/10 text-green-700 dark:text-green-400",
    });
  }
  return tags;
}
