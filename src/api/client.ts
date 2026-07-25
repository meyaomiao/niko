// momotoken 登录器 API 客户端

const BASE_URL = "https://momotoken.win";

export interface SiteConfig {
  system_name: string;
  turnstile_site_key: string;
  turnstile_enabled: boolean;
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

export interface BootstrapData {
  site: { base_url: string; system_name: string; server_version: string };
  user: { id: number; quota: number; group: string };
  models: string[];
  pricing: Record<string, unknown>;
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
    turnstile: string;
  }): Promise<LoginResult> {
    return post<LoginResult>(
      `/client/login?turnstile=${encodeURIComponent(params.turnstile)}`,
      {
        username: params.username,
        password: params.password,
        device_id: params.deviceId,
        device_name: params.deviceName,
        platform: params.platform,
        app_version: "0.1.0",
      }
    );
  },
  login2fa(pendingToken: string, code: string): Promise<LoginResult> {
    return post<LoginResult>("/client/login/2fa", {
      pending_token: pendingToken,
      code,
    });
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
};
