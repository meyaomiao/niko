export interface AuthState {
  accessToken: string;
  username: string;
  userId: number;
  quota: number;
  group: string;
  apiKey: string;
}

const KEY = "momo_launcher_auth";

export function loadAuth(): AuthState | null {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return null;
    return JSON.parse(raw) as AuthState;
  } catch {
    return null;
  }
}

export function saveAuth(state: AuthState) {
  localStorage.setItem(KEY, JSON.stringify(state));
}

export function clearAuth() {
  localStorage.removeItem(KEY);
}

/** 以新 bootstrap 数据刷新存储的用量/分组，不改变 token 和 apiKey */
export function refreshAuthMeta(patch: Partial<Pick<AuthState, "quota" | "group">>) {
  const cur = loadAuth();
  if (!cur) return;
  saveAuth({ ...cur, ...patch });
}
