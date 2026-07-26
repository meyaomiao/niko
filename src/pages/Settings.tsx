import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, clearAuth } from "../store/auth";
import { useNavigate } from "react-router-dom";

export default function Settings() {
  const auth = loadAuth();
  const navigate = useNavigate();

  const [pingUrl, setPingUrl] = useState("https://momotoken.win");
  const [pingResult, setPingResult] = useState<{
    reachable: boolean;
    latency_ms?: number;
    error?: string;
  } | null>(null);
  const [pinging, setPinging] = useState(false);

  // E8-1: 开机自启
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);

  useEffect(() => {
    invoke<boolean>("autostart_is_enabled")
      .then(setAutostart)
      .catch(() => setAutostart(null));
  }, []);

  const toggleAutostart = async () => {
    if (autostart === null) return;
    setAutostartBusy(true);
    try {
      if (autostart) {
        await invoke("autostart_disable");
        setAutostart(false);
      } else {
        await invoke("autostart_enable");
        setAutostart(true);
      }
    } catch (e) {
      console.error(e);
    } finally {
      setAutostartBusy(false);
    }
  };

  // E9-4: 检查更新
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  const checkUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateStatus(null);
    try {
      // tauri-plugin-updater 暴露为 JS API，通过 shell 调用 check
      const { check } = await import("@tauri-apps/plugin-updater");
      const update = await check();
      if (update?.available) {
        setUpdateStatus(`发现新版本 ${update.version}，正在下载安装…`);
        await update.downloadAndInstall();
      } else {
        setUpdateStatus("已是最新版本");
      }
    } catch (e) {
      setUpdateStatus(`检查失败：${String(e)}`);
    } finally {
      setCheckingUpdate(false);
    }
  };

  const doPing = async () => {
    setPinging(true);
    setPingResult(null);
    try {
      const r = await invoke<{ reachable: boolean; latency_ms?: number; error?: string }>(
        "ping",
        { url: pingUrl }
      );
      setPingResult(r);
    } catch (e) {
      setPingResult({ reachable: false, error: String(e) });
    } finally {
      setPinging(false);
    }
  };

  const logout = () => {
    clearAuth();
    navigate("/login", { replace: true });
  };

  return (
    <div className="flex h-screen flex-col bg-gray-950">
      <header className="flex items-center border-b border-gray-800 px-6 py-4">
        <h1 className="text-sm font-semibold text-white">设置</h1>
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-lg space-y-6">

          {/* 当前账户 */}
          <section className="rounded-2xl bg-gray-900 p-5">
            <h2 className="mb-3 text-xs font-medium uppercase tracking-wide text-gray-400">当前账户</h2>
            <div className="space-y-2 text-sm text-gray-300">
              <p>用户名：<span className="text-white">{auth?.username ?? "—"}</span></p>
              <p>分组：<span className="text-white">{auth?.group ?? "—"}</span></p>
            </div>
            <button
              onClick={logout}
              className="mt-4 rounded-lg bg-red-900/40 px-4 py-2 text-xs text-red-400 transition hover:bg-red-900/60"
            >
              退出登录
            </button>
          </section>

          {/* E8-1: 开机自启 */}
          <section className="rounded-2xl bg-gray-900 p-5">
            <h2 className="mb-3 text-xs font-medium uppercase tracking-wide text-gray-400">启动设置</h2>
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-gray-200">开机自启</p>
                <p className="mt-0.5 text-xs text-gray-500">登录时自动启动 momo·摸摸</p>
              </div>
              <button
                onClick={toggleAutostart}
                disabled={autostartBusy || autostart === null}
                className={`relative h-6 w-11 rounded-full transition-colors disabled:opacity-40 ${
                  autostart ? "bg-indigo-600" : "bg-gray-700"
                }`}
                role="switch"
                aria-checked={autostart ?? false}
              >
                <span
                  className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white shadow transition-transform ${
                    autostart ? "translate-x-5" : "translate-x-0"
                  }`}
                />
              </button>
            </div>
          </section>

          {/* 连通性检测 */}
          <section className="rounded-2xl bg-gray-900 p-5">
            <h2 className="mb-3 text-xs font-medium uppercase tracking-wide text-gray-400">连通性自检</h2>
            <div className="flex gap-2">
              <input
                value={pingUrl}
                onChange={(e) => setPingUrl(e.target.value)}
                className="flex-1 rounded-lg bg-gray-800 px-3 py-2 text-xs text-white outline-none focus:ring-1 focus:ring-indigo-500"
                placeholder="https://..."
              />
              <button
                onClick={doPing}
                disabled={pinging}
                className="rounded-lg bg-indigo-600 px-4 py-2 text-xs text-white transition hover:bg-indigo-500 disabled:opacity-40"
              >
                {pinging ? "检测中…" : "检测"}
              </button>
            </div>
            {pingResult && (
              <div
                className={`mt-3 rounded-lg px-3 py-2 text-xs ${
                  pingResult.reachable
                    ? "bg-green-900/30 text-green-400"
                    : "bg-red-900/30 text-red-400"
                }`}
              >
                {pingResult.reachable
                  ? `✓ 可达，延迟 ${pingResult.latency_ms ?? "?"}ms`
                  : `✗ 不可达：${pingResult.error ?? "未知错误"}`}
              </div>
            )}
          </section>

          {/* E9-4: 版本与更新 */}
          <section className="rounded-2xl bg-gray-900 p-5">
            <h2 className="mb-3 text-xs font-medium uppercase tracking-wide text-gray-400">关于 / 更新</h2>
            <p className="text-xs text-gray-400">momo·摸摸登录器 v0.1.0</p>
            <p className="mt-0.5 text-xs text-gray-600">momotoken.win</p>
            <button
              onClick={checkUpdate}
              disabled={checkingUpdate}
              className="mt-4 rounded-lg bg-gray-700 px-4 py-2 text-xs text-gray-200 transition hover:bg-gray-600 disabled:opacity-40"
            >
              {checkingUpdate ? "检查中…" : "检查更新"}
            </button>
            {updateStatus && (
              <p className="mt-2 text-xs text-gray-400">{updateStatus}</p>
            )}
          </section>

        </div>
      </main>
    </div>
  );
}
