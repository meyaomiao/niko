export interface AuthState {
  accessToken: string;
  username: string;
  userId: number;
  quota: number;
  quotaPerUnit?: number;
  balanceUpdatedAt?: number;
  /** 账户默认推荐分组；不代表任一目标应用当前生效的分组。 */
  defaultGroup: string;
  apiKey: string;
  /** 分组申请得到的 apiKey 所属分组；与账户默认推荐分组分开保存。 */
  apiKeyGroup?: string;
  remember: boolean;
}

const KEY = "niko_auth";

function loadStoredAuth(storage: Storage): AuthState | null {
  try {
    const raw = storage.getItem(KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<AuthState> & { group?: unknown };
    const defaultGroup =
      typeof parsed.defaultGroup === "string"
        ? parsed.defaultGroup
        : typeof parsed.group === "string"
          ? parsed.group
          : "";
    const { group: _legacyGroup, ...rest } = parsed;
    return { ...rest, defaultGroup } as AuthState;
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

/** 以服务端数据刷新存储的余额/默认推荐分组，不改变目标应用状态 */
export function refreshAuthMeta(
  patch: Partial<Pick<AuthState, "quota" | "quotaPerUnit" | "balanceUpdatedAt" | "defaultGroup">>,
) {
  const cur = loadAuth();
  if (!cur) return;
  saveAuth({ ...cur, ...patch });
}
