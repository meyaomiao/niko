import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
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
  codexNormalizationLabel,
  initialRequestGuard,
  mountRequests,
  safeFailure,
  SESSION_PAGE_SIZE,
  SESSION_QUERY_LIMIT,
  type CodexSessionMutationOutcome,
  type CodexSessionPage,
  type CodexSessionThread,
  unmountRequests,
} from "../lib/codexSessions";

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
  const [page, setPageState] = useState<CodexSessionPage | null>(null);
  const [query, setQueryState] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [migrating, setMigrating] = useState(false);
  const [migrationMessage, setMigrationMessage] = useState<string | null>(null);
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

  const load = async (requestedPage: number) => {
    const request = beginRequest(guard.current, "scan");
    guard.current = request.state;
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<CodexSessionPage>("scan_codex_session_inventory", {
        query: boundSessionQuery(queryRef.current),
        page: requestedPage,
        page_size: SESSION_PAGE_SIZE,
      });
      if (!acceptsResponse(guard.current, "scan", request.generation)) return;
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

  useEffect(() => {
    const timer = window.setTimeout(() => void load(1), 150);
    return () => window.clearTimeout(timer);
  }, [query]);

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
        setError(safeFailure(rejection).message);
      }
    } finally {
      if (acceptsResponse(guard.current, "action", request.generation)) {
        setOpening(null);
      }
    }
  };

  const migrateToCustom = async (threadIds?: string[]) => {
    const selected = threadIds && threadIds.length > 0 ? Array.from(new Set(threadIds)) : null;
    if (migrating || opening !== null || page?.status === "blocked") return;
    if (selected ? selected.length === 0 : page?.status !== "needs_check") return;
    const prompt = selected
      ? `将把选中的 ${selected.length} 个会话迁移到 custom，并自动备份后原子更新。是否继续？`
      : "将把所有待迁移的 Codex 会话统一迁移到 custom，并自动备份后原子更新。是否继续？";
    if (!window.confirm(prompt)) {
      return;
    }
    const request = beginRequest(guard.current, "action");
    guard.current = request.state;
    setMigrating(true);
    setError(null);
    setMigrationMessage(null);
    try {
      const result = await invoke<CodexSessionMutationOutcome>(
        selected ? "normalize_codex_session_storage_selected" : "normalize_codex_session_storage",
        selected
          ? { targetProvider: "custom", threadIds: selected }
          : { targetProvider: "custom" },
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
          <h1 className="nk-title">本地会话</h1>
        </div>
        <button
          onClick={() => void load(pageRef.current?.page ?? 1)}
          disabled={loading}
          className="nk-btn-ghost px-2.5"
          aria-label="重新检查会话"
          title="重新检查"
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
            {error && <p className="nk-alert-danger mt-3" role="alert">{error}</p>}
            {migrationMessage && <p className="nk-alert-success mt-3" role="status">{migrationMessage}</p>}
          </section>

          {loading && !page ? (
            <p className="nk-empty" role="status">正在检查会话…</p>
          ) : page?.items.length ? (
            <section className="space-y-2" aria-label="本地会话列表">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex items-center gap-2 text-xs text-[var(--nk-muted)]">
                  <span>已选 {selectedIds.size} 个</span>
                  <button
                    onClick={togglePageSelection}
                    disabled={selectableItems.length === 0 || migrating || opening !== null}
                    className="nk-btn-secondary px-2.5"
                  >
                    {allSelectableSelected ? "取消全选" : "全选待修复"}
                  </button>
                  <button
                    onClick={() => setSelectedIds(new Set())}
                    disabled={selectedIds.size === 0 || migrating}
                    className="nk-btn-secondary px-2.5"
                  >
                    清空选择
                  </button>
                </div>
                <div className="flex flex-wrap items-center gap-2">
                  {selectedIds.size > 0 && (
                    <button
                      onClick={() => void migrateToCustom(Array.from(selectedIds))}
                      disabled={migrating || opening !== null || page.status === "blocked"}
                      className="nk-btn-primary px-2.5"
                    >
                      {migrating ? "修复中…" : `修复选中 (${selectedIds.size})`}
                    </button>
                  )}
                <button
                  onClick={() => void migrateToCustom()}
                  disabled={page.status !== "needs_check" || migrating || opening !== null}
                  className="nk-btn-secondary px-2.5"
                >
                  {migrating ? "修复中…" : "修复全部待迁移会话"}
                </button>
                </div>
              </div>
              {page.items.map((thread) => {
                const isOpening = opening === thread.thread_id;
                const isSelected = selectedIds.has(thread.thread_id);
                return (
                  <div key={thread.thread_id} className="nk-row flex items-center gap-3">
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
                      aria-label={`选择会话 ${thread.title || thread.thread_id.slice(0, 8)}`}
                      className="h-4 w-4 shrink-0 accent-[var(--nk-accent)]"
                    />
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-xs">{thread.title || `会话 ${thread.thread_id.slice(0, 8)}`}</p>
                      {thread.summary && (
                        <p className="mt-1 truncate text-[11px] text-[var(--nk-muted)]">{thread.summary}</p>
                      )}
                      <p className="mt-1 text-[11px] text-[var(--nk-muted)]">
                        {thread.archived
                          ? "已归档"
                          : thread.needs_migration
                            ? "待迁移到 custom"
                            : thread.can_continue
                              ? "可续接"
                              : "本地检查发现阻塞"}
                        {thread.provider && ` · ${thread.provider}`}
                        {` · ${formatSessionTime(thread.updated_at)}`}
                      </p>
                    </div>
                    {thread.can_continue && (
                      <button
                        onClick={() => void openThread(thread)}
                        disabled={opening !== null || migrating}
                        className="nk-btn-ghost shrink-0 px-2.5"
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
