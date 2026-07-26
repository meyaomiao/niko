import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth } from "../store/auth";

interface TargetInfo {
  id: string;
  name: string;
  installed: boolean;
}

interface ApplyResult {
  id: string;
  ok: boolean;
  changed?: string[];
  error?: string;
}

// E8-2: 进程状态
interface ProcessStatus {
  target_id: string;
  running: boolean;
  pid?: number;
}

// E4-3: 兼容等级
type CompatLevel = "full" | "partial" | "none";

const TARGET_COMPAT: Record<string, { level: CompatLevel; note: string }> = {
  "codex": { level: "full", note: "完整支持：API Key、Base URL、模型选择" },
  "claude-desktop": { level: "partial", note: "部分支持：通过 MCP 代理，原生工具调用不可用" },
  "claude-code": { level: "full", note: "完整支持：API Key、Base URL 均可覆盖" },
};

const TARGET_ICONS: Record<string, string> = {
  "codex": "⌨️",
  "claude-desktop": "🖥️",
  "claude-code": "💻",
};

const TARGET_DESC: Record<string, string> = {
  "codex": "写入 ~/.codex/auth.json 和 config.toml",
  "claude-desktop": "写入 claude_desktop_config.json（mcpServers）",
  "claude-code": "写入 ~/.claude/settings.json",
};

function CompatBadge({ level, note }: { level: CompatLevel; note: string }) {
  const styles: Record<CompatLevel, string> = {
    full: "bg-green-900/30 text-green-400",
    partial: "bg-yellow-900/30 text-yellow-400",
    none: "bg-gray-800 text-gray-500",
  };
  const labels: Record<CompatLevel, string> = {
    full: "完整兼容",
    partial: "部分兼容",
    none: "不支持",
  };
  return (
    <span title={note} className={`inline-block rounded-full px-2 py-0.5 text-xs ${styles[level]}`}>
      {labels[level]}
    </span>
  );
}

export default function Targets() {
  const auth = loadAuth();
  const [targets, setTargets] = useState<TargetInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [applying, setApplying] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ApplyResult>>({});
  const [procs, setProcs] = useState<Record<string, boolean>>({});

  useEffect(() => {
    invoke<TargetInfo[]>("list_targets")
      .then(setTargets)
      .catch(console.error)
      .finally(() => setLoading(false));
    // E8-2: 同步获取进程状态
    invoke<ProcessStatus[]>("check_all_processes")
      .then((list) => {
        const m: Record<string, boolean> = {};
        list.forEach((p) => { m[p.target_id] = p.running; });
        setProcs(m);
      })
      .catch(() => {});
  }, []);

  const applyOne = async (targetId: string) => {
    if (!auth) return;
    setApplying(targetId);
    try {
      const changed = await invoke<string[]>("apply_target", {
        req: {
          target_id: targetId,
          base_url: "https://momotoken.win/v1",
          api_key: auth.apiKey,
          model_group: auth.group || null,
        },
      });
      setResults((r) => ({ ...r, [targetId]: { id: targetId, ok: true, changed } }));
    } catch (e) {
      setResults((r) => ({ ...r, [targetId]: { id: targetId, ok: false, error: String(e) } }));
    } finally {
      setApplying(null);
    }
  };

  const applyAll = async () => {
    if (!auth) return;
    setApplying("__all__");
    try {
      const res = await invoke<ApplyResult[]>("apply_all_targets", {
        baseUrl: "https://momotoken.win/v1",
        apiKey: auth.apiKey,
        modelGroup: auth.group || null,
      });
      const map: Record<string, ApplyResult> = {};
      res.forEach((r) => { map[r.id] = r; });
      setResults(map);
    } catch (e) {
      console.error(e);
    } finally {
      setApplying(null);
    }
  };

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-950">
        <span className="text-gray-400">检测中…</span>
      </div>
    );
  }

  const installedCount = targets.filter((t) => t.installed).length;

  return (
    <div className="flex h-screen flex-col bg-gray-950">
      <header className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
        <div>
          <h1 className="text-sm font-semibold text-white">接入目标</h1>
          <p className="text-xs text-gray-500">
            检测到 {installedCount}/{targets.length} 个应用已安装
          </p>
        </div>
        <button
          onClick={applyAll}
          disabled={applying !== null || installedCount === 0}
          className="rounded-lg bg-indigo-600 px-4 py-1.5 text-xs font-medium text-white transition hover:bg-indigo-500 disabled:opacity-40"
        >
          {applying === "__all__" ? "应用中…" : "一键配置全部"}
        </button>
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-lg space-y-3">
          {targets.map((t) => {
            const result = results[t.id];
            const isBusy = applying === t.id || applying === "__all__";
            const compat = TARGET_COMPAT[t.id] ?? { level: "none" as CompatLevel, note: "未知兼容性" };
            const isRunning = procs[t.id] ?? false;

            return (
              <div
                key={t.id}
                className={`rounded-2xl border p-5 transition ${
                  t.installed ? "border-gray-700 bg-gray-900" : "border-gray-800 bg-gray-900/50 opacity-60"
                }`}
              >
                <div className="flex items-start justify-between gap-4">
                  <div className="flex items-start gap-3 min-w-0">
                    <span className="mt-0.5 text-xl shrink-0">{TARGET_ICONS[t.id] ?? "🔧"}</span>
                    <div className="min-w-0">
                      <div className="flex items-center gap-2 flex-wrap">
                        <p className="text-sm font-medium text-white">{t.name}</p>
                        <CompatBadge level={compat.level} note={compat.note} />
                        {t.installed && (
                          <span className={`inline-block rounded-full px-2 py-0.5 text-xs ${
                            isRunning ? "bg-green-900/30 text-green-400" : "bg-gray-800 text-gray-500"
                          }`}>
                            {isRunning ? "● 运行中" : "○ 未运行"}
                          </span>
                        )}
                      </div>
                      <p className="mt-0.5 text-xs text-gray-500">{TARGET_DESC[t.id]}</p>
                      {!t.installed && (
                        <p className="mt-1 text-xs text-yellow-600">未检测到安装</p>
                      )}
                    </div>
                  </div>
                  <button
                    onClick={() => applyOne(t.id)}
                    disabled={isBusy || !t.installed || !auth?.apiKey}
                    className="shrink-0 rounded-lg bg-gray-700 px-3 py-1.5 text-xs text-gray-200 transition hover:bg-gray-600 disabled:opacity-40"
                  >
                    {isBusy ? "…" : "配置"}
                  </button>
                </div>

                {result && (
                  <div className={`mt-3 rounded-lg px-3 py-2 text-xs ${
                    result.ok ? "bg-green-900/30 text-green-400" : "bg-red-900/30 text-red-400"
                  }`}>
                    {result.ok ? (
                      result.changed && result.changed.length > 0
                        ? <>✓ 已更新：{result.changed.join("、")}</>
                        : <>✓ 配置已是最新，无需变更</>
                    ) : (
                      <>✗ {result.error}</>
                    )}
                  </div>
                )}
              </div>
            );
          })}

          {targets.length === 0 && (
            <p className="text-center text-sm text-gray-500">未找到任何支持的接入目标</p>
          )}
        </div>
      </main>
    </div>
  );
}
