import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, clearAuth } from "../store/auth";
import { useNavigate } from "react-router-dom";
import { BRAND } from "../lib/brand";
import Logo from "../components/Logo";
import { ArrowLeftIcon } from "../components/Icons";

const CARD = "nk-card";
const OVERLINE = "nk-overline";
const SECONDARY_BTN = "nk-btn-secondary";

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
  "codex": "ChatGPT 桌面端",
  "claude-desktop": "Claude 桌面端",
};

const ALL_TARGET_IDS = ["codex", "claude-desktop"];

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
        defaultPath: `niko-${stamp}.log`,
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
    <div className="nk-shell">
      <header className="nk-header">
        <button
          onClick={() => navigate("/home")}
          className="nk-btn-ghost px-2.5"
          aria-label="返回首页"
        >
          <ArrowLeftIcon />
        </button>
        <h1 className="nk-title">设置</h1>
      </header>

      <main className="nk-page">
        {/* 宽窗口下分两列，避免长单列造成左右大片留白 */}
        <div className="mx-auto max-w-5xl columns-1 gap-4 md:columns-2 [&>section]:mb-4 [&>section]:break-inside-avoid">

          {/* 当前账户 */}
          <section className={CARD}>
            <h2 className={`mb-3 ${OVERLINE}`}>当前账户</h2>
            <div className="space-y-2 text-sm text-gray-700 dark:text-gray-300">
              <p>用户名：<span className="text-gray-900 dark:text-white">{auth?.username ?? "—"}</span></p>
              <p>分组：<span className="text-gray-900 dark:text-white">{auth?.group ?? "—"}</span></p>
            </div>
            <button
              onClick={logout}
              className="nk-btn-danger mt-4"
            >
              退出登录
            </button>
          </section>

          {/* 开机自启 */}
          <section className={CARD}>
            <h2 className={`mb-3 ${OVERLINE}`}>启动设置</h2>
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm text-gray-800 dark:text-gray-200">开机自启</p>
                <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">登录时自动启动此应用</p>
              </div>
              <button
                onClick={toggleAutostart}
                disabled={autostartBusy || autostart === null}
                className={`relative h-6 w-11 rounded-full transition-colors disabled:opacity-40 ${
                  autostart ? "bg-indigo-600" : "bg-black/[0.06] dark:bg-white/10"
                }`}
                role="switch"
                aria-checked={autostart ?? false}
                aria-label="开机自启"
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
          <section className={CARD}>
            <div className="mb-3 flex items-center justify-between">
              <h2 className={OVERLINE}>配置快照恢复</h2>
              <button
                onClick={loadSnapshots}
                disabled={snapshotLoading}
                className="nk-btn-ghost"
              >
                {snapshotLoading ? "刷新中…" : "刷新"}
              </button>
            </div>

            {restoreMsg && (
              <div className={`mb-3 ${
                restoreMsg.ok ? "nk-alert-success" : "nk-alert-danger"
              }`}>
                {restoreMsg.text}
              </div>
            )}

            {!hasAnySnapshot && !snapshotLoading && (
              <p className="nk-empty">
                暂无备份。首次配置接入目标后会自动创建快照。
              </p>
            )}

            {ALL_TARGET_IDS.filter((id) => snapshots[id]).map((targetId) => {
              const list = snapshots[targetId];
              // 只展示最新 3 条
              const latest = list.slice(0, 3);
              return (
                <div key={targetId} className="mb-4 last:mb-0">
                  <p className="mb-2 text-xs font-medium text-gray-700 dark:text-gray-300">
                    {TARGET_LABELS[targetId] ?? targetId}
                  </p>
                  <div className="space-y-1.5">
                    {latest.map((snap) => {
                      const key = `${targetId}:${snap.filename}`;
                      const isRestoring = restoring === key;
                      return (
                        <div
                          key={snap.filename}
                          className="nk-row flex items-center justify-between gap-3"
                        >
                          <div>
                            <p className="text-xs text-gray-800 dark:text-gray-200">{snap.original_name}</p>
                            <p className="text-xs text-gray-500 dark:text-gray-400">{formatTime(snap.timestamp)}</p>
                          </div>
                          <button
                            onClick={() => restoreSnapshot(targetId, snap.filename)}
                            disabled={isRestoring || restoring !== null}
                            className={SECONDARY_BTN}
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
          <section className={CARD}>
            <h2 className={`mb-3 ${OVERLINE}`}>连通性自检</h2>
            <div className="flex gap-2">
              <input
                value={pingUrl}
                onChange={(e) => setPingUrl(e.target.value)}
                className="nk-input min-w-0 flex-1 text-xs"
                placeholder="https://..."
              />
              <button
                onClick={doPing}
                disabled={pinging}
                className="nk-btn-primary"
              >
                {pinging ? "检测中…" : "检测"}
              </button>
            </div>
            {pingResult && (
              pingResult.reachable ? (
                <div className="nk-alert-success mt-3">
                  ✓ 可达，延迟 {pingResult.latency_ms ?? "?"}ms
                </div>
              ) : (
                <div className="nk-alert-danger mt-3 space-y-1.5">
                  <p className="font-medium text-red-600 dark:text-red-400">
                    ✗ {ERROR_KIND_LABELS[pingResult.error_kind ?? "unknown"]}
                  </p>
                  {pingResult.error_detail && (
                    <p className="text-red-600/80 dark:text-red-300/80">详情：{pingResult.error_detail}</p>
                  )}
                  {pingResult.suggestion && (
                    <p className="text-gray-700 dark:text-gray-300">建议：{pingResult.suggestion}</p>
                  )}
                </div>
              )
            )}

            <div className="mt-4 border-t pt-4 [border-color:var(--nk-line)]">
              <p className="mb-2 text-xs text-gray-500 dark:text-gray-400">
                导出的日志已对 API Key 做脱敏处理，不含完整密钥。
              </p>
              <button
                onClick={exportLog}
                disabled={exporting}
                className={SECONDARY_BTN}
              >
                {exporting ? "导出中…" : "导出日志"}
              </button>
              {exportMsg && <p className="nk-muted mt-2 break-all">{exportMsg}</p>}
            </div>
          </section>

          {/* 版本与更新 */}
          <section className={CARD}>
            <h2 className={`mb-3 ${OVERLINE}`}>关于 / 更新</h2>
            <div className="flex items-center gap-2">
              <Logo size={24} />
              <div>
                <p className="text-xs text-gray-700 dark:text-gray-200">
                  登录器 v{BRAND.version}
                </p>
                <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-500">{BRAND.tagline}</p>
              </div>
            </div>
            <button
              onClick={checkUpdate}
              disabled={checkingUpdate}
              className={`${SECONDARY_BTN} mt-4`}
            >
              {checkingUpdate ? "检查中…" : "检查更新"}
            </button>
            {updateStatus && (
              <p className="nk-muted mt-2">{updateStatus}</p>
            )}
          </section>

        </div>
      </main>
    </div>
  );
}
