// E3-3 会话续期与失效处理
import { useEffect, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { loadAuth, refreshAuthMeta, clearAuth, isCurrentAuthSession, type AuthState } from "../store/auth";

const REFRESH_INTERVAL_MS = 5 * 60 * 1000; // 每 5 分钟刷新一次用量

export function useSession() {
  const navigate = useNavigate();
  const lifecycleRef = useRef(0);
  const activeRef = useRef(false);

  const handleSessionExpired = useCallback((expectedAuth?: AuthState | null) => {
    if (expectedAuth !== undefined && !isCurrentAuthSession(expectedAuth)) return;
    api.clearCatalog();
    clearAuth();
    navigate("/login", { replace: true });
  }, [navigate]);

  const refreshOnce = useCallback(async () => {
    if (!activeRef.current) return;
    const lifecycle = lifecycleRef.current;
    const auth = loadAuth();
    const isCurrent = () => activeRef.current
      && lifecycle === lifecycleRef.current
      && isCurrentAuthSession(auth);
    if (!auth?.accessToken) { handleSessionExpired(auth); return; }
    try {
      const data = await api.bootstrap(auth.accessToken);
      if (!isCurrent()) return;
      refreshAuthMeta({ quota: data.user.quota, defaultGroup: data.user.group }, auth);
    } catch (err) {
      if (!isCurrent()) return;
      const payload = err && typeof err === "object"
        ? err as { code?: string; message?: string; status?: number }
        : {};
      const msg = `${payload.code ?? ""} ${payload.message ?? (typeof err === "string" ? err : "")}`.toLowerCase();
      if (
        payload.status === 401
        || payload.code === "auth"
        || payload.code === "E_SESSION_EXPIRED"
        || msg.includes("401")
        || msg.includes("unauthorized")
        || msg.includes("未登录")
        || msg.includes("令牌无效")
        || /登录.*失效/.test(msg)
        || (msg.includes("已过期") && !msg.includes("名称已存在"))
      ) {
        handleSessionExpired(auth);
      }
    }
  }, [handleSessionExpired]);

  useEffect(() => {
    activeRef.current = true;
    lifecycleRef.current += 1;
    // 立刻刷新一次
    void refreshOnce();
    const timer = setInterval(() => { void refreshOnce(); }, REFRESH_INTERVAL_MS);
    return () => {
      activeRef.current = false;
      lifecycleRef.current += 1;
      clearInterval(timer);
    };
  }, [refreshOnce]);

  return { refreshOnce, handleSessionExpired };
}
