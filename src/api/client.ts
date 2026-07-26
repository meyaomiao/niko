// momotoken 登录器 API 客户端

const BASE_URL = "https://momotoken.win";

/** 网页端注册页，登录器无注册功能，引导用户去官网注册 */
export const REGISTER_URL = `${BASE_URL}/register`;

export interface SiteConfig {
  system_name: string;
  server_version: string;
}

export interface LoginResult {
  require_2fa: boolean;
  pending_token?: string;
  expires_in?: number;
  access_token?: string;
  username?: string;
  email?: string;
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

export interface BootstrapData {
  site: { base_url: string; system_name: string; server_version: string };
  user: { id: number; quota: number; group: string };
  models: string[];
  pricing: Record<string, unknown>;
  min_supported_version?: string;
  latest_version?: string;
  download_url?: string;
  announcement?: { content?: string; publish?: string } | null;
}

async function post<T>(path: string, body: unknown, token?: string): Promise<T> {
  const headers: Record<string, string> = { "Content-Type": "application/json" };
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${BASE_URL}/api${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  const json = await res.json() as { success: boolean; message?: string; data?: T };
  if (!json.success) throw new Error(json.message ?? "请求失败");
  return (json.data ?? json) as T;
}

async function get<T>(path: string, token?: string): Promise<T> {
  const headers: Record<string, string> = {};
  if (token) headers["Authorization"] = `Bearer ${token}`;
  const res = await fetch(`${BASE_URL}/api${path}`, { headers });
  const json = await res.json() as { success: boolean; message?: string; data?: T };
  if (!json.success) throw new Error(json.message ?? "请求失败");
  return (json.data ?? json) as T;
}

export const api = {
  getSite(): Promise<SiteConfig> {
    return get<SiteConfig>("/client/site");
  },
  login(params: {
    username: string;
    password: string;
    deviceId: string;
    deviceName: string;
    platform: string;
  }): Promise<LoginResult> {
    return post<RawLoginResponse>(
      "/client/login",
      {
        username: params.username,
        password: params.password,
        device_id: params.deviceId,
        device_name: params.deviceName,
        platform: params.platform,
        app_version: "0.1.0",
      }
    ).then(toLoginResult);
  },
  login2fa(pendingToken: string, code: string): Promise<LoginResult> {
    return post<RawLoginResponse>("/client/login/2fa", {
      pending_token: pendingToken,
      code,
    }).then(toLoginResult);
  },
  logout(token: string): Promise<void> {
    return post<void>("/client/logout", {}, token);
  },
  bootstrap(token: string): Promise<BootstrapData> {
    return get<BootstrapData>("/client/bootstrap", token);
  },
  provision(token: string, group: string): Promise<{ api_key: string; token_id: number; group: string }> {
    return post("/client/provision", { group }, token);
  },
  listDevices(token: string): Promise<DeviceItem[]> {
    return get<DeviceItem[]>("/client/devices", token);
  },
  usage(token: string, pageSize = 20): Promise<{ items: UsageLogItem[] | null }> {
    return get<{ items: UsageLogItem[] | null }>(
      `/client/usage?p=1&page_size=${pageSize}&type=2`,
      token
    );
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
}
