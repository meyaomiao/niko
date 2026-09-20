import { localDate, UsageLoadError } from "./usageLedger.ts";
import type {
  BootstrapData,
  GroupOption,
  PricingItem,
  PricingMeta,
  UsageLogItem,
  UsageSummary,
  VendorMeta,
} from "../api/client.ts";

export interface NewApiSnapshot {
  origin: string;
  status: unknown;
  user: unknown;
  groups: unknown;
  models: unknown;
  pricing: unknown;
}

function asRecord(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function unwrapData(value: unknown): unknown {
  const record = asRecord(value);
  if (!record || !("data" in record)) return value;
  // Rust 侧 get_json 已经拆过 envelope；前端再遇到 success:false 才当成失败。
  if (record.success === false) {
    const message = typeof record.message === "string" && record.message.trim()
      ? record.message
      : "请求失败";
    throw new Error(message);
  }
  return record.data;
}

function asString(value: unknown, fallback = ""): string {
  return typeof value === "string" ? value : fallback;
}

function asNumber(value: unknown, fallback = 0): number {
  if (typeof value === "number" && Number.isFinite(value)) return value;
  if (typeof value === "string" && value.trim()) {
    const parsed = Number(value);
    if (Number.isFinite(parsed)) return parsed;
  }
  return fallback;
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  return value.map((item) => asString(item)).filter(Boolean);
}

export function pricingFromSnapshot(pricing: unknown): {
  items: PricingItem[];
  meta: PricingMeta;
} {
  const root = asRecord(pricing) ?? {};
  const data = unwrapData(pricing);
  const rows = Array.isArray(data) ? data : [];
  const items: PricingItem[] = [];
  for (const row of rows) {
    const record = asRecord(row);
    if (!record) continue;
    const modelName = asString(record.model_name ?? record.modelName);
    if (!modelName) continue;
    items.push({
      model_name: modelName,
      quota_type: asNumber(record.quota_type ?? record.quotaType),
      model_ratio: asNumber(record.model_ratio ?? record.modelRatio, 1),
      model_price: asNumber(record.model_price ?? record.modelPrice),
      completion_ratio: asNumber(record.completion_ratio ?? record.completionRatio, 1),
      cache_ratio: record.cache_ratio == null && record.cacheRatio == null
        ? undefined
        : asNumber(record.cache_ratio ?? record.cacheRatio),
      create_cache_ratio: record.create_cache_ratio == null && record.createCacheRatio == null
        ? undefined
        : asNumber(record.create_cache_ratio ?? record.createCacheRatio),
      enable_groups: stringList(record.enable_groups ?? record.enableGroups),
      release_date: asString(record.release_date ?? record.releaseDate) || undefined,
      release_source: asString(record.release_source ?? record.releaseSource) || undefined,
      vendor_id: record.vendor_id == null && record.vendorId == null
        ? undefined
        : asNumber(record.vendor_id ?? record.vendorId),
      supported_endpoint_types: stringList(
        record.supported_endpoint_types ?? record.supportedEndpointTypes,
      ),
      tags: asString(record.tags) || undefined,
      description: asString(record.description) || undefined,
      billing_mode: asString(record.billing_mode ?? record.billingMode) || undefined,
    });
  }

  const vendorsRaw = Array.isArray(root.vendors) ? root.vendors : [];
  const vendors: VendorMeta[] = vendorsRaw.flatMap((row) => {
    const record = asRecord(row);
    if (!record) return [];
    const name = asString(record.name);
    if (!name) return [];
    return [{
      id: asNumber(record.id),
      name,
      description: asString(record.description) || undefined,
      icon: asString(record.icon) || undefined,
    }];
  });

  const usableGroup: Record<string, string> = {};
  const usableRaw = asRecord(root.usable_group ?? root.usableGroup);
  if (usableRaw) {
    for (const [name, desc] of Object.entries(usableRaw)) {
      usableGroup[name] = asString(desc, name);
    }
  }

  const groupRatio: Record<string, number> = {};
  const ratioRaw = asRecord(root.group_ratio ?? root.groupRatio);
  if (ratioRaw) {
    for (const [name, ratio] of Object.entries(ratioRaw)) {
      groupRatio[name] = asNumber(ratio, 1);
    }
  }

  return { items, meta: { vendors, usableGroup, groupRatio } };
}

function groupsFromSnapshot(groups: unknown, models: unknown): GroupOption[] {
  const data = unwrapData(groups);
  const record = asRecord(data);
  const modelMap = asRecord(unwrapData(models));
  const flatModels = Array.isArray(unwrapData(models)) ? stringList(unwrapData(models)) : [];
  if (!record) return [];

  const options: GroupOption[] = [];
  for (const [name, value] of Object.entries(record)) {
    const info = asRecord(value) ?? {};
    const mappedModels = stringList(modelMap?.[name]);
    options.push({
      name,
      desc: asString(info.desc ?? info.description),
      ratio: asNumber(info.ratio, 1),
      models: mappedModels.length > 0 ? mappedModels : flatModels,
    });
  }
  return options;
}

function modelsFromSnapshot(models: unknown, groups: GroupOption[]): string[] {
  const data = unwrapData(models);
  if (Array.isArray(data)) return stringList(data);
  const fromGroups = [...new Set(groups.flatMap((group) => group.models ?? []))];
  if (fromGroups.length > 0) return fromGroups;
  const record = asRecord(data);
  if (!record) return [];
  return [...new Set(Object.values(record).flatMap((value) => stringList(value)))];
}

export function assembleNewApiCatalog(snapshot: NewApiSnapshot): {
  bootstrap: BootstrapData;
  pricingMeta: PricingMeta;
} {
  const priced = pricingFromSnapshot(snapshot.pricing);
  return {
    bootstrap: assembleNewApiBootstrapFromParts(snapshot, priced.items),
    pricingMeta: priced.meta,
  };
}

export function assembleNewApiBootstrap(snapshot: NewApiSnapshot): BootstrapData {
  return assembleNewApiBootstrapFromParts(snapshot, pricingFromSnapshot(snapshot.pricing).items);
}

function assembleNewApiBootstrapFromParts(snapshot: NewApiSnapshot, pricing: PricingItem[]): BootstrapData {
  const status = asRecord(unwrapData(snapshot.status)) ?? asRecord(snapshot.status) ?? {};
  const user = asRecord(unwrapData(snapshot.user)) ?? {};
  let groups = groupsFromSnapshot(snapshot.groups, snapshot.models);
  const models = modelsFromSnapshot(snapshot.models, groups);
  const userGroups = asString(user.group)
    .split(",")
    .map((name) => name.trim())
    .filter(Boolean);
  for (const name of userGroups) {
    if (!groups.some((group) => group.name === name)) {
      groups = [...groups, { name, desc: "", ratio: 1, models }];
    }
  }
  const userGroup = userGroups[0] ?? groups[0]?.name ?? "";
  if (groups.length === 0) {
    groups = [{ name: userGroup || "default", desc: "", ratio: 1, models }];
  }
  const quotaPerUnit = status.quota_per_unit ?? status.quotaPerUnit;

  return {
    site: {
      base_url: snapshot.origin,
      system_name: asString(status.system_name ?? status.systemName, "new-api"),
      server_version: asString(status.version ?? status.server_version ?? status.serverVersion),
      quota_per_unit: quotaPerUnit as number | string | undefined,
    },
    user: {
      id: asNumber(user.id),
      quota: asNumber(user.quota),
      group: userGroup || groups[0]?.name || "",
    },
    models,
    pricing,
    groups,
    model_order: models,
  };
}

export function assembleNewApiPricingMeta(pricing: unknown): PricingMeta {
  return pricingFromSnapshot(pricing).meta;
}

export function mapNewApiLogs(payload: unknown): { items: UsageLogItem[]; total?: number } {
  const data = unwrapData(payload);
  const record = asRecord(data);
  const rows = Array.isArray(data)
    ? data
    : Array.isArray(record?.items)
      ? record.items
      : Array.isArray(record?.data)
        ? record.data
        : record?.items === null ? [] : null;
  if (!rows) throw new UsageLoadError("站点返回的用量记录格式无效。");
  const items: UsageLogItem[] = [];
  for (const row of rows) {
    const item = asRecord(row);
    if (!item) throw new UsageLoadError("站点返回的用量记录格式无效。");
    items.push({
      id: asNumber(item.id),
      created_at: asNumber(item.created_at ?? item.createdAt, NaN),
      model_name: asString(item.model_name ?? item.modelName),
      prompt_tokens: asNumber(item.prompt_tokens ?? item.promptTokens),
      completion_tokens: asNumber(item.completion_tokens ?? item.completionTokens),
      quota: asNumber(item.quota, NaN),
      group: asString(item.group) || undefined,
      use_time: item.use_time == null && item.useTime == null
        ? undefined
        : asNumber(item.use_time ?? item.useTime),
      is_stream: typeof item.is_stream === "boolean"
        ? item.is_stream
        : typeof item.isStream === "boolean"
          ? item.isStream
          : undefined,
    });
  }
  const total = record && (record.total != null || record.Total != null)
    ? asNumber(record.total ?? record.Total, NaN)
    : undefined;
  return { items, total };
}

export function summarizeNewApiLogs(items: UsageLogItem[] | null | undefined): UsageSummary {
  const list = items ?? [];
  const byModel = new Map<string, { quota: number; prompt_tokens: number; completion_tokens: number; requests: number }>();
  const byGroup = new Map<string, { quota: number; prompt_tokens: number; completion_tokens: number; requests: number }>();
  const byDay = new Map<string, { quota: number; tokens: number; requests: number }>();
  let quota = 0;
  let prompt = 0;
  let completion = 0;
  let stream = 0;
  let useTime = 0;

  for (const item of list) {
    quota += item.quota;
    prompt += item.prompt_tokens;
    completion += item.completion_tokens;
    if (item.is_stream) stream += 1;
    useTime += item.use_time ?? 0;

    const model = byModel.get(item.model_name) ?? { quota: 0, prompt_tokens: 0, completion_tokens: 0, requests: 0 };
    model.quota += item.quota;
    model.prompt_tokens += item.prompt_tokens;
    model.completion_tokens += item.completion_tokens;
    model.requests += 1;
    byModel.set(item.model_name, model);

    if (item.group) {
      const group = byGroup.get(item.group) ?? { quota: 0, prompt_tokens: 0, completion_tokens: 0, requests: 0 };
      group.quota += item.quota;
      group.prompt_tokens += item.prompt_tokens;
      group.completion_tokens += item.completion_tokens;
      group.requests += 1;
      byGroup.set(item.group, group);
    }

    if (item.created_at) {
      const date = localDate(item.created_at);
      const day = byDay.get(date) ?? { quota: 0, tokens: 0, requests: 0 };
      day.quota += item.quota;
      day.tokens += item.prompt_tokens + item.completion_tokens;
      day.requests += 1;
      byDay.set(date, day);
    }
  }

  return {
    quota,
    prompt_tokens: prompt,
    completion_tokens: completion,
    requests: list.length,
    stream_requests: stream,
    total_use_time: useTime,
    by_model: [...byModel.entries()].map(([name, value]) => ({ name, ...value })).sort((a, b) => b.quota - a.quota),
    by_group: [...byGroup.entries()].map(([name, value]) => ({ name, ...value })).sort((a, b) => b.quota - a.quota),
    by_day: [...byDay.entries()].map(([date, value]) => ({ date, ...value })).sort((a, b) => a.date.localeCompare(b.date)),
  };
}
