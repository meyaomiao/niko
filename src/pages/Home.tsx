import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, saveAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type DeviceItem } from "../api/client";
import { useSession } from "../hooks/useSession";
import { useTheme } from "../hooks/useTheme";
import { baselineFor, COMPAT_LABEL, COMPAT_STYLE } from "../lib/compat";
import { buildPricingIndex, priceOf, fmtUSD } from "../lib/pricing";
import { vendorOfGroup } from "../lib/vendor";

const RELAY_BASE_URL = "https://momotoken.win/v1";

interface TargetInfo {
  id: string;
  name: string;
  installed: boolean;
}

interface ApplyResult {
  ok: boolean;
  changed?: string[];
  error?: string;
}

const TARGET_ICONS: Record<string, string> = {
  codex: "⌨️",
  "claude-desktop": "🖥️",
  "claude-code": "💻",
};

function quotaToUSD(quota: number): string {
  return (quota / 1_000_000).toFixed(2);
}

function formatTime(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

const CARD = "rounded-2xl border border-black/5 bg-white p-5 shadow-sm dark:border-white/10 dark:bg-white/5";
const LABEL = "text-xs font-medium text-gray-500 dark:text-gray-400";
const TITLE = "text-sm font-semibold text-gray-900 dark:text-gray-100";
const SUBTLE = "text-xs text-gray-500 dark:text-gray-400";
const GHOST_BTN =
  "rounded-full border border-black/10 px-3 py-1.5 text-xs text-gray-700 transition hover:bg-black/5 disabled:opacity-40 dark:border-white/15 dark:text-gray-200 dark:hover:bg-white/10";
const PRIMARY_BTN =
  "rounded-full bg-gray-900 px-4 py-1.5 text-xs font-medium text-white transition hover:bg-gray-800 disabled:opacity-40 dark:bg-white dark:text-gray-900 dark:hover:bg-gray-200";

export default function Home() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const { handleSessionExpired } = useSession();
  const { theme, toggle } = useTheme();

  const [bootstrap, setBootstrap] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  const [group, setGroup] = useState(auth?.group ?? "");
  const [model, setModel] = useState("");
  const [modelFilter, setModelFilter] = useState("");
  const [apiKey, setApiKey] = useState(auth?.apiKey ?? "");
  const [provisioning, setProvisioning] = useState(false);
  const [notice, setNotice] = useState<{ ok: boolean; text: string } | null>(null);

  const [targets, setTargets] = useState<TargetInfo[]>([]);
  const [applying, setApplying] = useState<string | null>(null);
  const [results, setResults] = useState<Record<string, ApplyResult>>({});

  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [revoking, setRevoking] = useState<number | "others" | null>(null);

  useEffect(() => {
    if (!auth?.accessToken) {
      navigate("/login", { replace: true });
      return;
    }
    api
      .bootstrap(auth.accessToken)
      .then((data) => {
        setBootstrap(data);
        saveAuth({ ...auth, quota: data.user.quota, group: data.user.group });
        const groups = data.groups ?? [];
        // 优先沿用上次选择，其次用户自身分组，最后第一个有模型的分组
        const preferred =
          groups.find((g) => g.name === auth.group) ??
          groups.find((g) => g.name === data.user.group) ??
          groups[0];
        if (preferred) {
          setGroup(preferred.name);
          setModel(preferred.models[0] ?? "");
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));

    invoke<TargetInfo[]>("list_targets").then(setTargets).catch(() => {});
    api.listDevices(auth.accessToken).then(setDevices).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const groups: GroupOption[] = bootstrap?.groups ?? [];
  // 按分组名前缀归类到三家上游，未匹配的统一放「其他」
  const vendorSections = useMemo(() => {
    const buckets: Record<string, GroupOption[]> = { OpenAI: [], Anthropic: [], Google: [], 其他: [] };
    for (const g of groups) {
      buckets[vendorOfGroup(g.name)].push(g);
    }
    return Object.entries(buckets).filter(([, list]) => list.length > 0);
  }, [groups]);
  const currentGroup = groups.find((g) => g.name === group);
  const groupRatio = currentGroup?.ratio ?? 1;
  const pricingIndex = useMemo(() => buildPricingIndex(bootstrap?.pricing), [bootstrap]);
  const models = useMemo(() => {
    const list = currentGroup?.models ?? bootstrap?.models ?? [];
    const kw = modelFilter.trim().toLowerCase();
    return kw ? list.filter((m) => m.toLowerCase().includes(kw)) : list;
  }, [currentGroup, bootstrap, modelFilter]);

  // 列表内联价格：按次计费显示单次价格，否则显示「输入 / 输出」
  const priceLabel = (name: string) => {
    const p = priceOf(pricingIndex.get(name), groupRatio);
    if (!p) return "";
    if (p.perRequest) return `${fmtUSD(p.input)}/次`;
    return `${fmtUSD(p.input)} / ${fmtUSD(p.output)}`;
  };

  const selectedPrice = priceOf(pricingIndex.get(model), groupRatio);

  const pickGroup = (name: string) => {
    setGroup(name);
    setNotice(null);
    const g = groups.find((x) => x.name === name);
    setModel(g?.models[0] ?? "");
  };

  // 申领/切换当前分组的 Key，再写入所有已安装的目标
  const enable = async () => {
    if (!auth?.accessToken || !group) return;
    setProvisioning(true);
    setNotice(null);
    setResults({});
    try {
      const res = await api.provision(auth.accessToken, group);
      setApiKey(res.api_key);
      saveAuth({ ...auth, apiKey: res.api_key, group });

      const applied = await invoke<Array<{ id: string; ok: boolean; changed?: string[]; error?: string }>>(
        "apply_all_targets",
        { baseUrl: RELAY_BASE_URL, apiKey: res.api_key, modelGroup: group, model: model || null }
      );
      const map: Record<string, ApplyResult> = {};
      applied.forEach((r) => {
        map[r.id] = { ok: r.ok, changed: r.changed, error: r.error };
      });
      setResults(map);
      const okCount = applied.filter((r) => r.ok).length;
      setNotice(
        applied.length === 0
          ? { ok: false, text: "未检测到已安装的应用，请先安装 Codex 或 Claude" }
          : { ok: okCount > 0, text: `已为 ${okCount}/${applied.length} 个应用启用 ${model || group}` }
      );
    } catch (e) {
      setNotice({ ok: false, text: String(e instanceof Error ? e.message : e) });
    } finally {
      setProvisioning(false);
    }
  };

  const applyOne = async (targetId: string) => {
    if (!apiKey) {
      setNotice({ ok: false, text: "请先点击“启用”获取密钥" });
      return;
    }
    setApplying(targetId);
    try {
      const changed = await invoke<string[]>("apply_target", {
        req: {
          target_id: targetId,
          base_url: RELAY_BASE_URL,
          api_key: apiKey,
          model_group: group || null,
          model: model || null,
        },
      });
      setResults((r) => ({ ...r, [targetId]: { ok: true, changed } }));
    } catch (e) {
      setResults((r) => ({ ...r, [targetId]: { ok: false, error: String(e) } }));
    } finally {
      setApplying(null);
    }
  };

  const logout = async () => {
    if (auth?.accessToken) {
      try {
        await api.logout(auth.accessToken);
      } catch {
        /* ignore */
      }
    }
    handleSessionExpired();
  };

  const revokeDevice = async (id: number) => {
    if (!auth?.accessToken) return;
    setRevoking(id);
    try {
      await api.revokeDevice(auth.accessToken, id);
      setDevices((d) => d.filter((x) => x.id !== id));
    } catch {
      /* ignore */
    } finally {
      setRevoking(null);
    }
  };

  const revokeOthers = async () => {
    if (!auth?.accessToken) return;
    setRevoking("others");
    try {
      await api.revokeOtherDevices(auth.accessToken);
      setDevices((d) => d.filter((x) => x.is_current));
    } catch {
      /* ignore */
    } finally {
      setRevoking(null);
    }
  };

  if (loading) {
    return (
      <div className="flex h-screen items-center justify-center">
        <span className={SUBTLE}>加载中…</span>
      </div>
    );
  }

  const quota = bootstrap?.user.quota ?? auth?.quota ?? 0;
  const otherDevices = devices.filter((d) => !d.is_current).length;
  const installed = targets.filter((t) => t.installed);

  return (
    <div className="flex h-screen flex-col">
      <header className="flex items-center justify-between border-b border-black/5 px-6 py-3 dark:border-white/10">
        <span className={TITLE}>🐾 momo·摸摸</span>
        <div className="flex items-center gap-2">
          <button onClick={toggle} className={GHOST_BTN} aria-label="切换主题">
            {theme === "dark" ? "☀️" : "🌙"}
          </button>
          <button onClick={() => navigate("/settings")} className={GHOST_BTN}>
            设置
          </button>
          <button onClick={logout} className={GHOST_BTN}>
            退出
          </button>
        </div>
      </header>

      <main className="flex-1 overflow-y-auto px-6 py-5">
        <div className="mx-auto max-w-xl space-y-4">
          {/* 余额 */}
          <section className={CARD}>
            <div className="flex items-end justify-between">
              <div>
                <p className={LABEL}>{auth?.username ?? "已登录"}</p>
                <p className="mt-1 text-3xl font-semibold tracking-tight text-gray-900 dark:text-white">
                  ${quotaToUSD(quota)}
                </p>
                <p className={`mt-1 ${SUBTLE}`}>可用余额</p>
              </div>
              <button onClick={() => navigate("/usage")} className={GHOST_BTN}>
                用量明细
              </button>
            </div>
          </section>

          {/* 分组 + 模型选择 */}
          <section className={CARD}>
            <div className="mb-3 flex items-center justify-between">
              <h2 className={TITLE}>选择模型</h2>
              {currentGroup && (
                <span className={SUBTLE}>价格按每百万 token</span>
              )}
            </div>

            {groups.length === 0 ? (
              <p className={SUBTLE}>当前账号没有可用分组，请联系管理员开通</p>
            ) : (
              <>
                <p className={LABEL}>套餐分组</p>
                <div className="mt-2 space-y-3">
                  {vendorSections.map(([vendor, list]) => (
                    <div key={vendor}>
                      <p className="text-[11px] font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">
                        {vendor}
                      </p>
                      <div className="mt-1.5 flex flex-wrap gap-2">
                        {list.map((g) => (
                          <button
                            key={g.name}
                            onClick={() => pickGroup(g.name)}
                            title={g.desc}
                            className={`rounded-full px-3 py-1.5 text-xs transition ${
                              g.name === group
                                ? "bg-gray-900 text-white dark:bg-white dark:text-gray-900"
                                : "border border-black/10 text-gray-700 hover:bg-black/5 dark:border-white/15 dark:text-gray-200 dark:hover:bg-white/10"
                            }`}
                          >
                            {g.name}
                            <span className="ml-1.5 opacity-60">{g.ratio}x</span>
                          </button>
                        ))}
                      </div>
                    </div>
                  ))}
                </div>
                {currentGroup?.desc && (
                  <p className={`mt-2 ${SUBTLE}`}>{currentGroup.desc}</p>
                )}

                <div className="mt-4 flex items-center justify-between gap-3">
                  <p className={LABEL}>
                    模型
                    <span className="ml-1.5 opacity-70">{models.length}</span>
                  </p>
                  <input
                    value={modelFilter}
                    onChange={(e) => setModelFilter(e.target.value)}
                    placeholder="搜索模型"
                    className="w-40 rounded-full border border-black/10 bg-transparent px-3 py-1 text-xs text-gray-900 outline-none placeholder:text-gray-400 focus:border-gray-400 dark:border-white/15 dark:text-gray-100"
                  />
                </div>
                <div className="mt-2 max-h-44 space-y-1 overflow-y-auto pr-1">
                  {models.length === 0 && <p className={SUBTLE}>没有匹配的模型</p>}
                  {models.map((m) => (
                    <button
                      key={m}
                      onClick={() => setModel(m)}
                      className={`flex w-full items-center justify-between rounded-xl px-3 py-2 text-left text-xs transition ${
                        m === model
                          ? "bg-gray-900/5 text-gray-900 dark:bg-white/10 dark:text-white"
                          : "text-gray-600 hover:bg-black/5 dark:text-gray-300 dark:hover:bg-white/5"
                      }`}
                    >
                      <span className="truncate font-mono">{m}</span>
                      <span className="ml-2 flex shrink-0 items-center gap-2">
                        <span className="tabular-nums text-[11px] text-gray-500 dark:text-gray-400">
                          {priceLabel(m)}
                        </span>
                        {m === model && <span>✓</span>}
                      </span>
                    </button>
                  ))}
                </div>

                {selectedPrice && (
                  <div className="mt-3 rounded-xl bg-black/[0.03] px-3 py-2.5 dark:bg-white/5">
                    <p className={LABEL}>
                      {model} 价格（已含 {groupRatio}x 分组倍率）
                    </p>
                    {selectedPrice.perRequest ? (
                      <p className="mt-1.5 text-xs text-gray-700 dark:text-gray-200">
                        按次计费 {fmtUSD(selectedPrice.input)} / 次
                      </p>
                    ) : (
                      <div className="mt-1.5 grid grid-cols-2 gap-x-4 gap-y-1 text-xs text-gray-700 dark:text-gray-200">
                        <span>
                          输入 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.input)}</span>
                        </span>
                        <span>
                          输出 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.output)}</span>
                        </span>
                        {selectedPrice.cache !== undefined && (
                          <span>
                            读缓存 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.cache)}</span>
                          </span>
                        )}
                        {selectedPrice.createCache !== undefined && (
                          <span>
                            写缓存 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.createCache)}</span>
                          </span>
                        )}
                      </div>
                    )}
                  </div>
                )}

                <button
                  onClick={enable}
                  disabled={provisioning || !group || !model}
                  className={`mt-4 w-full rounded-xl bg-gray-900 py-2.5 text-sm font-medium text-white transition hover:bg-gray-800 disabled:opacity-40 dark:bg-white dark:text-gray-900 dark:hover:bg-gray-200`}
                >
                  {provisioning ? "配置中…" : "一键启用到已安装应用"}
                </button>
                {notice && (
                  <p
                    className={`mt-2 text-xs ${
                      notice.ok ? "text-green-600 dark:text-green-400" : "text-orange-600 dark:text-orange-400"
                    }`}
                  >
                    {notice.text}
                  </p>
                )}
              </>
            )}
          </section>

          {/* 接入目标 */}
          <section className={CARD}>
            <div className="mb-3 flex items-center justify-between">
              <h2 className={TITLE}>接入应用</h2>
              <span className={SUBTLE}>
                已安装 {installed.length}/{targets.length}
              </span>
            </div>
            <div className="space-y-2">
              {targets.map((t) => {
                const compat = model ? baselineFor(t.id, model) : null;
                const result = results[t.id];
                return (
                  <div
                    key={t.id}
                    className={`rounded-xl px-3 py-2.5 ${
                      t.installed ? "bg-black/[0.03] dark:bg-white/5" : "bg-black/[0.02] opacity-60 dark:bg-white/[0.03]"
                    }`}
                  >
                    <div className="flex items-center justify-between gap-3">
                      <div className="flex min-w-0 items-center gap-2">
                        <span>{TARGET_ICONS[t.id] ?? "🔧"}</span>
                        <div className="min-w-0">
                          <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                            {t.name}
                          </p>
                          <p className={SUBTLE}>
                            {t.installed ? "已安装" : "未检测到安装"}
                            {compat && ` · ${COMPAT_LABEL[compat.level]}`}
                          </p>
                        </div>
                      </div>
                      <div className="flex shrink-0 items-center gap-2">
                        {compat && (
                          <span
                            title={compat.note}
                            className={`hidden rounded-full px-2 py-0.5 text-xs sm:inline-block ${COMPAT_STYLE[compat.level]}`}
                          >
                            {COMPAT_LABEL[compat.level]}
                          </span>
                        )}
                        <button
                          onClick={() => applyOne(t.id)}
                          disabled={!t.installed || applying !== null}
                          className={GHOST_BTN}
                        >
                          {applying === t.id ? "…" : "单独配置"}
                        </button>
                      </div>
                    </div>
                    {result && (
                      <p
                        className={`mt-1.5 text-xs ${
                          result.ok ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"
                        }`}
                      >
                        {result.ok
                          ? result.changed && result.changed.length > 0
                            ? `✓ 已更新 ${result.changed.join("、")}`
                            : "✓ 配置已是最新"
                          : `✗ ${result.error}`}
                      </p>
                    )}
                  </div>
                );
              })}
              {targets.length === 0 && <p className={SUBTLE}>未找到支持的应用</p>}
            </div>
            <button onClick={() => navigate("/install-guide")} className={`mt-3 ${GHOST_BTN}`}>
              安装指引
            </button>
          </section>

          {/* 设备（折叠） */}
          <section className={CARD}>
            <button
              onClick={() => setDevicesOpen((v) => !v)}
              className="flex w-full items-center justify-between"
            >
              <h2 className={TITLE}>登录设备</h2>
              <span className={SUBTLE}>
                {devices.length} 台 {devicesOpen ? "▲" : "▼"}
              </span>
            </button>
            {devicesOpen && (
              <div className="mt-3 space-y-2">
                {devices.length === 0 && <p className={SUBTLE}>暂无设备记录</p>}
                {devices.map((d) => (
                  <div
                    key={d.id}
                    className="flex items-center justify-between gap-3 rounded-xl bg-black/[0.03] px-3 py-2 dark:bg-white/5"
                  >
                    <div className="min-w-0">
                      <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                        {d.device_name || d.device_id.slice(0, 8)}
                        {d.is_current && <span className="ml-2 opacity-60">当前</span>}
                      </p>
                      <p className={SUBTLE}>
                        {d.platform} · 最后活跃 {formatTime(d.accessed_time)}
                      </p>
                    </div>
                    {!d.is_current && (
                      <button
                        onClick={() => revokeDevice(d.id)}
                        disabled={revoking !== null}
                        className={GHOST_BTN}
                      >
                        {revoking === d.id ? "…" : "撤销"}
                      </button>
                    )}
                  </div>
                ))}
                {otherDevices > 0 && (
                  <button onClick={revokeOthers} disabled={revoking !== null} className={PRIMARY_BTN}>
                    {revoking === "others" ? "操作中…" : `踢出其他 ${otherDevices} 台`}
                  </button>
                )}
              </div>
            )}
          </section>

          {apiKey && (
            <p className={`text-center ${SUBTLE}`}>
              当前密钥 {apiKey.slice(0, 10)}
              {"•".repeat(6)} · 分组 {group}
            </p>
          )}
        </div>
      </main>
    </div>
  );
}
