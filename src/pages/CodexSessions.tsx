import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import {
  ArrowLeftIcon,
  ArrowRightIcon,
  BookOpenIcon,
  RefreshCwIcon,
} from "../components/Icons";
import {
  acceptsResponse,
  beginRequest,
  boundSessionQuery,
  CODEX_SESSION_SYNC_PROGRESS_EVENT,
  codexNormalizationLabel,
  codexProviderLabel,
  initialRequestGuard,
  mountRequests,
  normalizeCodexSessionPage,
  normalizeCodexSessionSyncProgress,
  safeFailure,
  SESSION_PAGE_SIZE,
  SESSION_QUERY_LIMIT,
  type CodexSessionSyncProgress,
  type CodexSessionMutationOutcome,
  type CodexSessionPage,
  type CodexSessionThread,
  unmountRequests,
} from "../lib/codexSessions";

type SyncTarget = "custom" | "openai";

const SYNC_TARGET_LABELS: Record<SyncTarget, string> = {
  custom: "Niko 模型服务",
  openai: "ChatGPT 官方模型服务",
};

const SYNC_PHASE_LABELS: Record<CodexSessionSyncProgress["phase"], string> = {
  preparing: "准备同步",
  backing_up: "备份会话数据",
  staging: "写入同步副本",
  committing: "提交同步结果",
  validating: "校验会话记录",
  completed: "同步完成",
};

function formatSessionTime(value?: string | null): string {
  if (!value) return "时间未知";
  const date = new Date(Number(value));
  return Number.isNaN(date.getTime()) ? "时间未知" : date.toLocaleString("zh-CN");
}

export default function CodexSessions() {
  const navigate = useNavigate();
  const signedIn = Boolean(loadAuth()?.accessToken);
  const guard = useRef(initialRequestGuard());
  const pageRef = useRef<CodexSessionPage | null>(null);
  const queryRef = useRef("");
  const targetRef = useRef<SyncTarget>("custom");
  const [page, setPageState] = useState<CodexSessionPage | null>(null);
  const [query, setQueryState] = useState("");
  const [targetProvider, setTargetProvider] = useState<SyncTarget>("custom");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [migrating, setMigrating] = useState(false);
  const [closing, setClosing] = useState(false);
  const [migrationMessage, setMigrationMessage] = useState<string | null>(null);
  const [syncProgress, setSyncProgress] = useState<CodexSessionSyncProgress | null>(null);
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());

  const setPage = (value: CodexSessionPage | null) => {
    pageRef.current = value;
    setPageState(value);
  };

  const setQuery = (value: string) => {
    const request = beginRequest(guard.current, "scan");
    guard.current = request.state;
    queryRef.current = value;
    setQueryState(value);
    setPage(null);
    setSelectedIds(new Set());
    setLoading(true);
    setMigrationMessage(null);
  };

  const setSyncTarget = (value: SyncTarget) => {
    const request = beginRequest(guard.current, "scan");
    guard.current = request.state;
    targetRef.current = value;
    setTargetProvider(value);
    setPage(null);
    setSelectedIds(new Set());
    setLoading(true);
    setError(null);
    setMigrationMessage(null);
    setSyncProgress(null);
  };

  const load = async (requestedPage: number) => {
    const request = beginRequest(guard.current, "scan");
    guard.current = request.state;
    setLoading(true);
    setError(null);
    try {
      const rawResult = await invoke<unknown>("scan_codex_session_inventory", {
        query: boundSessionQuery(queryRef.current),
        page: requestedPage,
        page_size: SESSION_PAGE_SIZE,
        targetProvider: targetRef.current,
      });
      if (!acceptsResponse(guard.current, "scan", request.generation)) return;
      const result = normalizeCodexSessionPage(rawResult);
      if (!result) throw new Error("invalid session response");
      setPage(result);
    } catch (rejection) {
      if (acceptsResponse(guard.current, "scan", request.generation)) {
        setError(safeFailure(rejection).message);
      }
    } finally {
      if (acceptsResponse(guard.current, "scan", request.generation)) {
        setLoading(false);
      }
    }
  };

  const retryScan = () => {
    setSelectedIds(new Set());
    setMigrationMessage(null);
    void load(1);
  };

  const refreshSessions = () => {
    if (loading || migrating || opening !== null || closing) return;
    setSelectedIds(new Set());
    setError(null);
    setMigrationMessage(null);
    setSyncProgress(null);
    void load(pageRef.current?.page ?? 1);
  };

  const closeChatGptAndScan = async () => {
    if (closing || loading || migrating || opening !== null) return;
    const request = beginRequest(guard.current, "action");
    guard.current = request.state;
    setClosing(true);
    setError(null);
    setMigrationMessage(null);
    try {
      const result = await invoke<{ status: string; message: string }>("close_target", {
        targetId: "codex",
      });
      if (!acceptsResponse(guard.current, "action", request.generation)) return;
      setMigrationMessage(result.message);
      await load(1);
    } catch (rejection) {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setSyncProgress(null);
        setError(safeFailure(rejection).message);
      }
    } finally {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setClosing(false);
      }
    }
  };

  useEffect(() => {
    const timer = window.setTimeout(() => void load(1), 150);
    return () => window.clearTimeout(timer);
  }, [query, targetProvider]);

  useEffect(() => {
    let active = true;
    let unlisten: (() => void) | null = null;
    void listen<unknown>(CODEX_SESSION_SYNC_PROGRESS_EVENT, (event) => {
      const progress = normalizeCodexSessionSyncProgress(event.payload);
      if (active && progress && progress.target_provider === targetRef.current) {
        setSyncProgress(progress);
      }
    }).then((cleanup) => {
      if (active) unlisten = cleanup;
      else cleanup();
    });
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    guard.current = mountRequests(guard.current);
    return () => {
      guard.current = unmountRequests(guard.current);
    };
  }, []);

  const openThread = async (thread: CodexSessionThread) => {
    if (!thread.can_continue || migrating) return;
    const request = beginRequest(guard.current, "action");
    guard.current = request.state;
    setOpening(thread.thread_id);
    setError(null);
    try {
      await invoke("open_codex_thread", { threadId: thread.thread_id });
    } catch (rejection) {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setSyncProgress(null);
        setError(safeFailure(rejection).message);
      }
    } finally {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setOpening(null);
      }
    }
  };

  const migrateSessions = async (threadIds?: string[]) => {
    const selected = threadIds && threadIds.length > 0 ? Array.from(new Set(threadIds)) : null;
    if (migrating || opening !== null || page?.status === "blocked") return;
    if (selected ? selected.length === 0 : page?.status !== "needs_check") return;
    const targetLabel = SYNC_TARGET_LABELS[targetProvider];
    const prompt = selected
      ? `将把选中的 ${selected.length} 个会话同步到${targetLabel}，并自动备份后原子更新。是否继续？`
      : `将把所有待同步的 ChatGPT 会话统一同步到${targetLabel}，并自动备份后原子更新。是否继续？`;
    if (!window.confirm(prompt)) {
      return;
    }
    const request = beginRequest(guard.current, "action");
    guard.current = request.state;
    setMigrating(true);
    setError(null);
    setMigrationMessage(null);
    setSyncProgress({
      phase: "preparing",
      percent: 5,
      processed: 0,
      total: 0,
      target_provider: targetProvider,
    });
    try {
      const result = await invoke<CodexSessionMutationOutcome>(
        selected ? "normalize_codex_session_storage_selected" : "normalize_codex_session_storage",
        selected
          ? { targetProvider, threadIds: selected }
          : { targetProvider },
      );
      if (!acceptsResponse(guard.current, "action", request.generation)) return;
      setMigrationMessage(result.message);
      setSelectedIds(new Set());
      await load(pageRef.current?.page ?? 1);
    } catch (rejection) {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setError(safeFailure(rejection).message);
      }
    } finally {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setMigrating(false);
      }
    }
  };

  const selectableItems = page?.items.filter((thread) => thread.needs_migration) ?? [];
  const allSelectableSelected = selectableItems.length > 0
    && selectableItems.every((thread) => selectedIds.has(thread.thread_id));
  const globalBlockers = page?.blockers.filter((blocker) => blocker.thread_id === "无法确认") ?? [];

  const togglePageSelection = () => {
    setSelectedIds((current) => {
      const next = new Set(current);
      for (const thread of selectableItems) {
        if (allSelectableSelected) next.delete(thread.thread_id);
        else next.add(thread.thread_id);
      }
      return next;
    });
  };

  return (
    <div className="nk-shell">
      <header className="nk-header justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <button
            onClick={() => navigate(signedIn ? "/home" : "/login")}
            className="nk-btn-ghost px-2.5"
            aria-label="返回"
          >
            <ArrowLeftIcon />
          </button>
          <BookOpenIcon />
          <h1 className="nk-title">ChatGPT 会话</h1>
        </div>
        <button
          onClick={refreshSessions}
          disabled={loading || migrating || opening !== null || closing}
          className="nk-btn-ghost px-2.5"
          aria-label="刷新会话"
          title="刷新会话"
        >
          <RefreshCwIcon className={loading ? "animate-spin motion-reduce:animate-none" : ""} />
        </button>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-3xl space-y-4">
          <section className="nk-card">
            <div className="flex flex-wrap items-start justify-between gap-3">
              <div className="min-w-0">
                <p className="nk-overline">ChatGPT</p>
                <h2 className="mt-1 text-base font-semibold">
                  {page ? codexNormalizationLabel(page.status) : "正在检查会话…"}
                </h2>
              </div>
              <span className="nk-pill shrink-0">{page?.total ?? 0} 个会话</span>
            </div>
            <input
              value={query}
              maxLength={SESSION_QUERY_LIMIT}
              onChange={(event) => setQuery(event.target.value)}
              className="nk-input mt-4 w-full text-xs"
              placeholder="搜索会话"
              aria-label="搜索会话"
            />
            <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-end">
              <label className="min-w-0 flex-1 text-xs text-[var(--nk-muted)]">
                <span className="mb-1 block">同步目标</span>
                <select
                  value={targetProvider}
                  onChange={(event) => setSyncTarget(event.target.value as SyncTarget)}
                  disabled={loading || migrating || opening !== null || closing}
                  className="nk-select w-full"
                  aria-label="同步目标"
                >
                  <option value="custom">Niko 模型服务（custom）</option>
                  <option value="openai">ChatGPT 官方模型服务（openai）</option>
                </select>
              </label>
              <button
                onClick={refreshSessions}
                disabled={loading || migrating || opening !== null || closing}
                className="nk-btn-secondary shrink-0 whitespace-nowrap px-2.5"
              >
                <RefreshCwIcon className={loading ? "animate-spin motion-reduce:animate-none" : ""} />
                {loading ? "刷新中…" : "刷新会话"}
              </button>
            </div>
            {error && <p className="nk-alert-danger mt-3" role="alert">{error}</p>}
            {migrationMessage && <p className="nk-alert-success mt-3" role="status">{migrationMessage}</p>}
            {syncProgress && (
              <div className="nk-inset mt-3 p-3" role="status" aria-live="polite">
                <div className="flex items-center justify-between gap-3 text-xs">
                  <span className="font-medium">{SYNC_PHASE_LABELS[syncProgress.phase]}</span>
                  <span className="text-[var(--nk-muted)]">{syncProgress.percent}%</span>
                </div>
                <div
                  className="mt-2 h-1.5 overflow-hidden rounded-full bg-[var(--nk-line)]"
                  role="progressbar"
                  aria-label="会话同步进度"
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={syncProgress.percent}
                >
                  <div
                    className="h-full rounded-full bg-[var(--nk-accent)] transition-[width] duration-200"
                    style={{ width: `${syncProgress.percent}%` }}
                  />
                </div>
                <p className="mt-1.5 text-[11px] text-[var(--nk-muted)]">
                  目标：{SYNC_TARGET_LABELS[syncProgress.target_provider]}
                  {syncProgress.total > 0 && ` · 已处理 ${syncProgress.processed}/${syncProgress.total} 个文件`}
                </p>
              </div>
            )}
            <div className="nk-inset mt-4 p-3 text-xs text-[var(--nk-muted)]">
              <p className="font-medium text-[var(--nk-ink)]">会话管理范围</p>
              <p className="mt-1">仅支持 ChatGPT 桌面端会话的检查、迁移和恢复；Claude 桌面端不支持这些操作。</p>
            </div>
          </section>

          {globalBlockers.length > 0 && (
            <section className="nk-alert-danger" aria-label="会话检查状态">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-medium">会话检查暂时无法确认</p>
                <span className="text-[11px]">{globalBlockers.length} 项扫描阻塞</span>
              </div>
              <details className="mt-1.5">
                <summary className="cursor-pointer text-[11px]">查看阻塞原因</summary>
                <div className="mt-1.5 space-y-1.5">
                  {globalBlockers.map((blocker, index) => (
                    <div key={`${blocker.reason}-${index}`}>
                      <p>{blocker.reason}</p>
                      <p className="mt-0.5 opacity-80">下一步：{blocker.next_step}</p>
                    </div>
                  ))}
                </div>
              </details>
              <div className="mt-2 flex flex-wrap gap-1.5">
                <button
                  onClick={() => void closeChatGptAndScan()}
                  disabled={loading || migrating || opening !== null || closing}
                  className="nk-btn-primary px-2.5"
                >
                  {closing ? "关闭中…" : "关闭 ChatGPT 并检查"}
                </button>
                <button
                  onClick={retryScan}
                  disabled={loading || migrating || opening !== null || closing}
                  className="nk-btn-secondary px-2.5"
                >
                  {loading ? "检查中…" : "重新检查"}
                </button>
              </div>
            </section>
          )}

          {loading && !page ? (
            <p className="nk-empty" role="status">正在检查会话…</p>
          ) : page?.items.length ? (
            <section className="space-y-2" aria-label="本地会话列表">
              <div className="flex flex-col items-start justify-between gap-2 sm:flex-row sm:items-center">
                <div className="flex flex-wrap items-center gap-1.5 text-xs text-[var(--nk-muted)]">
                  <span>已选 {selectedIds.size} 个</span>
                  <button
                    onClick={togglePageSelection}
                    disabled={selectableItems.length === 0 || migrating || opening !== null || page.status === "blocked"}
                    className="nk-btn-secondary whitespace-nowrap px-2.5"
                  >
                    {allSelectableSelected ? "取消全选" : "全选待同步"}
                  </button>
                  <button
                    onClick={() => setSelectedIds(new Set())}
                    disabled={selectedIds.size === 0 || migrating}
                    className="nk-btn-secondary whitespace-nowrap px-2.5"
                  >
                    清空选择
                  </button>
                </div>
                <div className="flex w-full flex-wrap gap-1.5 sm:w-auto sm:justify-end">
                  {selectedIds.size > 0 && (
                    <button
                      onClick={() => void migrateSessions(Array.from(selectedIds))}
                      disabled={migrating || opening !== null || page.status === "blocked"}
                      className="nk-btn-primary whitespace-nowrap px-2.5"
                    >
                      {migrating ? "同步中…" : `同步选中 (${selectedIds.size})`}
                    </button>
                  )}
                  <button
                    onClick={() => void migrateSessions()}
                    disabled={page.status !== "needs_check" || migrating || opening !== null}
                    className="nk-btn-secondary whitespace-nowrap px-2.5"
                  >
                    {migrating ? "同步中…" : "同步全部待处理会话"}
                  </button>
                </div>
              </div>
              {page.items.map((thread) => {
                const isOpening = opening === thread.thread_id;
                const isSelected = selectedIds.has(thread.thread_id);
                return (
                  <div key={thread.thread_id} className="nk-row flex items-start gap-3">
                    <input
                      type="checkbox"
                      checked={isSelected}
                      disabled={!thread.needs_migration || migrating || opening !== null}
                      onChange={() => {
                        setSelectedIds((current) => {
                          const next = new Set(current);
                          if (next.has(thread.thread_id)) next.delete(thread.thread_id);
                          else next.add(thread.thread_id);
                          return next;
                        });
                      }}
                      aria-label={`选择会话 ${thread.title}`}
                      className="mt-0.5 h-4 w-4 shrink-0 accent-[var(--nk-accent)]"
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate text-xs font-medium">{thread.title}</p>
                      {thread.blockers.length > 0 && (
                        <div className="mt-2 space-y-1.5">
                          {thread.blockers.map((blocker, index) => (
                            <div
                              key={`${blocker.reason}-${index}`}
                              className="rounded-lg bg-[var(--nk-danger-soft)] px-2.5 py-2 text-[11px] text-[var(--nk-danger)]"
                            >
                              <p>阻塞原因：{blocker.reason}</p>
                              <p className="mt-0.5 opacity-80">下一步：{blocker.next_step}</p>
                              <div className="mt-1.5 flex flex-wrap gap-1.5">
                                <button
                                  onClick={() => void closeChatGptAndScan()}
                                  disabled={loading || migrating || opening !== null || closing}
                                  className="nk-btn-primary whitespace-nowrap px-2 py-1 text-[11px]"
                                >
                                  {closing ? "关闭中…" : "关闭 ChatGPT 并检查"}
                                </button>
                                <button
                                  onClick={retryScan}
                                  disabled={loading || migrating || opening !== null || closing}
                                  className="nk-btn-secondary whitespace-nowrap px-2 py-1 text-[11px]"
                                >
                                  {loading ? "检查中…" : "重新检查"}
                                </button>
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                      <p className="mt-1 text-[11px] text-[var(--nk-muted)]">
                        {thread.archived
                            ? "已归档"
                            : thread.needs_migration
                            ? `待同步到${SYNC_TARGET_LABELS[targetProvider]}`
                            : thread.can_continue
                              ? "可续接"
                              : "本地检查发现阻塞"}
                        {codexProviderLabel(thread.provider) && ` · ${codexProviderLabel(thread.provider)}`}
                        {` · 会话 ID：${thread.thread_id}`}
                        {` · ${formatSessionTime(thread.updated_at)}`}
                      </p>
                    </div>
                    {thread.can_continue && (
                      <button
                        onClick={() => void openThread(thread)}
                        disabled={opening !== null || migrating}
                        className="nk-btn-ghost shrink-0 whitespace-nowrap px-2.5"
                        aria-label="在 Codex 中打开当前会话"
                        title="在 Codex 中打开"
                      >
                        <span>{isOpening ? "打开中…" : "打开"}</span>
                        {!isOpening && <ArrowRightIcon />}
                      </button>
                    )}
                  </div>
                );
              })}
              <div className="flex items-center justify-center gap-3 pt-2">
                <button
                  onClick={() => {
                    setSelectedIds(new Set());
                    void load(page.page - 1);
                  }}
                  disabled={loading || page.page <= 1}
                  className="nk-btn-secondary px-2.5"
                >
                  上一页
                </button>
                <span className="text-xs text-[var(--nk-muted)]">
                  {page.total_pages > 0 ? `第 ${page.page} / ${page.total_pages} 页` : "暂无页面"}
                </span>
                <button
                  onClick={() => {
                    setSelectedIds(new Set());
                    void load(page.page + 1);
                  }}
                  disabled={loading || page.page >= page.total_pages}
                  className="nk-btn-secondary px-2.5"
                >
                  下一页
                </button>
              </div>
            </section>
          ) : (
            <p className="nk-empty">
              {query.trim() ? "没有匹配的会话，请换一个关键词。" : "还没有找到可继续的会话。"}
            </p>
          )}

        </div>
      </main>
    </div>
  );
}
