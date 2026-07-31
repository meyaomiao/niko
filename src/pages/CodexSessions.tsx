import { useEffect, useMemo, useState } from "react";
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
  codexNormalizationLabel,
  codexProviderLabel,
  filterCodexSessionThreads,
  type CodexSessionInventory,
  type CodexSessionThread,
} from "../lib/codexSessions";

const EMPTY_INVENTORY: CodexSessionInventory = {
  codex_home: "",
  active_provider: null,
  defined_providers: [],
  provider_layout: "empty",
  layout_hint: "还没有本地会话",
  normalization_status: "no_changes",
  normalization_target_provider: "custom",
  session_index_entries: null,
  thread_count: 0,
  archived_thread_count: 0,
  diagnostics: [],
  threads: [],
};

function scanErrorMessage(): string {
  return "本地会话暂时无法读取，请稍后再试。";
}

export default function CodexSessions() {
  const navigate = useNavigate();
  const signedIn = Boolean(loadAuth()?.accessToken);
  const [inventory, setInventory] = useState<CodexSessionInventory | null>(null);
  const [query, setQuery] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [opening, setOpening] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);

  const scan = async () => {
    setLoading(true);
    setError(null);
    try {
      setInventory(await invoke<CodexSessionInventory>("scan_codex_session_inventory"));
    } catch {
      setInventory(null);
      setError(scanErrorMessage());
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void scan();
  }, []);

  const visibleThreads = useMemo(
    () => filterCodexSessionThreads(inventory?.threads ?? [], query),
    [inventory, query],
  );

  const openThread = async (thread: CodexSessionThread) => {
    setOpening(thread.thread_id);
    setOpenError(null);
    try {
      await invoke("open_codex_thread", { threadId: thread.thread_id });
    } catch {
      setOpenError("打开会话失败，请确认 ChatGPT 桌面端已安装。");
    } finally {
      setOpening(null);
    }
  };

  const view = inventory ?? EMPTY_INVENTORY;
  const blockerCount = view.diagnostics.filter((diagnostic) => diagnostic.level === "blocker").length;

  return (
    <div className="nk-shell">
      <header className="nk-header justify-between">
        <div className="flex min-w-0 items-center gap-2">
          <button
            onClick={() => navigate(signedIn ? "/home" : "/login")}
            className="nk-btn-ghost px-2.5"
            aria-label={signedIn ? "返回首页" : "返回登录"}
          >
            <ArrowLeftIcon />
          </button>
          <BookOpenIcon />
          <h1 className="nk-title">本地会话</h1>
        </div>
        <button
          onClick={() => void scan()}
          disabled={loading}
          className="nk-btn-ghost px-2.5"
          aria-label="重新检查本地会话"
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
                <h2 className="mt-1 text-base font-semibold">{view.layout_hint}</h2>
              </div>
              <div className="flex shrink-0 items-center gap-2 text-xs text-[var(--nk-muted)]">
                <span>{view.thread_count} 个会话</span>
                {view.archived_thread_count > 0 && <span>· {view.archived_thread_count} 个已归档</span>}
              </div>
            </div>
            <div className="mt-4 flex flex-col gap-2 sm:flex-row sm:items-center">
              <input
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                className="nk-input min-w-0 flex-1 text-xs"
                placeholder="搜索本地会话"
                aria-label="搜索本地会话"
              />
              <span className="nk-pill self-start sm:self-auto">
                {codexNormalizationLabel(view.normalization_status)}
              </span>
            </div>
            {error && <p className="nk-alert-danger mt-3" role="alert">{error}</p>}
            {openError && <p className="nk-alert-warning mt-3" role="alert">{openError}</p>}
            {blockerCount > 0 && !error && (
              <p className="nk-alert-warning mt-3">有部分会话暂时无法读取，请重新检查。</p>
            )}
          </section>

          {loading && !inventory ? (
            <div className="nk-empty flex items-center justify-center gap-2" role="status">
              <span className="nk-spinner h-4 w-4" aria-hidden="true" />
              正在检查本地会话…
            </div>
          ) : visibleThreads.length === 0 ? (
            <p className="nk-empty">
              {query.trim() ? "没有匹配的会话" : "还没有找到本地会话"}
            </p>
          ) : (
            <section aria-label="本地会话列表" className="space-y-2">
              {visibleThreads.map((thread) => {
                const isOpening = opening === thread.thread_id;
                return (
                  <div key={thread.thread_id} className="nk-row flex items-center gap-3">
                    <div className="min-w-0 flex-1">
                      <p className="truncate font-mono text-xs text-[var(--nk-ink)]">
                        {thread.thread_id}
                      </p>
                      <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-[var(--nk-muted)]">
                        <span>{thread.providers.length > 0 ? thread.providers.map(codexProviderLabel).join("、") : "本地"}</span>
                        <span>·</span>
                        <span>{thread.archived ? "已归档" : "可继续"}</span>
                        <span>·</span>
                        <span>{thread.rollout_count} 次记录</span>
                      </div>
                    </div>
                    <button
                      onClick={() => void openThread(thread)}
                      disabled={opening !== null}
                      className="nk-btn-secondary px-2.5"
                      aria-label={`继续会话 ${thread.thread_id.slice(0, 8)}`}
                      title="继续原会话"
                    >
                      <span>{isOpening ? "打开中…" : "继续"}</span>
                      {!isOpening && <ArrowRightIcon />}
                    </button>
                  </div>
                );
              })}
            </section>
          )}
        </div>
      </main>
    </div>
  );
}
