// 轻量级认证状态管理（无第三方状态库，用 Context + localStorage）

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
