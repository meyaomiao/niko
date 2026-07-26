import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import {
  baselineFor,
  COMPAT_LABEL,
  COMPAT_STYLE,
  formatCheckedAt,
  type CompatLevel,
  type CompatProbe,
} from "../lib/compat";

const BASE_URL = "https://momotoken.win/v1";

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

// E7-2: 兼容等级 badge，实测过的会带 ✓/! 标记
function CompatBadge({
  level,
  note,
  measured,
}: {
  level: CompatLevel;
  note: string;
  measured?: "ok" | "fail";
}) {
  const mark = measured === "ok" ? "✓ " : measured === "fail" ? "! " : "";
  return (
    <span title={note} className={`inline-block rounded-full px-2 py-0.5 text-xs ${COMPAT_STYLE[level]}`}>
      {mark}{COMPAT_LABEL[level]}
    </span>
  );
}

export default function Targets() {
  const auth = loadAuth();
  const navigate = useNavigate();
  const [targets, setTargets] = useState<TargetInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [applying, setApplying] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ApplyResult>>({});
  const [procs, setProcs] = useState<Record<string, boolean>>({});
  // E7-2: 实测结果
  const [probes, setProbes] = useState<Record<string, CompatProbe>>({});
  const [probing, setProbing] = useState(false);
  const probeModel = auth?.group ? `${auth.group} 分组默认模型` : "";

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

  // E7-2: 用当前分组的默认模型实测每个目标，实测结果只降级不升级
  const runProbe = async () => {
    if (!auth?.apiKey) return;
    setProbing(true);
    try {
      const model = await invoke<string>("resolve_model_cmd", {
        role: "balanced",
        group: auth.group || null,
      });
      const baselines: Record<string, string> = {};
      targets.forEach((t) => {
        baselines[t.id] = baselineFor(t.id, model).level;
      });
      const list = await invoke<CompatProbe[]>("probe_compat", {
        baseUrl: BASE_URL,
        apiKey: auth.apiKey,
        model,
        baselines,
      });
      const map: Record<string, CompatProbe> = {};
      list.forEach((p) => { map[p.target_id] = p; });
      setProbes(map);
    } catch (e) {
      console.error(e);
    } finally {
      setProbing(false);
    }
  };

  const applyOne = async (targetId: string) => {
    if (!auth) return;
    setApplying(targetId);
    try {
      const changed = await invoke<string[]>("apply_target", {
        req: {
          target_id: targetId,
          base_url: BASE_URL,
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
        baseUrl: BASE_URL,
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
        <div className="flex items-center gap-3">
          <button
            onClick={() => navigate("/home")}
            className="text-gray-500 transition hover:text-white"
          >
            ←
          </button>
          <div>
            <h1 className="text-sm font-semibold text-white">接入目标</h1>
            <p className="text-xs text-gray-500">
              检测到 {installedCount}/{targets.length} 个应用已安装
            </p>
          </div>
        </div>
        <div className="flex items-center gap-2">
          <button
            onClick={runProbe}
            disabled={probing || installedCount === 0 || !auth?.apiKey}
            className="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-300 transition hover:bg-gray-800 disabled:opacity-40"
          >
            {probing ? "实测中…" : "实测兼容性"}
          </button>
          <button
            onClick={applyAll}
            disabled={applying !== null || installedCount === 0}
            className="rounded-lg bg-indigo-600 px-4 py-1.5 text-xs font-medium text-white transition hover:bg-indigo-500 disabled:opacity-40"
          >
            {applying === "__all__" ? "应用中…" : "一键配置全部"}
          </button>
        </div>
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-lg space-y-3">
          {targets.map((t) => {
            const result = results[t.id];
            const isBusy = applying === t.id || applying === "__all__";
            const probe = probes[t.id];
            const baseline = baselineFor(t.id, probe?.model ?? probeModel);
            // 实测过就以实测等级为准（只降不升），否则显示基线
            const level = probe?.level ?? baseline.level;
            const note = probe?.detail ?? baseline.note;
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
                        <CompatBadge
                          level={level}
                          note={note}
                          measured={probe ? (probe.ok ? "ok" : "fail") : undefined}
                        />
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
                      {probe && (
                        <p className={`mt-1 text-xs ${probe.ok ? "text-gray-500" : "text-yellow-600"}`}>
                          实测 {probe.model}
                          {probe.ok
                            ? ` 通过${probe.latency_ms != null ? `（${probe.latency_ms}ms）` : ""}`
                            : ` 未通过：${probe.detail ?? probe.error_kind ?? "未知原因"}`}
                          {probe.checked_at ? ` · ${formatCheckedAt(probe.checked_at)}` : ""}
                        </p>
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
