// momotoken 登录器 API 客户端

import { invoke } from "@tauri-apps/api/core";
import { APP_VERSION } from "../lib/version";
import {
  assembleNewApiCatalog,
  assembleNewApiPricingMeta,
  mapNewApiLogs,
  summarizeNewApiLogs,
  type NewApiSnapshot,
} from "../lib/newapi";
import {
  catalogCacheKey,
  clearCatalogCache,
  EMPTY_PRICING_META,
  loadCatalogCached,
  peekCatalogCache,
  type CatalogSnapshot,
} from "../lib/catalogCache";
import { currentDeviceName, isNewApiAuth, MOMOTOKEN_ORIGIN } from "../lib/station";
import { loadAuth } from "../store/auth";

const BASE_URL = MOMOTOKEN_ORIGIN;
const API_REQUEST_TIMEOUT_MS = 10_000;

function newApiSession() {
  const auth = loadAuth();
  if (!isNewApiAuth(auth) || !auth?.origin) {
    throw new Error("尚未连接中转站");
  }
  return {
    origin: auth.origin,
    access_token: auth.accessToken,
    user_id: auth.userId,
  };
}

function catalogKeyForAuth(): string | null {
  const auth = loadAuth();
  if (!auth?.accessToken) return null;
  const origin = isNewApiAuth(auth) ? (auth.origin ?? "newapi") : MOMOTOKEN_ORIGIN;
  return catalogCacheKey(origin, auth.accessToken);
}

async function fetchCatalogSnapshot(token: string): Promise<Omit<CatalogSnapshot, "at">> {
  const auth = loadAuth();
  if (isNewApiAuth(auth)) {
    const snapshot = await invoke<NewApiSnapshot>("newapi_bootstrap", { req: newApiSession() });
    return assembleNewApiCatalog(snapshot);
  }
  const [bootstrap, pricingMeta] = await Promise.all([
    get<BootstrapData>("/client/bootstrap", token),
    fetchPublicPricingMeta().catch(() => EMPTY_PRICING_META),
  ]);
  return { bootstrap, pricingMeta };
}

async function fetchPublicPricingMeta(): Promise<PricingMeta> {
  if (isNewApiAuth(loadAuth())) {
    const json = await invoke<unknown>("newapi_pricing", { origin: newApiSession().origin });
    return assembleNewApiPricingMeta(json);
  }
  const res = await fetch(`${BASE_URL}/api/pricing`);
  if (!res.ok) throw new Error(`pricing ${res.status}`);
  const json = (await res.json()) as {
    vendors?: VendorMeta[];
    usable_group?: Record<string, string>;
    group_ratio?: Record<string, number>;
  };
  return {
    vendors: json.vendors ?? [],
    usableGroup: json.usable_group ?? {},
    groupRatio: json.group_ratio ?? {},
  };
}

export interface SiteConfig {
  system_name: string;
  server_version: string;
}

export interface StatusData {
  quota_per_unit: number | string;
}

export interface LoginResult {
  require_2fa: boolean;
  pending_token?: string;
  expires_in?: number;
  access_token?: string;
  username?: string;
  email?: string;
}

/** 设备数达上限时后端回带的设备列表，登录页据此让用户当场释放旧设备 */
export class DeviceLimitError extends Error {
  constructor(
    message: string,
    readonly deviceLimit: number,
    readonly devices: DeviceItem[]
  ) {
    super(message);
    this.name = "DeviceLimitError";
  }
}

// 后端 /api/client/login 的原始返回：成功时是 session_token + 嵌套 user，
// 需要 2FA 时是 require_2fa + pending_token。
interface RawLoginResponse {
  require_2fa?: boolean;
  pending_token?: string;
  expires_in?: number;
  session_token?: string;
  user?: { username?: string; email?: string };
}

function toLoginResult(raw: RawLoginResponse): LoginResult {
  return {
    require_2fa: raw.require_2fa === true,
    pending_token: raw.pending_token,
    expires_in: raw.expires_in,
    access_token: raw.session_token,
    username: raw.user?.username,
    email: raw.user?.email,
  };
}

export interface GroupOption {
  name: string;
  desc: string;
  ratio: number;
  models: string[];
}

export interface ModelMetadata {
  name?: string;
  model_name?: string;
  id?: string;
  /** Official release date/version date used for client sorting; YYYY-MM-DD is preferred. */
  release_date?: string;
  released_at?: string;
  official_release_date?: string;
  version_date?: string;
  /** Stable server catalog position used when an explicit model_order is unavailable. */
  catalog_order?: number;
  /** Optional traceable source label or URL supplied by the server/admin panel. */
  release_source?: string;
  source?: string;
}

/** A string entry keeps the existing API compatible while metadata is adopted. */
export type BootstrapModel = string | ModelMetadata;

export interface PricingItem {
  model_name: string;
  quota_type: number;
  model_ratio: number;
  model_price: number;
  completion_ratio: number;
  cache_ratio?: number | null;
  create_cache_ratio?: number | null;
  enable_groups?: string[];
  /** Optional official release metadata mirrored by bootstrap model catalog. */
  release_date?: string;
  release_source?: string;
  /** 服务端厂商归属（新版目录下发），用于「厂家 → 模型」归类 */
  vendor_id?: number;
  /** 端点能力：openai / anthropic / gemini / image-generation ... */
  supported_endpoint_types?: string[];
  tags?: string;
  description?: string;
  billing_mode?: string;
}

/** 全站模型用量排行条目 */
export interface ModelUsageStat {
  model_name: string;
  requests: number;
  quota: number;
}

/** 厂商元数据（来自公开 /api/pricing 的 vendors 数组） */
export interface VendorMeta {
  id: number;
  name: string;
  description?: string;
  icon?: string;
}

/** 公开定价元数据：厂商表 + 分组说明/倍率 */
export interface PricingMeta {
  vendors: VendorMeta[];
  usableGroup: Record<string, string>;
  groupRatio: Record<string, number>;
}

export interface BootstrapData {
  site: {
    base_url: string;
    system_name: string;
    server_version: string;
    quota_per_unit?: number | string;
  };
  user: { id: number; quota: number; group: string };
  models: BootstrapModel[];
  /** Keyed official release metadata. The key must match the model name. */
  model_metadata?: Record<string, ModelMetadata>;
  /** Explicit server display order for all enabled models. */
  model_order?: string[];
  groups?: GroupOption[];
  pricing: PricingItem[];
  /** 全站模型用量排行（服务端跨用户聚合，最近 30 天） */
  model_usage?: ModelUsageStat[];
  /** 允许的最大登录设备数，用于首页展示 已用/上限 */
  device_limit?: number;
  min_supported_version?: string;
  latest_version?: string;
  download_url?: string;
  announcement?: { content?: string; publish?: string } | null;
}

async function post<T>(path: string, body: unknown, token?: string): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const controller = new AbortController();
  const timeoutId = setTimeout(() => controller.abort(), API_REQUEST_TIMEOUT_MS);
  try {
    const res = await fetch(`${BASE_URL}/api${path}`, {
      method: "POST",
      headers,
      body: JSON.stringify(body),
      signal: controller.signal,
    });
    const json = await res.json() as { success: boolean; message?: string; data?: T };
    if (!json.success) throw new Error(json.message ?? "请求失败");
    return (json.data ?? json) as T;
  } catch (error) {
    if (controller.signal.aborted) {
      throw new Error("模型服务请求超时，请检查网络或服务状态后重试");
    }
    throw error;
  } finally {
    clearTimeout(timeoutId);
  }
}

/** 登录请求：需要在失败分支识别 E_DEVICE_LIMIT 并取出回带的设备列表 */
async function postLogin(path: string, body: unknown): Promise<LoginResult> {
  const res = await fetch(`${BASE_URL}/api${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const json = (await res.json()) as {
    success: boolean;
    message?: string;
    code?: string;
    data?: RawLoginResponse & { device_limit?: number; active_devices?: DeviceItem[] | null };
  };
  if (!json.success) {
    if (json.code === "E_DEVICE_LIMIT") {
      throw new DeviceLimitError(
        json.message ?? "设备数已达上限",
        json.data?.device_limit ?? 0,
        json.data?.active_devices ?? []
      );
    }
    throw new Error(json.message ?? "登录失败");
  }
  return toLoginResult((json.data ?? {}) as RawLoginResponse);
}

async function get<T>(path: string, token?: string): Promise<T> {
  const headers: Record<string, string> = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${BASE_URL}/api${path}`, { headers, cache: "no-store" });
  const json = await res.json() as { success: boolean; message?: string; data?: T };
  if (!json.success) throw new Error(json.message ?? "请求失败");
  return (json.data ?? json) as T;
}

export interface UsageQuery {
  page?: number;
  pageSize?: number;
  startTimestamp?: number;
  endTimestamp?: number;
  group?: string;
  /** 按厂商筛选时传入该厂商的模型名列表，日志列表侧只取第一个模型精确匹配 */
  models?: string[];
  modelName?: string;
}

function usageQueryString(params: UsageQuery): string {
  const q = new URLSearchParams();
  q.set("p", String(params.page ?? 1));
  if (params.pageSize) q.set("page_size", String(params.pageSize));
  if (params.startTimestamp) q.set("start_timestamp", String(params.startTimestamp));
  if (params.endTimestamp) q.set("end_timestamp", String(params.endTimestamp));
  if (params.group) q.set("group", params.group);
  if (params.modelName) q.set("model_name", params.modelName);
  if (params.models?.length) q.set("models", params.models.join(","));
  return q.toString();
}

export interface PayMethod {
  name: string;
  type: string;
  color?: string;
  tag?: string;
  tag_color?: string;
  min_topup?: string;
}

export interface TopUpInfo {
  enable_online_topup: boolean;
  pay_methods: PayMethod[] | null;
  min_topup: number;
  amount_options?: number[] | null;
  discount?: Record<string, number> | null;
  topup_link?: string;
}

export interface TopUpRecord {
  id: number;
  amount: number;
  money: number;
  trade_no: string;
  payment_method: string;
  create_time: number;
  complete_time: number;
  status: string;
}

/** 易支付下单返回 {message,data,url}，与常规 {success,data} 格式不同，需单独解析 */
export interface EpayOrder {
  url: string;
  params: Record<string, unknown>;
}

export const api = {
  async status(): Promise<StatusData> {
    if (isNewApiAuth(loadAuth())) {
      const data = await invoke<StatusData>("newapi_status", { origin: newApiSession().origin });
      return data;
    }
    return get<StatusData>("/status");
  },
  /** 公开定价接口：厂商表 + 全部分组说明与倍率。登录后优先复用目录缓存，避免再拉一份超大 pricing。 */
  async pricingMeta(): Promise<PricingMeta> {
    const key = catalogKeyForAuth();
    if (key) {
      const hit = peekCatalogCache(key);
      if (hit) return hit.pricingMeta;
      try {
        const auth = loadAuth();
        return (await loadCatalogCached(key, () => fetchCatalogSnapshot(auth?.accessToken ?? ""))).pricingMeta;
      } catch {
        /* 缓存 miss 时仍走公开接口 */
      }
    }
    return fetchPublicPricingMeta();
  },
  getSite(): Promise<SiteConfig> {
    return get<SiteConfig>("/client/site");
  },
  connectNewApi(params: {
    origin: string;
    accessToken: string;
    userId?: number;
  }): Promise<{
    origin: string;
    user_id: number;
    username: string;
    quota: number;
    group: string;
    quota_per_unit?: number | string;
    system_name: string;
  }> {
    return invoke("newapi_connect", {
      req: {
        origin: params.origin,
        access_token: params.accessToken,
        user_id: params.userId ?? null,
      },
    });
  },
  login(params: {
    username: string;
    password: string;
    deviceId: string;
    deviceName: string;
    platform: string;
    revokeSessionIds?: number[];
  }): Promise<LoginResult> {
    return postLogin("/client/login", {
      username: params.username,
      password: params.password,
      device_id: params.deviceId,
      device_name: params.deviceName,
      platform: params.platform,
      app_version: APP_VERSION,
      revoke_session_ids: params.revokeSessionIds,
    });
  },
  login2fa(
    pendingToken: string,
    code: string,
    revokeSessionIds?: number[]
  ): Promise<LoginResult> {
    return postLogin("/client/login/2fa", {
      pending_token: pendingToken,
      code,
      revoke_session_ids: revokeSessionIds,
    });
  },
  logout(token: string): Promise<void> {
    if (isNewApiAuth(loadAuth())) return Promise.resolve();
    return post<void>("/client/logout", {}, token);
  },
  async bootstrap(token: string, options?: { force?: boolean }): Promise<BootstrapData> {
    const auth = loadAuth();
    const origin = isNewApiAuth(auth) ? (auth?.origin ?? "newapi") : MOMOTOKEN_ORIGIN;
    const key = catalogCacheKey(origin, token);
    const snapshot = await loadCatalogCached(key, () => fetchCatalogSnapshot(token), options?.force === true);
    return snapshot.bootstrap;
  },
  async catalog(token: string, options?: { force?: boolean }): Promise<CatalogSnapshot> {
    const auth = loadAuth();
    const origin = isNewApiAuth(auth) ? (auth?.origin ?? "newapi") : MOMOTOKEN_ORIGIN;
    return loadCatalogCached(catalogCacheKey(origin, token), () => fetchCatalogSnapshot(token), options?.force === true);
  },
  clearCatalog(): void {
    clearCatalogCache();
  },
  async provision(token: string, group: string): Promise<{ api_key: string; token_id: number; group: string }> {
    if (isNewApiAuth(loadAuth())) {
      return invoke("newapi_provision", {
        req: {
          ...newApiSession(),
          group,
          device_name: currentDeviceName(),
        },
      });
    }
    return post("/client/provision", { group }, token);
  },
  listDevices(token: string): Promise<DeviceItem[]> {
    if (isNewApiAuth(loadAuth())) return Promise.resolve([]);
    return get<DeviceItem[]>("/client/devices", token);
  },
  async usage(
    token: string,
    params: UsageQuery = {}
  ): Promise<{ items: UsageLogItem[] | null; total?: number }> {
    if (isNewApiAuth(loadAuth())) {
      const payload = await invoke<unknown>("newapi_usage", {
        req: {
          ...newApiSession(),
          page: params.page,
          page_size: params.pageSize,
          start_timestamp: params.startTimestamp,
          end_timestamp: params.endTimestamp,
          group: params.group,
          model_name: params.modelName ?? params.models?.[0],
        },
      });
      return mapNewApiLogs(payload);
    }
    return get<{ items: UsageLogItem[] | null; total?: number }>(
      `/client/usage?type=2&${usageQueryString(params)}`,
      token
    );
  },
  async usageSummary(token: string, params: UsageQuery = {}): Promise<UsageSummary> {
    if (isNewApiAuth(loadAuth())) {
      const logs = await this.usage(token, { ...params, pageSize: params.pageSize ?? 100 });
      return summarizeNewApiLogs(logs.items);
    }
    return get<UsageSummary>(`/client/usage/summary?${usageQueryString(params)}`, token);
  },
  topupInfo(token: string): Promise<TopUpInfo> {
    return get<TopUpInfo>("/client/topup/info", token);
  },
  topupHistory(token: string, pageSize = 10): Promise<{ items: TopUpRecord[] | null }> {
    return get<{ items: TopUpRecord[] | null }>(
      `/client/topup/self?p=1&page_size=${pageSize}`,
      token
    );
  },
  async requestEpay(token: string, amount: number, paymentMethod: string): Promise<EpayOrder> {
    const res = await fetch(`${BASE_URL}/api/client/pay`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body: JSON.stringify({ amount, payment_method: paymentMethod }),
    });
    const json = await res.json() as {
      message?: string;
      data?: unknown;
      url?: string;
      success?: boolean;
    };
    if (json.message !== "success") {
      throw new Error(typeof json.data === "string" ? json.data : json.message ?? "下单失败");
    }
    return {
      url: json.url ?? "",
      params: (json.data ?? {}) as Record<string, unknown>,
    };
  },
  revokeDevice(token: string, id: number): Promise<void> {
    const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
    return fetch(`${BASE_URL}/api/client/devices/${id}`, {
      method: "DELETE",
      headers,
    }).then(async (r) => {
      const json = await r.json() as { success: boolean; message?: string };
      if (!json.success) throw new Error(json.message ?? "撤销失败");
    });
  },
  revokeOtherDevices(token: string): Promise<void> {
    const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
    return fetch(`${BASE_URL}/api/client/devices`, {
      method: "DELETE",
      headers,
    }).then(async (r) => {
      const json = await r.json() as { success: boolean; message?: string };
      if (!json.success) throw new Error(json.message ?? "操作失败");
    });
  },
};

export interface DeviceItem {
  id: number;
  device_id: string;
  device_name: string;
  platform: string;
  app_version: string;
  created_time: number;
  accessed_time: number;
  is_current: boolean;
}

export interface UsageLogItem {
  id: number;
  created_at: number;
  model_name: string;
  prompt_tokens: number;
  completion_tokens: number;
  quota: number;
  group?: string;
  use_time?: number;
  is_stream?: boolean;
}

export interface UsageDimension {
  name: string;
  quota: number;
  prompt_tokens: number;
  completion_tokens: number;
  requests: number;
}

export interface UsageDayBucket {
  date: string;
  quota: number;
  tokens: number;
  requests: number;
}

export interface UsageSummary {
  quota: number;
  prompt_tokens: number;
  completion_tokens: number;
  requests: number;
  stream_requests: number;
  total_use_time: number;
  by_model: UsageDimension[] | null;
  by_group: UsageDimension[] | null;
  by_day: UsageDayBucket[] | null;
}
