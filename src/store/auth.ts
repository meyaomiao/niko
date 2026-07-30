export interface AuthState {
  accessToken: string;
  username: string;
  userId: number;
  quota: number;
  quotaPerUnit?: number;
  balanceUpdatedAt?: number;
  group: string;
  apiKey: string;
  remember: boolean;
}

const KEY = "niko_auth";

function loadStoredAuth(storage: Storage): AuthState | null {
  try {
    const raw = storage.getItem(KEY);
    if (!raw) return null;
    return JSON.parse(raw) as AuthState;
  } catch {
    return null;
  }
}

export function loadAuth(): AuthState | null {
  const session = loadStoredAuth(sessionStorage);
  if (session) return session;

  const persisted = loadStoredAuth(localStorage);
  return persisted?.remember ? persisted : null;
}

export function shouldPersistAuthSession(
  updateRememberedLogin: boolean,
  remember: boolean,
): boolean {
  return updateRememberedLogin && remember;
}

export function saveAuth(state: AuthState) {
  const value = JSON.stringify(state);
  if (state.remember) {
    localStorage.setItem(KEY, value);
    sessionStorage.removeItem(KEY);
  } else {
    sessionStorage.setItem(KEY, value);
    localStorage.removeItem(KEY);
  }
}

export function clearAuth() {
  localStorage.removeItem(KEY);
  sessionStorage.removeItem(KEY);
}

/** 以服务端数据刷新存储的余额/分组，不改变 token 和 apiKey */
export function refreshAuthMeta(
  patch: Partial<Pick<AuthState, "quota" | "quotaPerUnit" | "balanceUpdatedAt" | "group">>,
) {
  const cur = loadAuth();
  if (!cur) return;
  saveAuth({ ...cur, ...patch });
}
