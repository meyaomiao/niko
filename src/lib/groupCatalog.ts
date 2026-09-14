// 分组目录：模型的「令牌分组 / API 分组」来自服务端 pricing[].enable_groups。
// bootstrap 的 groups 只是「当前账号可用的分组」（通常只有自己的那一个），
// 不能拿它当模型的分组列表——那会永远只显示一个。
// 分组名称与说明来自公开 /api/pricing 的 usable_group，倍率优先用 bootstrap 的
// 账号专属倍率，其次用公开 group_ratio。

import type { GroupOption, PricingItem } from "../api/client.ts";

export interface GroupInfo {
  name: string;
  desc: string;
  ratio: number;
  /** 当前账号是否可以在这个分组上申请密钥（bootstrap.groups 里有才算） */
  usable: boolean;
}

export function buildGroupCatalog(params: {
  /** bootstrap.groups：账号可用分组（含账号专属倍率） */
  accountGroups: GroupOption[];
  /** 公开 /api/pricing 的 usable_group：全部分组的中文说明 */
  usableGroup?: Record<string, string>;
  /** 公开 /api/pricing 的 group_ratio：公开倍率 */
  groupRatio?: Record<string, number>;
}): Map<string, GroupInfo> {
  const { accountGroups, usableGroup, groupRatio } = params;
  const catalog = new Map<string, GroupInfo>();

  for (const [name, desc] of Object.entries(usableGroup ?? {})) {
    catalog.set(name, {
      name,
      desc: desc || "",
      ratio: groupRatio?.[name] ?? 1,
      usable: false,
    });
  }

  for (const group of accountGroups) {
    const existing = catalog.get(group.name);
    catalog.set(group.name, {
      name: group.name,
      desc: group.desc || existing?.desc || "",
      ratio: group.ratio ?? existing?.ratio ?? 1,
      usable: true,
    });
  }

  return catalog;
}

/** 某模型支持的令牌分组（无 enable_groups 时退回账号可用分组里的交集） */
export function groupsForModel(
  catalog: Map<string, GroupInfo>,
  pricing: PricingItem[] | undefined,
  model: string,
  accountGroups: GroupOption[]
): GroupInfo[] {
  const item = pricing?.find((entry) => entry.model_name === model);
  const names = item?.enable_groups?.length
    ? item.enable_groups
    : accountGroups.filter((g) => (g.models ?? []).includes(model)).map((g) => g.name);

  return names
    .map((name) => {
      const info = catalog.get(name);
      if (info) return info;
      // 目录里没有的分组：用账号分组数据补，或给一个最小信息
      const account = accountGroups.find((g) => g.name === name);
      return {
        name,
        desc: account?.desc ?? "",
        ratio: account?.ratio ?? 1,
        usable: Boolean(account),
      } satisfies GroupInfo;
    })
    .sort((a, b) => Number(b.usable) - Number(a.usable) || a.ratio - b.ratio || a.name.localeCompare(b.name));
}
