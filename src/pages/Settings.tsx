import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, clearAuth } from "../store/auth";
import { useNavigate } from "react-router-dom";
import { BRAND } from "../lib/brand";
import Logo from "../components/Logo";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  BookOpenIcon,
  RefreshCwIcon,
} from "../components/Icons";
import type {
  CodexSessionMutationOutcome,
  CodexSessionPage,
} from "../lib/codexSessions";
import {
  acceptsResponse,
  beginRequest,
  initialRequestGuard,
  mountRequests,
  normalizeCodexSessionPage,
  safeFailure,
  unmountRequests,
} from "../lib/codexSessions";

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
  network: "网络连接失败",
  auth: "连接密钥无效或已过期",
  server: "模型服务暂时不可用",
  unknown: "检查没有完成",
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
  const [snapshotError, setSnapshotError] = useState("");
  const [restoring, setRestoring] = useState<string | null>(null);
  const [restoreMsg, setRestoreMsg] = useState<{ ok: boolean; text: string } | null>(null);

  // 检查更新
  const [updateStatus, setUpdateStatus] = useState<string | null>(null);
  const [checkingUpdate, setCheckingUpdate] = useState(false);

  // Codex 会话状态与整理
  const [codexInventory, setCodexInventory] = useState<CodexSessionPage | null>(null);
  const [codexScanning, setCodexScanning] = useState(false);
  const [codexAction, setCodexAction] = useState<"custom" | "openai" | null>(null);
  const [codexMessage, setCodexMessage] = useState<{ ok: boolean; text: string } | null>(null);
  const [codexRetryTarget, setCodexRetryTarget] = useState<"custom" | "openai" | null>(null);
  const codexGuard = useRef(initialRequestGuard());

  const scanCodexSessions = async () => {
    const request = beginRequest(codexGuard.current, "scan");
    codexGuard.current = request.state;
    setCodexScanning(true);
    try {
      const rawResult = await invoke<unknown>("scan_codex_session_inventory", {
        query: "",
        page: 1,
        page_size: 1,
      });
      if (!acceptsResponse(codexGuard.current, "scan", request.generation)) return null;
      const result = normalizeCodexSessionPage(rawResult);
      if (!result) throw new Error("invalid session response");
      setCodexInventory(result);
      return result;
    } catch (error) {
      if (!acceptsResponse(codexGuard.current, "scan", request.generation)) return null;
      setCodexMessage({ ok: false, text: safeFailure(error).message });
      return null;
    } finally {
      if (acceptsResponse(codexGuard.current, "scan", request.generation)) {
        setCodexScanning(false);
      }
    }
  };

  const normalizeCodexSessions = async (targetProvider: "custom" | "openai") => {
    const request = beginRequest(codexGuard.current, "action");
    codexGuard.current = request.state;
    setCodexAction(targetProvider);
    setCodexMessage(null);
    setCodexRetryTarget(null);
    try {
      const result = await invoke<CodexSessionMutationOutcome>(
        "normalize_codex_session_storage",
        { targetProvider },
      );
      if (!acceptsResponse(codexGuard.current, "action", request.generation)) return;
      setCodexMessage({ ok: true, text: result.message });
      setCodexRetryTarget(null);
      await scanCodexSessions();
    } catch (error) {
      if (!acceptsResponse(codexGuard.current, "action", request.generation)) return;
      const failure = safeFailure(error);
      setCodexMessage({
        ok: false,
        text: failure.message,
      });
      setCodexRetryTarget(failure.retryable ? targetProvider : null);
    } finally {
      if (acceptsResponse(codexGuard.current, "action", request.generation)) {
        setCodexAction(null);
      }
    }
  };

  useEffect(() => {
    codexGuard.current = mountRequests(codexGuard.current);
    invoke<boolean>("autostart_is_enabled")
      .then(setAutostart)
      .catch(() => setAutostart(null));
    loadSnapshots();
    void scanCodexSessions();
    return () => {
      codexGuard.current = unmountRequests(codexGuard.current);
    };
  }, []);

  const loadSnapshots = async () => {
    setSnapshotLoading(true);
    setSnapshotError("");
    const result: Record<string, SnapshotEntry[]> = {};
    let failed = false;
    await Promise.all(
      ALL_TARGET_IDS.map(async (id) => {
        try {
          const list = await invoke<SnapshotEntry[]>("list_snapshots", { targetId: id });
          if (list.length > 0) result[id] = list;
        } catch {
          failed = true;
        }
      })
    );
    setSnapshots(result);
    if (failed) setSnapshotError("备份暂时无法读取，请重新读取。");
    setSnapshotLoading(false);
  };

  const restoreSnapshot = async (targetId: string, filename: string) => {
    setRestoring(`${targetId}:${filename}`);
    setRestoreMsg(null);
    try {
      await invoke("restore_snapshot", { targetId, filename });
      setRestoreMsg({ ok: true, text: `✓ 已恢复 ${TARGET_LABELS[targetId] ?? targetId} 的应用设置` });
    } catch (e) {
      setRestoreMsg({ ok: false, text: `✗ 恢复失败：${safeFailure(e).message}` });
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
      setUpdateStatus(`检查失败：${safeFailure(e).message}`);
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
        error_detail: safeFailure(e).message,
        suggestion: "请稍后重试。",
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
      await invoke<string>("export_log", { destPath: dest });
      setExportMsg("✓ 日志已导出，可交给支持人员查看。");
    } catch (e) {
      setExportMsg(`✗ 导出失败：${safeFailure(e).message}`);
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
              <p>推荐的模型服务：<span className="text-gray-900 dark:text-white">{auth?.defaultGroup ?? "—"}</span></p>
            </div>
            <button
              onClick={logout}
              className="nk-btn-danger mt-4"
            >
              退出登录
            </button>
          </section>

          {/* Codex 本地会话 */}
          <section className={CARD}>
            <div className="mb-3 flex items-start justify-between gap-3">
              <div>
                <h2 className={OVERLINE}>ChatGPT 会话</h2>
                <p className="mt-1 text-sm text-gray-800 dark:text-gray-200">
                  ChatGPT 支持会话检查、迁移和恢复；Claude 桌面端不支持会话管理。
                </p>
              </div>
              <span className="nk-pill shrink-0">
                {codexInventory
                  ? codexInventory.status === "healthy"
                      ? "会话可续接"
                      : codexInventory.status === "needs_check"
                        ? "有会话待处理"
                        : "会话检查暂时无法确认"
                  : "检查中"}
              </span>
            </div>

            {codexMessage && (
              <div className={`mb-3 ${codexMessage.ok ? "nk-alert-success" : "nk-alert-danger"}`} role="status">
                {codexMessage.text}
              </div>
            )}

            <div className="flex flex-wrap items-center gap-2">
              <button
                onClick={() => navigate("/sessions")}
                className={SECONDARY_BTN}
              >
                <BookOpenIcon />
                查看 ChatGPT 会话
                <ArrowRightIcon />
              </button>
              <button
                onClick={() => void scanCodexSessions()}
                disabled={codexScanning || codexAction !== null}
                className={SECONDARY_BTN}
              >
                <RefreshCwIcon />
                {codexScanning ? "检查中…" : "重新检查"}
              </button>
              <button
                onClick={() => void normalizeCodexSessions("openai")}
                disabled={codexScanning || codexAction !== null}
                className={SECONDARY_BTN}
              >
                {codexAction === "openai" ? "恢复中…" : "恢复到官方"}
              </button>
              {codexRetryTarget && (
                <button
                  onClick={() => void normalizeCodexSessions(codexRetryTarget)}
                  disabled={codexScanning || codexAction !== null}
                  className="nk-btn-ghost"
                >
                  重试
                </button>
              )}
            </div>
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
              <h2 className={OVERLINE}>可恢复的备份</h2>
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

            {snapshotError && (
              <p className="nk-alert-danger mb-3" role="alert">{snapshotError}</p>
            )}

            {!hasAnySnapshot && !snapshotLoading && !snapshotError && (
              <p className="nk-empty">
                还没有备份。首次接入应用时会自动保存一份，之后可从这里恢复。
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
                            <p className="text-xs text-gray-800 dark:text-gray-200">可恢复的应用设置</p>
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

          {/* 检查是否能正常使用 */}
          <section className={CARD}>
            <h2 className={`mb-3 ${OVERLINE}`}>检查是否能正常使用</h2>
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
                {pinging ? "检查中…" : "开始检查"}
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
                    <p className="text-red-600/80 dark:text-red-300/80">当前状态：{pingResult.error_detail}</p>
                  )}
                  {pingResult.suggestion && (
                    <p className="text-gray-700 dark:text-gray-300">下一步：{pingResult.suggestion}</p>
                  )}
                </div>
              )
            )}

            <div className="mt-4 border-t pt-4 [border-color:var(--nk-line)]">
              <p className="mb-2 text-xs text-gray-500 dark:text-gray-400">
                导出的日志已对敏感凭证做脱敏处理，不含完整内容。
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
