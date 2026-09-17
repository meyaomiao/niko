// E3-3 会话续期与失效处理
import { useEffect, useCallback, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { api } from "../api/client";
import { loadAuth, refreshAuthMeta, clearAuth } from "../store/auth";

const REFRESH_INTERVAL_MS = 5 * 60 * 1000; // 每 5 分钟刷新一次用量

export function useSession() {
  const navigate = useNavigate();
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);

  const handleSessionExpired = useCallback(() => {
    clearAuth();
    navigate("/login", { replace: true });
  }, [navigate]);

  const refreshOnce = useCallback(async () => {
    const auth = loadAuth();
    if (!auth?.accessToken) { handleSessionExpired(); return; }
    try {
      const data = await api.bootstrap(auth.accessToken);
      refreshAuthMeta({ quota: data.user.quota, defaultGroup: data.user.group });
    } catch (err) {
      const payload = err && typeof err === "object" ? err as { code?: string; message?: string } : {};
      const msg = `${payload.code ?? ""} ${payload.message ?? (err instanceof Error ? err.message : "")}`.toLowerCase();
      if (
        payload.code === "auth"
        || msg.includes("401")
        || msg.includes("unauthorized")
        || msg.includes("未登录")
        || msg.includes("令牌无效")
        || msg.includes("已过期")
      ) {
        handleSessionExpired();
      }
    }
  }, [handleSessionExpired]);

  useEffect(() => {
    // 立刻刷新一次
    void refreshOnce();
    timerRef.current = setInterval(() => { void refreshOnce(); }, REFRESH_INTERVAL_MS);
    return () => { if (timerRef.current) clearInterval(timerRef.current); };
  }, [refreshOnce]);

  return { refreshOnce, handleSessionExpired };
}
