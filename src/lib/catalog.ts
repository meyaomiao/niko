// 目录重建：momotoken 新版 bootstrap 里 groups[].models 与 models 可能为空，
// 真正的目录在 pricing（每个模型带 enable_groups）。这里做反查：
//   可用模型 = pricing 中 enable_groups 与账号可用分组有交集的那些；
//   该模型的可用的令牌分组 = enable_groups ∩ 账号可用分组。
// 服务端若没下发 enable_groups，则退回 groups[].models 的并集（旧版行为）。

import type { GroupOption, PricingItem } from "../api/client.ts";
import { vendorOfModel } from "./vendor.ts";

export interface CatalogModel {
  name: string;
  /** 该模型在账号可用分组内的令牌分组名 */
  groupNames: string[];
}

export interface ModelCatalog {
  models: CatalogModel[];
  /** 模型名 → 账号可用分组名 */
  groupsOf: (model: string) => string[];
  /** 目录来源，用于界面提示与排查 */
  source: "pricing" | "group-models" | "empty";
}

export function buildModelCatalog(
  pricing: PricingItem[] | undefined,
  accountGroups: GroupOption[]
): ModelCatalog {
  const accountNames = new Set(accountGroups.map((g) => g.name));
  const usable: CatalogModel[] = [];

  for (const item of pricing ?? []) {
    const enableGroups = item.enable_groups ?? [];
    if (!item.model_name || enableGroups.length === 0) continue;
    const groupNames = enableGroups.filter((name) => accountNames.has(name));
    if (groupNames.length > 0) usable.push({ name: item.model_name, groupNames });
  }

  if (usable.length > 0) {
    const index = new Map(usable.map((m) => [m.name, m.groupNames]));
    return {
      models: usable,
      groupsOf: (model) => index.get(model) ?? [],
      source: "pricing",
    };
  }

  // 旧响应：pricing 没有 enable_groups 时，退回分组里的模型列表
  const fromGroups = new Map<string, string[]>();
  for (const group of accountGroups) {
    for (const model of group.models ?? []) {
      const list = fromGroups.get(model) ?? [];
      list.push(group.name);
      fromGroups.set(model, list);
    }
  }
  const fallback: CatalogModel[] = [...fromGroups.entries()].map(([name, groupNames]) => ({
    name,
    groupNames,
  }));
  return {
    models: fallback,
    groupsOf: (model) => fromGroups.get(model) ?? [],
    source: fallback.length > 0 ? "group-models" : "empty",
  };
}

/** 按厂家给模型分桶（厂家名由服务端目录优先，缺失回退名称启发式） */
export function bucketByVendor(
  models: CatalogModel[],
  vendorNameOf: (model: string) => string
): { vendor: string; models: CatalogModel[] }[] {
  const buckets = new Map<string, CatalogModel[]>();
  for (const model of models) {
    const vendor = vendorNameOf(model.name);
    const list = buckets.get(vendor) ?? [];
    list.push(model);
    buckets.set(vendor, list);
  }
  return [...buckets.entries()]
    .map(([vendor, list]) => ({ vendor, models: list }))
    .sort((a, b) => b.models.length - a.models.length || a.vendor.localeCompare(b.vendor));
}

/** 本地启发式厂家名（服务端厂商表不可用时） */
export function heuristicVendor(model: string): string {
  return vendorOfModel(model);
}
