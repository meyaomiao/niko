import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth, saveAuth } from "../store/auth";
import { api, type BootstrapData } from "../api/client";
import { useSession } from "../hooks/useSession";

// 将 quota 数值转换为美元显示（quota 单位 = 0.000001 USD）
function quotaToUSD(quota: number): string {
  return (quota / 1_000_000).toFixed(4);
}

function QuotaCard({ quota, username, group }: { quota: number; username: string; group: string }) {
  const pct = Math.min(100, Math.max(0, (quota / 10_000_000) * 100));
  return (
    <div className="rounded-2xl bg-gray-900 p-6 shadow-lg">
      <div className="mb-4 flex items-center justify-between">
        <div>
          <p className="text-xs text-gray-400">欢迎回来</p>
          <p className="text-lg font-semibold text-white">{username}</p>
        </div>
        <span className="rounded-full bg-indigo-600/20 px-3 py-1 text-xs text-indigo-400">
          {group || "default"}
        </span>
      </div>
      <div className="mb-1 flex items-end justify-between">
        <span className="text-xs text-gray-400">可用余额</span>
        <span className="text-2xl font-bold text-white">${quotaToUSD(quota)}</span>
      </div>
      <div className="h-1.5 w-full overflow-hidden rounded-full bg-gray-800">
        <div
          className="h-full rounded-full bg-indigo-500 transition-all"
          style={{ width: `${pct}%` }}
        />
      </div>
    </div>
  );
}

function ModelCard({
  models,
  selectedGroup,
  apiKey,
}: {
  models: string[];
  selectedGroup: string;
  apiKey: string;
}) {
  const [copied, setCopied] = useState(false);

  const copyKey = () => {
    if (!apiKey) return;
    navigator.clipboard.writeText(apiKey).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    });
  };

  return (
    <div className="rounded-2xl bg-gray-900 p-6 shadow-lg">
      <div className="mb-4 flex items-center justify-between">
        <h2 className="text-sm font-medium text-gray-200">当前分组可用模型</h2>
        <span className="text-xs text-gray-500">{models.length} 个</span>
      </div>

      {models.length === 0 ? (
        <p className="text-sm text-gray-500">暂无模型</p>
      ) : (
        <div className="flex max-h-40 flex-wrap gap-2 overflow-y-auto pr-1">
          {models.map((m) => (
            <span
              key={m}
              className="rounded-lg bg-gray-800 px-2 py-1 text-xs text-gray-300"
            >
              {m}
            </span>
          ))}
        </div>
      )}

      {apiKey && (
        <div className="mt-4 rounded-lg bg-gray-800 p-3">
          <p className="mb-1 text-xs text-gray-400">
            分组 <span className="text-indigo-400">{selectedGroup}</span> 的 API Key
          </p>
          <div className="flex items-center gap-2">
            <code className="flex-1 truncate text-xs text-green-400">
              {apiKey.slice(0, 12)}{"•".repeat(8)}
            </code>
            <button
              onClick={copyKey}
              className="shrink-0 rounded-md bg-gray-700 px-2 py-1 text-xs text-gray-300 transition hover:bg-gray-600"
            >
              {copied ? "已复制" : "复制"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}

export default function Home() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const { handleSessionExpired } = useSession();

  const [bootstrap, setBootstrap] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);

  // 退出登录
  const logout = async () => {
    if (auth?.accessToken) {
      try { await api.logout(auth.accessToken); } catch { /* ignore */ }
    }
    handleSessionExpired();
  };

  useEffect(() => {
    if (!auth?.accessToken) { navigate("/login", { replace: true }); return; }
    api.bootstrap(auth.accessToken)
      .then((data) => {
        setBootstrap(data);
        saveAuth({ ...auth, quota: data.user.quota, group: data.user.group });
      })
      .catch(() => { /* 用缓存数据展示，等 useSession 定期刷新 */ })
      .finally(() => setLoading(false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center bg-gray-950">
        <div className="text-gray-400">加载中…</div>
      </div>
    );
  }

  const quota = bootstrap?.user.quota ?? auth?.quota ?? 0;
  const group = bootstrap?.user.group ?? auth?.group ?? "";
  const username = auth?.username ?? "";
  const models = bootstrap?.models ?? [];
  const apiKey = auth?.apiKey ?? "";

  return (
    <div className="flex h-screen flex-col bg-gray-950">
      {/* 顶部导航 */}
      <header className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
        <span className="text-sm font-semibold text-white">🐾 momo·摸摸</span>
        <nav className="flex items-center gap-4 text-xs text-gray-400">
          <button
            onClick={() => navigate("/targets")}
            className="transition hover:text-white"
          >
            接入目标
          </button>
          <button
            onClick={() => navigate("/devices")}
            className="transition hover:text-white"
          >
            设备
          </button>
          <button
            onClick={() => navigate("/usage")}
            className="transition hover:text-white"
          >
            用量
          </button>
          <button
            onClick={() => navigate("/settings")}
            className="transition hover:text-white"
          >
            设置
          </button>
          <button
            onClick={logout}
            className="rounded-md bg-gray-800 px-3 py-1 transition hover:bg-gray-700"
          >
            退出
          </button>
        </nav>
      </header>

      {/* 主内容 */}
      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-lg space-y-4">
          <QuotaCard quota={quota} username={username} group={group} />
          <ModelCard models={models} selectedGroup={group} apiKey={apiKey} />
        </div>
      </main>
    </div>
  );
}
