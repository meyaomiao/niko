import type { BootstrapData, ModelMetadata, PricingItem } from "../api/client.ts";
import type { CatalogSnapshot } from "./catalogCache.ts";
import { buildModelCatalog } from "./catalog.ts";

function modelName(model: BootstrapData["models"][number]): string {
  return typeof model === "string" ? model : model.name ?? model.model_name ?? model.id ?? "";
}

/** Refresh the public catalog without replacing account permissions, balance or application settings. */
export function mergeOfficialCatalog(account: BootstrapData, current: PricingItem[]): BootstrapData {
  const previous = new Map((account.pricing ?? []).map((item) => [item.model_name, item]));
  const metadata = new Map<string, ModelMetadata>();
  for (const model of account.models ?? []) {
    if (typeof model !== "string") metadata.set(modelName(model), model);
  }
  for (const [name, value] of Object.entries(account.model_metadata ?? {})) metadata.set(name, value);

  const rows = new Map<string, PricingItem>();
  for (const item of current) {
    const old = previous.get(item.model_name);
    // Absent optional metadata may be supplied by the account endpoint, but an explicit
    // empty enable_groups array is authoritative and must never restore old permissions.
    const defined = Object.fromEntries(Object.entries(item).filter(([, value]) => value !== undefined));
    const merged = { ...old, ...defined } as PricingItem;
    if (!Array.isArray(merged.enable_groups)) {
      merged.enable_groups = (account.groups ?? [])
        .filter((group) => group.models?.includes(item.model_name))
        .map((group) => group.name);
    }
    rows.set(item.model_name, merged);
    const releaseDate = modelReleaseDate(undefined, item.release_date);
    if (releaseDate) {
      metadata.set(item.model_name, {
        ...metadata.get(item.model_name),
        release_date: releaseDate,
        ...(item.release_source ? { release_source: item.release_source } : {}),
      });
    }
  }
  const pricing = [...rows.values()];
  const names = new Set(rows.keys());
  const order = [...new Set([...(account.model_order ?? []).filter((name) => names.has(name)), ...names])];
  const allowed = buildModelCatalog(pricing, account.groups ?? [], { authoritative: true });
  return {
    ...account,
    models: allowed.models.map((model) => model.name),
    pricing,
    pricing_authoritative: true,
    groups: account.groups?.map((group) => ({
      ...group,
      models: allowed.models.filter((model) => model.groupNames.includes(group.name)).map((model) => model.name),
    })),
    model_metadata: Object.fromEntries([...metadata].filter(([name]) => names.has(name))),
    model_order: order,
  };
}

/** Empty or malformed older metadata must not hide a real release date from current pricing. */
export function modelReleaseDate(meta: ModelMetadata | undefined, fallback?: string): string {
  for (const value of [meta?.release_date, meta?.released_at, meta?.official_release_date, meta?.version_date, fallback]) {
    if (typeof value === "string" && value.trim() && Number.isFinite(Date.parse(value))) return value.trim();
  }
  return "";
}

export function catalogSyncLabel(snapshot: Pick<CatalogSnapshot, "at" | "sync"> | null): string {
  if (!snapshot) return "模型目录待同步";
  const syncedAt = snapshot.sync?.at;
  const time = new Date(syncedAt ?? snapshot.at).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" });
  if (snapshot.sync?.source === "account") {
    return syncedAt ? `在线目录同步失败，保留 ${time} 的目录` : "在线模型目录暂未同步，当前显示账号目录";
  }
  return `目录已同步 ${time}`;
}
