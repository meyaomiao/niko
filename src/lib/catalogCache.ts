// 首页与模型页共用的目录快照：避免同一份超大 pricing 被反复 IPC / JSON 解析。
import type { BootstrapData, PricingMeta } from "../api/client.ts";
import { loadAuth } from "../store/auth.ts";
import { isNewApiAuth, MOMOTOKEN_ORIGIN } from "./station.ts";

export interface CatalogSnapshot {
  bootstrap: BootstrapData;
  pricingMeta: PricingMeta;
  at: number;
}

export const EMPTY_PRICING_META: PricingMeta = {
  vendors: [],
  usableGroup: {},
  groupRatio: {},
};

const TTL_MS = 60_000;

let cached: CatalogSnapshot | null = null;
let cachedKey = "";
let inflight: Promise<CatalogSnapshot> | null = null;
let inflightKey = "";
let writeSeq = 0;

export function catalogCacheKey(origin: string, token: string): string {
  return `${origin}:${token}`;
}

export function currentCatalogKey(): string | null {
  const auth = loadAuth();
  if (!auth?.accessToken) return null;
  const origin = isNewApiAuth(auth) ? (auth.origin ?? "newapi") : MOMOTOKEN_ORIGIN;
  return catalogCacheKey(origin, auth.accessToken);
}

export function peekCatalogCache(key: string): CatalogSnapshot | null {
  if (!cached || cachedKey !== key) return null;
  if (Date.now() - cached.at > TTL_MS) return null;
  return cached;
}

export function peekCurrentCatalog(): CatalogSnapshot | null {
  const key = currentCatalogKey();
  if (!key || !cached || cachedKey !== key) return null;
  // 过期也先画出上一份目录，后台再刷新，避免模型页空白卡死。
  return cached;
}

export function clearCatalogCache(): void {
  cached = null;
  cachedKey = "";
  inflight = null;
  inflightKey = "";
  writeSeq += 1;
}

export async function loadCatalogCached(
  key: string,
  loader: () => Promise<Omit<CatalogSnapshot, "at">>,
  force = false,
): Promise<CatalogSnapshot> {
  if (!force) {
    const hit = peekCatalogCache(key);
    if (hit) return hit;
    if (inflight && inflightKey === key) return inflight;
  }

  const seq = ++writeSeq;
  const request = loader()
    .then((value) => {
      const snapshot: CatalogSnapshot = { ...value, at: Date.now() };
      if (seq === writeSeq) {
        cached = snapshot;
        cachedKey = key;
      }
      return snapshot;
    })
    .finally(() => {
      if (inflight === request) {
        inflight = null;
        inflightKey = "";
      }
    });

  inflight = request;
  inflightKey = key;
  return request;
}
