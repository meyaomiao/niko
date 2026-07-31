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
  type CodexSessionPage,
  type CodexSessionThread,
  unmountRequests,
} from "../lib/codexSessions";

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

  const setPage = (value: CodexSessionPage | null) => {
    pageRef.current = value;
    setPageState(value);
  };

  const setQuery = (value: string) => {
    queryRef.current = value;
    setQueryState(value);
  };

  const load = async (append: boolean) => {
    const request = beginRequest(guard.current, "scan");
    guard.current = request.state;
    setLoading(true);
    setError(null);
    try {
      const current = pageRef.current;
      const result = await invoke<CodexSessionPage>("scan_codex_session_inventory", {
        query: boundSessionQuery(queryRef.current),
        cursor: append ? current?.next_cursor ?? null : null,
        limit: SESSION_PAGE_SIZE,
      });
      if (!acceptsResponse(guard.current, "scan", request.generation)) return;
      setPage(
        append && current
          ? { ...result, items: [...current.items, ...result.items] }
          : result,
      );
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
    const timer = window.setTimeout(() => void load(false), 150);
    return () => window.clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    guard.current = mountRequests(guard.current);
    return () => {
      guard.current = unmountRequests(guard.current);
    };
  }, []);

  const openThread = async (thread: CodexSessionThread) => {
    if (!thread.can_continue) return;
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
          onClick={() => void load(false)}
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
              <span className="nk-pill shrink-0">{page?.items.length ?? 0} 个会话</span>
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
          </section>

          {loading && !page ? (
            <p className="nk-empty" role="status">正在检查会话…</p>
          ) : page?.items.length ? (
            <section className="space-y-2" aria-label="本地会话列表">
              {page.items.map((thread) => {
                const isOpening = opening === thread.thread_id;
                return (
                  <div key={thread.thread_id} className="nk-row flex items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-xs">
                        {thread.title || "未命名会话"}
                      </p>
                      <p className="mt-1 text-[11px] text-[var(--nk-muted)]">
                        {thread.archived
                          ? "已归档"
                          : thread.can_continue
                            ? "可以继续"
                            : "暂时无法继续"}
                      </p>
                    </div>
                    <button
                      onClick={() => void openThread(thread)}
                      disabled={!thread.can_continue || opening !== null}
                      className="nk-btn-secondary px-2.5"
                      aria-label="继续当前会话"
                    >
                      <span>{isOpening ? "打开中…" : "继续"}</span>
                      {!isOpening && <ArrowRightIcon />}
                    </button>
                  </div>
                );
              })}
              {page.next_cursor && (
                <button
                  onClick={() => void load(true)}
                  disabled={loading}
                  className="nk-btn-secondary mx-auto block"
                >
                  {loading ? "加载中…" : "加载更多"}
                </button>
              )}
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
