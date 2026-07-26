import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type UsageLogItem } from "../api/client";

function quotaToUSD(q: number) {
  return (q / 1_000_000).toFixed(6);
}

export default function Usage() {
  const auth = loadAuth();
  const navigate = useNavigate();
  const [logs, setLogs] = useState<UsageLogItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!auth?.accessToken) return;
    api
      .usage(auth.accessToken)
      .then((data) => setLogs(data.items ?? []))
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  return (
    <div className="flex h-screen flex-col bg-gray-950">
      <header className="flex items-center gap-3 border-b border-gray-800 px-6 py-4">
        <button
          onClick={() => navigate("/home")}
          className="text-gray-500 transition hover:text-white"
        >
          ←
        </button>
        <h1 className="text-sm font-semibold text-white">用量明细</h1>
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-2xl">
          {loading && (
            <p className="text-center text-sm text-gray-500">加载中…</p>
          )}
          {error && (
            <p className="text-center text-sm text-red-400">{error}</p>
          )}
          {!loading && !error && logs.length === 0 && (
            <p className="text-center text-sm text-gray-500">暂无用量记录</p>
          )}
          {!loading && logs.length > 0 && (
            <table className="w-full text-xs text-gray-300">
              <thead>
                <tr className="border-b border-gray-800 text-left text-gray-500">
                  <th className="pb-2 pr-4">时间</th>
                  <th className="pb-2 pr-4">模型</th>
                  <th className="pb-2 pr-4 text-right">输入</th>
                  <th className="pb-2 pr-4 text-right">输出</th>
                  <th className="pb-2 text-right">消费</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((l) => (
                  <tr key={l.id} className="border-b border-gray-800/50">
                    <td className="py-2 pr-4 text-gray-500">
                      {new Date(l.created_at * 1000).toLocaleString("zh-CN", {
                        month: "2-digit",
                        day: "2-digit",
                        hour: "2-digit",
                        minute: "2-digit",
                      })}
                    </td>
                    <td className="py-2 pr-4 font-mono">{l.model_name}</td>
                    <td className="py-2 pr-4 text-right">{l.prompt_tokens.toLocaleString()}</td>
                    <td className="py-2 pr-4 text-right">{l.completion_tokens.toLocaleString()}</td>
                    <td className="py-2 text-right text-indigo-400">${quotaToUSD(l.quota)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </main>
    </div>
  );
}
