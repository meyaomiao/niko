import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, clearAuth } from "../store/auth";
import { useNavigate } from "react-router-dom";

interface SnapshotEntry {
  target_id: string;
  filename: string;
  timestamp: number;
  original_name: string;
}

interface DiagPingResult {
  reachable: boolean;
  latency_ms?: number;
  error_kind?: string;
  error_detail?: string;
  suggestion?: string;
}

const ERROR_KIND_LABELS: Record<string, string> = {
  network: "网络不通",
  auth: "API Key 无效",
  server: "服务端错误",
  unknown: "未知错误",
};

const TARGET_LABELS: Record<string, string> = {
  "codex": "Codex",
  "claude-desktop": "Claude Desktop",
  "claude-code": "Claude Code",
};

const ALL_TARGET_IDS = ["codex", "claude-desktop", "claude-code"];

function formatTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit", day: "2-digit",
    hour: "2-digit", minute: "2-digit",
  });
}

export default function Settings() {
  const auth = loadAuth();
  const navigate = useNavigate();

  // 连通性检测
  const [pingUrl, setPingUrl] = useState("https://momotoken.win");
  const [pingResult, setPingResult] = useState<DiagPingResult | null>(null);
  const [pinging, setPinging] = useState(false);

  // 日志导出 (E7-3)
  const [exportMsg, setExportMsg] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);

  // 开机自启
  const [autostart, setAutostart] = useState<boolean | null>(null);
  const [autostartBusy, setAutostartBusy] = useState(false);

  // 快照恢复 (E5-5)
  const [snapshots, setSnapshots] = useState<Record<string, SnapshotEntry[]>>({});
  const [snapshotLoading, setSnapshotLoading] = useState(false);
  const [restoring, setRestoring] = useState<string | null>(null);
  const [restoreMsg, setRestoreMsg] = useState<{ ok: boolean; text: string } | null>(null);

  // 检查更新
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  useEffect(() => {
    invoke<boolean>("autostart_is_enabled")
      .then(setAutostart)
      .catch(() => setAutostart(null));
    loadSnapshots();
  }, []);

  const loadSnapshots = async () => {
    setSnapshotLoading(true);
    const result: Record<string, SnapshotEntry[]> = {};
    await Promise.all(
      ALL_TARGET_IDS.map(async (id) => {
        try {
          const list = await invoke<SnapshotEntry[]>("list_snapshots", { targetId: id });
          if (list.length > 0) result[id] = list;
        } catch { /* ignore */ }
      })
    );
    setSnapshots(result);
    setSnapshotLoading(false);
  };

  const restoreSnapshot = async (targetId: string, filename: string) => {
    setRestoring(`${targetId}:${filename}`);
    setRestoreMsg(null);
    try {
      await invoke("restore_snapshot", { targetId, filename });
      setRestoreMsg({ ok: true, text: `✓ 已恢复 ${TARGET_LABELS[targetId] ?? targetId} 的快照` });
    } catch (e) {
      setRestoreMsg({ ok: false, text: `✗ 恢复失败：${String(e)}` });
    } finally {
      setRestoring(null);
    }
  };

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

  const checkUpdate = async () => {
    setCheckingUpdate(true);
    setUpdateStatus(null);
    try {
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
      const r = await invoke<DiagPingResult>("ping_diag", { url: pingUrl });
      setPingResult(r);
    } catch (e) {
      setPingResult({
        reachable: false,
        error_kind: "unknown",
        error_detail: String(e),
        suggestion: "请导出日志后联系支持",
      });
    } finally {
      setPinging(false);
    }
  };

  const exportLog = async () => {
    setExporting(true);
    setExportMsg(null);
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const stamp = new Date().toISOString().slice(0, 19).replace(/[:T]/g, "-");
      const dest = await save({
        defaultPath: `momo-launcher-${stamp}.log`,
        filters: [{ name: "日志", extensions: ["log", "txt"] }],
      });
      if (!dest) {
        setExporting(false);
        return;
      }
      const written = await invoke<string>("export_log", { destPath: dest });
      setExportMsg(`✓ 日志已导出到 ${written}`);
    } catch (e) {
      setExportMsg(`✗ 导出失败：${String(e)}`);
    } finally {
      setExporting(false);
    }
  };

  const logout = () => {
    clearAuth();
    navigate("/login", { replace: true });
  };

  const hasAnySnapshot = Object.keys(snapshots).length > 0;

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

          {/* 开机自启 */}
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

          {/* E5-5: 快照恢复 */}
          <section className="rounded-2xl bg-gray-900 p-5">
            <div className="mb-3 flex items-center justify-between">
              <h2 className="text-xs font-medium uppercase tracking-wide text-gray-400">配置快照恢复</h2>
              <button
                onClick={loadSnapshots}
                disabled={snapshotLoading}
                className="text-xs text-indigo-400 hover:text-indigo-300 disabled:opacity-40"
              >
                {snapshotLoading ? "刷新中…" : "刷新"}
              </button>
            </div>

            {restoreMsg && (
              <div className={`mb-3 rounded-lg px-3 py-2 text-xs ${
                restoreMsg.ok ? "bg-green-900/30 text-green-400" : "bg-red-900/30 text-red-400"
              }`}>
                {restoreMsg.text}
              </div>
            )}

            {!hasAnySnapshot && !snapshotLoading && (
              <p className="text-xs text-gray-500">
                暂无备份。首次配置接入目标后会自动创建快照。
              </p>
            )}

            {ALL_TARGET_IDS.filter((id) => snapshots[id]).map((targetId) => {
              const list = snapshots[targetId];
              // 只展示最新 3 条
              const latest = list.slice(0, 3);
              return (
                <div key={targetId} className="mb-4 last:mb-0">
                  <p className="mb-2 text-xs font-medium text-gray-300">
                    {TARGET_LABELS[targetId] ?? targetId}
                  </p>
                  <div className="space-y-1.5">
                    {latest.map((snap) => {
                      const key = `${targetId}:${snap.filename}`;
                      const isRestoring = restoring === key;
                      return (
                        <div
                          key={snap.filename}
                          className="flex items-center justify-between rounded-lg bg-gray-800 px-3 py-2"
                        >
                          <div>
                            <p className="text-xs text-gray-200">{snap.original_name}</p>
                            <p className="text-xs text-gray-500">{formatTime(snap.timestamp)}</p>
                          </div>
                          <button
                            onClick={() => restoreSnapshot(targetId, snap.filename)}
                            disabled={isRestoring || restoring !== null}
                            className="rounded-md bg-gray-700 px-3 py-1 text-xs text-gray-200 transition hover:bg-gray-600 disabled:opacity-40"
                          >
                            {isRestoring ? "恢复中…" : "恢复"}
                          </button>
                        </div>
                      );
                    })}
                  </div>
                </div>
              );
            })}
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
              pingResult.reachable ? (
                <div className="mt-3 rounded-lg bg-green-900/30 px-3 py-2 text-xs text-green-400">
                  ✓ 可达，延迟 {pingResult.latency_ms ?? "?"}ms
                </div>
              ) : (
                <div className="mt-3 space-y-1.5 rounded-lg bg-red-900/30 px-3 py-2 text-xs text-red-300">
                  <p className="font-medium text-red-400">
                    ✗ {ERROR_KIND_LABELS[pingResult.error_kind ?? "unknown"]}
                  </p>
                  {pingResult.error_detail && (
                    <p className="text-red-300/80">详情：{pingResult.error_detail}</p>
                  )}
                  {pingResult.suggestion && (
                    <p className="text-gray-300">建议：{pingResult.suggestion}</p>
                  )}
                </div>
              )
            )}

            <div className="mt-4 border-t border-gray-800 pt-4">
              <p className="mb-2 text-xs text-gray-500">
                导出的日志已对 API Key 做脱敏处理，不含完整密钥。
              </p>
              <button
                onClick={exportLog}
                disabled={exporting}
                className="rounded-lg bg-gray-700 px-4 py-2 text-xs text-gray-200 transition hover:bg-gray-600 disabled:opacity-40"
              >
                {exporting ? "导出中…" : "导出日志"}
              </button>
              {exportMsg && <p className="mt-2 break-all text-xs text-gray-400">{exportMsg}</p>}
            </div>
          </section>

          {/* 版本与更新 */}
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
