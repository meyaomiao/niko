import type { BootstrapModel, GroupOption, ModelMetadata } from "../api/client.ts";
import { vendorOfGroup, type Vendor } from "./vendor.ts";

export interface VendorModelChoice {
  name: string;
  groups: GroupOption[];
  releaseTime: number | null;
}

export interface VendorModelTab {
  vendor: Vendor;
  groups: GroupOption[];
  models: VendorModelChoice[];
}

interface ModelOrder {
  /** Server-provided catalog order is the primary display order. */
  catalogOrder: number | null;
  /** Used only when the server did not provide a catalog position. */
  releaseTime: number | null;
}

function nameOfModel(item: BootstrapModel): string {
  return typeof item === "string" ? item : (item.name ?? item.model_name ?? item.id ?? "");
}

function parseReleaseTime(meta: ModelMetadata | undefined): number | null {
  const raw = meta?.official_release_date ?? meta?.release_date ?? meta?.released_at ?? meta?.version_date;
  if (!raw) return null;
  const ms = Date.parse(raw);
  return Number.isFinite(ms) ? ms : null;
}

function buildModelOrder(
  models: BootstrapModel[] | undefined,
  metadata: Record<string, ModelMetadata> | undefined,
  modelOrder: string[] | undefined,
): Map<string, ModelOrder> {
  const order = new Map<string, ModelOrder>();
  for (const [index, name] of (modelOrder ?? []).entries()) {
    const normalized = name.trim();
    if (normalized) order.set(normalized, { catalogOrder: index, releaseTime: null });
  }
  for (const item of models ?? []) {
    const name = nameOfModel(item);
    if (!name) continue;
    const meta = typeof item === "string" ? metadata?.[name] : { ...metadata?.[name], ...item };
    const existing = order.get(name);
    order.set(name, {
      // Never let model metadata override the server's explicit order.
      catalogOrder: existing?.catalogOrder ?? meta?.catalog_order ?? null,
      releaseTime: parseReleaseTime(meta) ?? existing?.releaseTime ?? null,
    });
  }
  for (const [name, meta] of Object.entries(metadata ?? {})) {
    const existing = order.get(name);
    order.set(name, {
      catalogOrder: existing?.catalogOrder ?? meta.catalog_order ?? null,
      releaseTime: parseReleaseTime(meta) ?? existing?.releaseTime ?? null,
    });
  }
  return order;
}

function compareModels(a: VendorModelChoice, b: VendorModelChoice, order: Map<string, ModelOrder>) {
  const aOrder = order.get(a.name);
  const bOrder = order.get(b.name);
  const aCatalog = aOrder?.catalogOrder ?? null;
  const bCatalog = bOrder?.catalogOrder ?? null;
  if (aCatalog !== null || bCatalog !== null) {
    if (aCatalog === null) return 1;
    if (bCatalog === null) return -1;
    if (aCatalog !== bCatalog) return aCatalog - bCatalog;
  }
  const aRelease = aOrder?.releaseTime ?? a.releaseTime;
  const bRelease = bOrder?.releaseTime ?? b.releaseTime;
  if (aRelease !== null || bRelease !== null) {
    if (aRelease === null) return 1;
    if (bRelease === null) return -1;
    if (aRelease !== bRelease) return bRelease - aRelease;
  }
  return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
}

function compareGroups(a: GroupOption, b: GroupOption): number {
  return a.ratio - b.ratio || (a.name < b.name ? -1 : a.name > b.name ? 1 : 0);
}

export function buildVendorModelTabs(params: {
  groups: GroupOption[];
  models?: BootstrapModel[];
  modelMetadata?: Record<string, ModelMetadata>;
  modelOrder?: string[];
  recommendVendor?: Vendor | null;
}): VendorModelTab[] {
  const { groups, models, modelMetadata, modelOrder, recommendVendor } = params;
  const order = buildModelOrder(models, modelMetadata, modelOrder);
  const buckets = new Map<Vendor, { groups: GroupOption[]; models: Map<string, VendorModelChoice> }>();

  for (const group of groups) {
    const vendor = vendorOfGroup(group.name);
    let bucket = buckets.get(vendor);
    if (!bucket) {
      bucket = { groups: [], models: new Map() };
      buckets.set(vendor, bucket);
    }
    bucket.groups.push(group);
    for (const model of group.models) {
      // Groups can contain a custom model absent from the bootstrap catalog.
      // It has no server position or release metadata, so it uses the stable
      // model-name fallback in compareModels.
      if (!order.has(model)) {
        order.set(model, { catalogOrder: null, releaseTime: null });
      }
      const existing = bucket.models.get(model);
      if (existing) {
        existing.groups.push(group);
      } else {
        bucket.models.set(model, {
          name: model,
          groups: [group],
          releaseTime: order.get(model)?.releaseTime ?? null,
        });
      }
    }
  }

  return Array.from(buckets.entries())
    .map(([vendor, bucket]) => ({
      vendor,
      groups: [...bucket.groups].sort(compareGroups),
      models: Array.from(bucket.models.values())
        .map((choice) => ({ ...choice, groups: [...choice.groups].sort(compareGroups) }))
        .sort((a, b) => compareModels(a, b, order)),
    }))
    .sort((a, b) => Number(b.vendor === recommendVendor) - Number(a.vendor === recommendVendor));
}
