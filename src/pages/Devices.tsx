import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type DeviceItem } from "../api/client";

function formatTime(ts: number): string {
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

function PlatformIcon({ platform }: { platform: string }) {
  const p = platform.toLowerCase();
  if (p.includes("mac") || p.includes("darwin")) return <>🍎</>;
  if (p.includes("win")) return <>🪟</>;
  if (p.includes("linux")) return <>🐧</>;
  return <>💻</>;
}

export default function Devices() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<number | "others" | null>(null);

  const load = () => {
    if (!auth?.accessToken) return;
    setLoading(true);
    setError(null);
    api
      .listDevices(auth.accessToken)
      .then(setDevices)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };

  useEffect(load, []); // eslint-disable-line react-hooks/exhaustive-deps

  const handleRevoke = async (id: number) => {
    if (!auth?.accessToken) return;
    setRevoking(id);
    try {
      await api.revokeDevice(auth.accessToken, id);
      setDevices((d) => d.filter((x) => x.id !== id));
    } catch (e) {
      setError(String(e));
    } finally {
      setRevoking(null);
    }
  };

  const handleRevokeOthers = async () => {
    if (!auth?.accessToken) return;
    setRevoking("others");
    try {
      await api.revokeOtherDevices(auth.accessToken);
      setDevices((d) => d.filter((x) => x.is_current));
    } catch (e) {
      setError(String(e));
    } finally {
      setRevoking(null);
    }
  };

  const otherCount = devices.filter((d) => !d.is_current).length;

  return (
    <div className="flex h-screen flex-col bg-gray-950">
      <header className="flex items-center justify-between border-b border-gray-800 px-6 py-4">
        <div className="flex items-center gap-3">
          <button
            onClick={() => navigate(-1)}
            className="text-gray-500 transition hover:text-white"
          >
            ←
          </button>
          <h1 className="text-sm font-semibold text-white">设备管理</h1>
        </div>
        {otherCount > 0 && (
          <button
            onClick={handleRevokeOthers}
            disabled={revoking !== null}
            className="rounded-lg bg-red-900/40 px-3 py-1.5 text-xs text-red-400 transition hover:bg-red-900/60 disabled:opacity-40"
          >
            {revoking === "others" ? "操作中…" : `踢出其他 ${otherCount} 台设备`}
          </button>
        )}
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-lg space-y-3">
          {loading && (
            <p className="text-center text-sm text-gray-500">加载中…</p>
          )}
          {error && (
            <p className="text-center text-sm text-red-400">{error}</p>
          )}
          {!loading && !error && devices.length === 0 && (
            <p className="text-center text-sm text-gray-500">暂无设备记录</p>
          )}
          {devices.map((d) => (
            <div
              key={d.id}
              className={`rounded-2xl border p-4 transition ${
                d.is_current
                  ? "border-indigo-700/50 bg-indigo-950/30"
                  : "border-gray-800 bg-gray-900"
              }`}
            >
              <div className="flex items-start justify-between gap-4">
                <div className="flex items-start gap-3">
                  <span className="mt-0.5 text-xl">
                    <PlatformIcon platform={d.platform} />
                  </span>
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <p className="text-sm font-medium text-white truncate">
                        {d.device_name || d.device_id.slice(0, 8)}
                      </p>
                      {d.is_current && (
                        <span className="shrink-0 rounded-full bg-indigo-600/30 px-2 py-0.5 text-xs text-indigo-400">
                          当前
                        </span>
                      )}
                    </div>
                    <p className="mt-0.5 text-xs text-gray-500">
                      {d.platform} · v{d.app_version}
                    </p>
                    <p className="mt-0.5 text-xs text-gray-600">
                      最后活跃 {formatTime(d.accessed_time)}
                    </p>
                  </div>
                </div>
                {!d.is_current && (
                  <button
                    onClick={() => handleRevoke(d.id)}
                    disabled={revoking !== null}
                    className="shrink-0 rounded-lg bg-gray-700 px-3 py-1.5 text-xs text-gray-300 transition hover:bg-red-900/50 hover:text-red-400 disabled:opacity-40"
                  >
                    {revoking === d.id ? "…" : "撤销"}
                  </button>
                )}
              </div>
            </div>
          ))}
        </div>
      </main>
    </div>
  );
}
