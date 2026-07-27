import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, saveAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type DeviceItem } from "../api/client";
import { useSession } from "../hooks/useSession";
import { useTheme } from "../hooks/useTheme";
import { baselineFor, COMPAT_LABEL, COMPAT_STYLE, NATIVE_VENDOR } from "../lib/compat";
import { buildPricingIndex, priceOf, fmtUSD } from "../lib/pricing";
import { vendorOfGroup, VENDORS, type Vendor } from "../lib/vendor";
import { BRAND } from "../lib/brand";
import Logo from "../components/Logo";

const RELAY_BASE_URL = "https://momotoken.win/v1";
/// 记住上次配置的应用，多应用用户不必每次重选
const TARGET_STORAGE_KEY = "momo_last_target";
/// 应用选择里代表「全部已安装应用」的哨兵值
const ALL_TARGETS = "__all__";
/// 记住 Codex 是否用混用模式（有 ChatGPT 订阅时保留官方登录态）
const CODEX_MIXED_STORAGE_KEY = "momo_codex_mixed";

interface TargetInfo {
  id: string;
  name: string;
  installed: boolean;
  /// 后端从本机已安装 App 提取的真实图标（data URI），取不到时为 null
  icon?: string | null;
}

interface ApplyResult {
  ok: boolean;
  changed?: string[];
  error?: string;
}

const TARGET_ICONS: Record<string, string> = {
  codex: "🖥️",
  "claude-desktop": "🖥️",
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

/** 小白用户不懂 token 单位，用字数举例说明 */
const TOKEN_TIP =
  "token 是模型计费的最小文本单位。中文约 1 个字 ≈ 1.5 token，英文约 1 个单词 ≈ 1.3 token。100 万 token 大致相当于 60~70 万汉字，约等于一本长篇小说的量。输入（你发的内容）和输出（模型回复）分别计价。";

const CARD = "rounded-2xl border border-black/5 bg-white p-4 shadow-sm dark:border-white/10 dark:bg-white/5";
const LABEL = "text-xs font-medium text-gray-500 dark:text-gray-400";
const TITLE = "text-sm font-semibold text-gray-900 dark:text-gray-100";
const SUBTLE = "text-xs text-gray-500 dark:text-gray-400";
const INPUT =
  "rounded-full border border-black/10 bg-transparent px-3 py-1 text-xs text-gray-900 outline-none placeholder:text-gray-400 focus:border-gray-400 dark:border-white/15 dark:text-gray-100";
const SELECT = `${INPUT} w-full appearance-none truncate pr-7`;
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
  const [targetId, setTargetId] = useState("");
  const [results, setResults] = useState<Record<string, ApplyResult>>({});
  // 用户一旦手动挑过分组，就不再按所选应用自动推荐
  const [groupTouched, setGroupTouched] = useState(false);
  // Codex 专属：有 ChatGPT 订阅的用户走混用模式，保留官方登录态
  const [codexMixed, setCodexMixed] = useState(
    () => localStorage.getItem(CODEX_MIXED_STORAGE_KEY) === "1"
  );

  // 连通性测试 / 恢复默认：都直读磁盘配置，只在有已配置目标时可用
  const [testing, setTesting] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [confirmRestore, setConfirmRestore] = useState(false);

  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [tokenTipOpen, setTokenTipOpen] = useState(false);
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
        // 分组不在这里定：等选好应用后按应用推荐（见下方 effect）
        const groups = data.groups ?? [];
        const remembered = groups.find((g) => g.name === auth.group);
        if (remembered) {
          setGroup(remembered.name);
          setModel(remembered.models[0] ?? "");
          setGroupTouched(true);
        }
      })
      .catch(() => {})
      .finally(() => setLoading(false));

    // 先选应用：只装了一个就直接选中，装了多个则沿用上次
    invoke<TargetInfo[]>("list_targets")
      .then((list) => {
        setTargets(list);
        const installed = list.filter((t) => t.installed);
        const last = localStorage.getItem(TARGET_STORAGE_KEY);
        const pick =
          installed.find((t) => t.id === last)?.id ??
          (installed.length === 1 ? installed[0].id : installed[0]?.id ?? "");
        setTargetId(pick);
      })
      .catch(() => {});
    api.listDevices(auth.accessToken).then(setDevices).catch(() => {});
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const groups: GroupOption[] = bootstrap?.groups ?? [];
  const deviceLimit = bootstrap?.device_limit ?? 0;
  const installedTargets = targets.filter((t) => t.installed);
  // 选「全部」时按第一个已装应用推荐，语义上等价于用户最常用的那个
  const recommendVendor: Vendor | null = useMemo(() => {
    const id = targetId === ALL_TARGETS ? installedTargets[0]?.id : targetId;
    const v = id ? NATIVE_VENDOR[id] : undefined;
    return (VENDORS as readonly string[]).includes(v ?? "") ? (v as Vendor) : null;
  }, [targetId, targets]); // eslint-disable-line react-hooks/exhaustive-deps

  // 按分组名前缀归类到三家上游，未匹配的统一放「其他」；只保留有分组的厂商作为页签。
  // 与所选应用原生匹配的厂商排在最前，让推荐路径成为默认路径。
  const vendorTabs = useMemo(() => {
    const buckets: Record<string, GroupOption[]> = { OpenAI: [], Anthropic: [], Google: [], 其他: [] };
    for (const g of groups) {
      buckets[vendorOfGroup(g.name)].push(g);
    }
    const tabs = Object.entries(buckets).filter(([, list]) => list.length > 0) as [Vendor, GroupOption[]][];
    return tabs.sort(([a], [b]) => Number(b === recommendVendor) - Number(a === recommendVendor));
  }, [groups, recommendVendor]);
  // 应用选定后自动落到推荐厂商的第一个分组；用户手动挑过分组后不再干预
  useEffect(() => {
    if (groupTouched || groups.length === 0) return;
    const preferred = vendorTabs[0]?.[1][0];
    if (preferred) {
      setGroup(preferred.name);
      setModel(preferred.models[0] ?? "");
    }
  }, [groupTouched, groups, vendorTabs]);

  const selectedTarget = targets.find((t) => t.id === targetId) ?? null;
  const targetLabel =
    targetId === ALL_TARGETS ? `${installedTargets.length} 个应用` : selectedTarget?.name ?? "";
  // 兼容等级按所选应用判断；选「全部」时以第一个已装应用为准
  const compatTargetId = targetId === ALL_TARGETS ? installedTargets[0]?.id ?? "" : targetId;
  const compatOf = (name: string) => (compatTargetId ? baselineFor(compatTargetId, name) : null);
  const selectedCompat = model ? compatOf(model) : null;

  const currentGroup = groups.find((g) => g.name === group);
  // 页签跟随当前分组所属厂商，切换页签时自动选中该厂商第一个分组
  const activeVendor: Vendor | null = currentGroup
    ? vendorOfGroup(currentGroup.name)
    : vendorTabs[0]?.[0] ?? null;
  const vendorGroups = vendorTabs.find(([v]) => v === activeVendor)?.[1] ?? [];
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
    setGroupTouched(true);
    setNotice(null);
    const g = groups.find((x) => x.name === name);
    setModel(g?.models[0] ?? "");
  };

  const pickTarget = (id: string) => {
    setTargetId(id);
    setResults({});
    setNotice(null);
  };

  const pickCodexMixed = (mixed: boolean) => {
    setCodexMixed(mixed);
    localStorage.setItem(CODEX_MIXED_STORAGE_KEY, mixed ? "1" : "0");
    setResults({});
    setNotice(null);
  };

  // 申领/切换当前分组的 Key，再写入所选应用（或全部已安装应用）
  const enable = async () => {
    if (!auth?.accessToken || !group || !targetId) return;
    setProvisioning(true);
    setNotice(null);
    setResults({});
    try {
      const res = await api.provision(auth.accessToken, group);
      setApiKey(res.api_key);
      saveAuth({ ...auth, apiKey: res.api_key, group });

      if (targetId === ALL_TARGETS) {
        const applied = await invoke<Array<{ id: string; ok: boolean; changed?: string[]; error?: string }>>(
          "apply_all_targets",
          {
            baseUrl: RELAY_BASE_URL,
            apiKey: res.api_key,
            modelGroup: group,
            model: model || null,
            codexMixed: codexMixed,
          }
        );
        const map: Record<string, ApplyResult> = {};
        applied.forEach((r) => {
          map[r.id] = { ok: r.ok, changed: r.changed, error: r.error };
        });
        setResults(map);
        const okCount = applied.filter((r) => r.ok).length;
        setNotice(
          applied.length === 0
            ? { ok: false, text: "未检测到已安装的应用，请先安装 ChatGPT 或 Claude" }
            : { ok: okCount > 0, text: `已为 ${okCount}/${applied.length} 个应用启用 ${model || group}` }
        );
      } else {
        const changed = await invoke<string[]>("apply_target", {
          req: {
            target_id: targetId,
            base_url: RELAY_BASE_URL,
            api_key: res.api_key,
            model_group: group || null,
            model: model || null,
            codex_mixed: codexMixed,
          },
        });
        setResults({ [targetId]: { ok: true, changed } });
        setNotice({ ok: true, text: `已为 ${targetLabel} 启用 ${model || group}` });
      }
      localStorage.setItem(TARGET_STORAGE_KEY, targetId);
    } catch (e) {
      setNotice({ ok: false, text: String(e instanceof Error ? e.message : e) });
    } finally {
      setProvisioning(false);
    }
  };

  // 应用到「全部」时，逐个已安装应用处理
  const actionTargetIds = () =>
    targetId === ALL_TARGETS ? installedTargets.map((t) => t.id) : targetId ? [targetId] : [];

  const testConnectivity = async () => {
    const ids = actionTargetIds();
    if (ids.length === 0) return;
    setTesting(true);
    setNotice(null);
    try {
      const lines: string[] = [];
      let okCount = 0;
      for (const id of ids) {
        const name = targets.find((t) => t.id === id)?.name ?? id;
        try {
          const r = await invoke<{ ok: boolean; detail: string }>("test_connectivity", { targetId: id });
          if (r.ok) okCount += 1;
          lines.push(`${name}：${r.detail}`);
        } catch (e) {
          lines.push(`${name}：${e instanceof Error ? e.message : String(e)}`);
        }
      }
      setNotice({ ok: okCount === ids.length, text: lines.join("；") });
    } finally {
      setTesting(false);
    }
  };

  const restoreDefaults = async () => {
    const ids = actionTargetIds();
    if (ids.length === 0) return;
    // 二次确认：这会移除中转配置，用户可能只是误点
    if (!confirmRestore) {
      setConfirmRestore(true);
      setNotice({ ok: false, text: "将移除 Piko 写入的中转配置，恢复用官方账号登录，再点一次确认" });
      window.setTimeout(() => setConfirmRestore(false), 5000);
      return;
    }
    setConfirmRestore(false);
    setRestoring(true);
    setNotice(null);
    try {
      let total = 0;
      for (const id of ids) {
        const changed = await invoke<string[]>("restore_target_defaults", { targetId: id });
        total += changed.length;
      }
      setResults({});
      setNotice({
        ok: true,
        text: total === 0 ? "本就是官方默认配置，无需改动" : `已恢复官方默认，重启应用后用官方账号登录`,
      });
    } catch (e) {
      setNotice({ ok: false, text: e instanceof Error ? e.message : String(e) });
    } finally {
      setRestoring(false);
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

  return (
    <div className="flex h-screen flex-col">
      <header className="flex items-center justify-between border-b border-black/5 px-6 py-3 dark:border-white/10">
        <span className={`flex items-center gap-2 ${TITLE}`}>
          <Logo size={20} />
          {BRAND.name}
        </span>
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

      <main className="flex-1 overflow-hidden px-5 py-4">
        {/* 双列：左侧账户与应用，右侧模型选择，避免宽窗口下大量留白 */}
        <div className="mx-auto grid h-full max-w-5xl grid-cols-1 gap-4 md:grid-cols-[minmax(0,19rem)_minmax(0,1fr)]">
          <div className="flex min-h-0 flex-col gap-3 overflow-y-auto pr-0.5">
            {/* 余额 */}
            <section className={CARD}>
              <div className="flex items-end justify-between">
                <div>
                  <p className={LABEL}>{auth?.username ?? "已登录"}</p>
                  <p className="mt-1 text-2xl font-semibold tracking-tight text-gray-900 dark:text-white">
                    ${quotaToUSD(quota)}
                  </p>
                  <p className={`mt-1 ${SUBTLE}`}>可用余额</p>
                </div>
                <div className="flex items-center gap-2">
                  <button onClick={() => navigate("/topup")} className={PRIMARY_BTN}>
                    充值
                  </button>
                  <button onClick={() => navigate("/usage")} className={GHOST_BTN}>
                    用量明细
                  </button>
                </div>
              </div>
            </section>

            {/* 接入应用（先选应用，再按应用推荐模型） */}
            <section className={CARD}>
              <div className="mb-3 flex items-center justify-between">
                <h2 className={TITLE}>接入应用</h2>
                <span className={SUBTLE}>
                  已安装 {installedTargets.length}/{targets.length}
                </span>
              </div>
              {installedTargets.length === 0 ? (
                <div>
                  <p className={SUBTLE}>
                    没检测到支持的应用。先安装 ChatGPT 桌面端或 Claude 桌面端，再回来一键接入。
                  </p>
                  <button onClick={() => navigate("/install-guide")} className={`mt-3 ${GHOST_BTN}`}>
                    安装指引
                  </button>
                </div>
              ) : (
                <>
                  <div className="space-y-2">
                    {targets.map((t) => {
                      const result = results[t.id];
                      const active = t.id === targetId;
                      return (
                        <div key={t.id}>
                        <button
                          onClick={() => pickTarget(t.id)}
                          disabled={!t.installed}
                          className={`w-full rounded-xl px-3 py-2.5 text-left transition ${
                            active
                              ? "bg-gray-900/5 ring-1 ring-gray-900/20 dark:bg-white/10 dark:ring-white/25"
                              : t.installed
                                ? "bg-black/[0.03] hover:bg-black/[0.06] dark:bg-white/5 dark:hover:bg-white/10"
                                : "bg-black/[0.02] opacity-60 dark:bg-white/[0.03]"
                          }`}
                        >
                          <div className="flex items-center justify-between gap-3">
                            <div className="flex min-w-0 items-center gap-2">
                              {t.icon ? (
                                <img
                                  src={t.icon}
                                  alt=""
                                  className="h-6 w-6 shrink-0 rounded-md"
                                />
                              ) : (
                                <span className="w-6 shrink-0 text-center">
                                  {TARGET_ICONS[t.id] ?? "🔧"}
                                </span>
                              )}
                              <div className="min-w-0">
                                <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                                  {t.name}
                                </p>
                                <p className={SUBTLE}>{t.installed ? "已安装" : "未检测到安装"}</p>
                              </div>
                            </div>
                            {active && <span className="shrink-0 text-xs">✓</span>}
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
                        </button>
                        {/* Claude 的作用范围说明单独成子卡片：常驻在选项里会把卡片撑高，
                            且未选中时并不需要这条信息。 */}
                        {t.id === "claude-desktop" && active && t.installed && (
                          <div className="mt-1.5 rounded-xl bg-black/[0.03] p-2 dark:bg-white/5">
                            <p className={SUBTLE}>
                              仅作用于内置 Claude Code 面板，桌面端普通对话仍用你的 Anthropic 账号
                            </p>
                          </div>
                        )}
                        {/* Codex 独有：有 ChatGPT 订阅时保留官方登录态，密钥走 provider 段 */}
                        {t.id === "codex" && active && t.installed && (
                          <div className="mt-1.5 rounded-xl bg-black/[0.03] p-2 dark:bg-white/5">
                            <div className="grid grid-cols-2 gap-1.5">
                              {[
                                { mixed: false, label: "我没有 ChatGPT 订阅" },
                                { mixed: true, label: "我有 ChatGPT 付费订阅" },
                              ].map((opt) => (
                                <button
                                  key={String(opt.mixed)}
                                  onClick={() => pickCodexMixed(opt.mixed)}
                                  className={`rounded-lg px-2.5 py-1.5 text-xs transition ${
                                    codexMixed === opt.mixed
                                      ? "bg-white font-medium text-gray-900 shadow-sm dark:bg-white/15 dark:text-gray-100"
                                      : "text-gray-500 hover:bg-black/[0.04] dark:text-gray-400 dark:hover:bg-white/10"
                                  }`}
                                >
                                  {opt.label}
                                </button>
                              ))}
                            </div>
                            <p className={`mt-1.5 ${SUBTLE}`}>
                              {codexMixed
                                ? "保留 ChatGPT 登录态，官方额度与账号功能照常，模型走 momo"
                                : "只用 momo 的额度，不需要 ChatGPT 账号"}
                            </p>
                          </div>
                        )}
                        </div>
                      );
                    })}
                    {installedTargets.length > 1 && (
                      <button
                        onClick={() => pickTarget(ALL_TARGETS)}
                        className={`w-full rounded-xl px-3 py-2.5 text-left text-xs transition ${
                          targetId === ALL_TARGETS
                            ? "bg-gray-900/5 ring-1 ring-gray-900/20 dark:bg-white/10 dark:ring-white/25"
                            : "bg-black/[0.03] hover:bg-black/[0.06] dark:bg-white/5 dark:hover:bg-white/10"
                        }`}
                      >
                        <span className="font-medium text-gray-900 dark:text-gray-100">全部已安装应用</span>
                        <span className="ml-1.5 opacity-60">{installedTargets.length}</span>
                      </button>
                    )}
                  </div>
                  <button onClick={() => navigate("/install-guide")} className={`mt-3 ${GHOST_BTN}`}>
                    安装指引
                  </button>
                </>
              )}
            </section>

            {/* 设备（折叠） */}
            <section className={CARD}>
              <button
                onClick={() => setDevicesOpen((v) => !v)}
                className="flex w-full items-center justify-between"
              >
                <h2 className={TITLE}>登录设备</h2>
                <span className={SUBTLE}>
                  {devices.length}
                  {deviceLimit > 0 ? ` / ${deviceLimit}` : ""} 台{" "}
                  {devicesOpen ? "▲" : "▼"}
                </span>
              </button>
              {devicesOpen && (
                <div className="mt-3 space-y-2">
                  {deviceLimit > 0 && devices.length >= deviceLimit - 1 && (
                    <p className="rounded-xl bg-amber-500/10 px-3 py-2 text-xs text-amber-700 dark:text-amber-300">
                      已用 {devices.length} / {deviceLimit} 台，达到上限后新设备将无法登录，建议清理不用的设备。
                    </p>
                  )}
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
              <p className={SUBTLE}>
                当前密钥 {apiKey.slice(0, 10)}
                {"•".repeat(6)} · 分组 {group}
              </p>
            )}
          </div>

          {/* 右列不设滚动：滚动只交给内部的模型列表，避免出现嵌套双层滚动条 */}
          <div className="flex min-h-0 flex-col overflow-hidden">
            {/* 分组 + 模型选择（跟随所选应用推荐） */}
            {installedTargets.length > 0 && (
            <section className={`${CARD} flex min-h-0 flex-1 flex-col`}>
              <div className="mb-2.5 flex shrink-0 items-center justify-between">
                <h2 className={TITLE}>{targetLabel ? `为 ${targetLabel} 选择模型` : "选择模型"}</h2>
                {currentGroup && (
                  <span className={`relative flex items-center gap-1 ${SUBTLE}`}>
                    每百万 token · 含 {groupRatio}x 倍率
                    <button
                      type="button"
                      aria-label="什么是 token"
                      aria-expanded={tokenTipOpen}
                      onClick={() => setTokenTipOpen((v) => !v)}
                      onMouseEnter={() => setTokenTipOpen(true)}
                      onMouseLeave={() => setTokenTipOpen(false)}
                      onBlur={() => setTokenTipOpen(false)}
                      className="inline-flex h-4 w-4 items-center justify-center rounded-full border border-black/15 text-[10px] leading-none text-gray-500 transition hover:border-black/35 hover:text-gray-800 dark:border-white/20 dark:text-gray-400 dark:hover:border-white/50 dark:hover:text-gray-100"
                    >
                      ?
                    </button>
                    {tokenTipOpen && (
                      <span
                        role="tooltip"
                        className="absolute right-0 top-6 z-20 w-72 rounded-xl border border-black/10 bg-white p-3 text-left text-[11px] font-normal leading-relaxed text-gray-600 shadow-lg dark:border-white/15 dark:bg-gray-900 dark:text-gray-300"
                      >
                        {TOKEN_TIP}
                      </span>
                    )}
                  </span>
                )}
              </div>

              {groups.length === 0 ? (
                <p className={SUBTLE}>当前账号没有可用分组，请联系管理员开通</p>
              ) : (
                <>
                  <div className="flex shrink-0 gap-1 border-b border-black/5 dark:border-white/10">
                    {vendorTabs.map(([vendor, list]) => (
                      <button
                        key={vendor}
                        onClick={() => pickGroup(list[0].name)}
                        className={`-mb-px border-b-2 px-3 py-2 text-xs transition ${
                          vendor === activeVendor
                            ? "border-gray-900 font-medium text-gray-900 dark:border-white dark:text-white"
                            : "border-transparent text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200"
                        }`}
                      >
                        {vendor}
                        <span className="ml-1.5 opacity-60">{list.length}</span>
                        {recommendVendor && vendor !== recommendVendor && (
                          <span className="ml-1.5 opacity-60">转换接入</span>
                        )}
                      </button>
                    ))}
                  </div>
                  {/* 分组用下拉而非 chip 平铺：分组多时换行会吃掉模型列表的高度，
                      下拉把「名称 + 倍率 + 说明」压进一行，列表高度也不再随分组数变化。 */}
                  <div className="mt-2.5 flex shrink-0 items-center gap-3">
                    <p className={`${LABEL} shrink-0`}>
                      分组
                      <span className="ml-1.5 opacity-70">{vendorGroups.length}</span>
                    </p>
                    <div className="relative min-w-0 flex-1">
                      <select
                        value={group}
                        onChange={(e) => pickGroup(e.target.value)}
                        aria-label="选择分组"
                        className={SELECT}
                      >
                        {vendorGroups.map((g) => (
                          <option key={g.name} value={g.name} className="text-gray-900">
                            {g.name} · {g.ratio}x{g.desc ? ` · ${g.desc}` : ""}
                          </option>
                        ))}
                      </select>
                      {/* appearance-none 去掉系统箭头后自己补一个，保持和输入框同一套视觉 */}
                      <span className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[10px] text-gray-400">
                        ▾
                      </span>
                    </div>
                  </div>

                  <div className="mt-2.5 flex shrink-0 items-center justify-between gap-3">
                    <p className={LABEL}>
                      模型
                      <span className="ml-1.5 opacity-70">{models.length}</span>
                    </p>
                    <input
                      value={modelFilter}
                      onChange={(e) => setModelFilter(e.target.value)}
                      placeholder="搜索模型"
                      className={`w-40 ${INPUT}`}
                    />
                  </div>
                  <div className="mt-2 min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
                    {models.length === 0 && <p className={SUBTLE}>没有匹配的模型</p>}
                    {models.map((m) => {
                      const compat = compatOf(m);
                      return (
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
                          {compat && (
                            <span
                              title={compat.note}
                              className={`rounded-full px-2 py-0.5 text-[11px] ${COMPAT_STYLE[compat.level]}`}
                            >
                              {COMPAT_LABEL[compat.level]}
                            </span>
                          )}
                          <span className="tabular-nums text-[11px] text-gray-500 dark:text-gray-400">
                            {priceLabel(m)}
                          </span>
                          {m === model && <span>✓</span>}
                        </span>
                      </button>
                      );
                    })}
                  </div>

                  {/* 价格明细常驻：条件渲染会在选中模型时挤压上方列表，导致刚点的行跳走。
                      倍率已在卡片标题右侧说明，这里不再重复模型名与倍率。 */}
                  <div className="mt-2.5 min-h-[3.25rem] shrink-0 rounded-xl bg-black/[0.03] px-3 py-2 dark:bg-white/5">
                    {selectedPrice ? (
                      <>
                        {selectedPrice.perRequest ? (
                          <p className="text-xs text-gray-700 dark:text-gray-200">
                            按次计费 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.input)}</span> / 次
                          </p>
                        ) : (
                          <div className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-gray-700 dark:text-gray-200">
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
                        <p className={`mt-1 truncate ${SUBTLE}`}>
                          {selectedCompat?.note || "\u00a0"}
                        </p>
                      </>
                    ) : (
                      <p className={`${SUBTLE} leading-9`}>选择模型后显示价格</p>
                    )}
                  </div>

                  <button
                    onClick={enable}
                    disabled={provisioning || !group || !model || !targetId}
                    className={`mt-2.5 w-full shrink-0 rounded-xl bg-gray-900 py-2.5 text-sm font-medium text-white transition hover:bg-gray-800 disabled:opacity-40 dark:bg-white dark:text-gray-900 dark:hover:bg-gray-200`}
                  >
                    {provisioning ? "配置中…" : targetLabel ? `启用到 ${targetLabel}` : "选择应用后启用"}
                  </button>
                  {/* 配置后自检与回退：直读磁盘配置，不改内存状态 */}
                  <div className="mt-1.5 flex shrink-0 gap-1.5">
                    <button
                      onClick={testConnectivity}
                      disabled={provisioning || testing || restoring || !targetId}
                      className={`${GHOST_BTN} flex-1`}
                    >
                      {testing ? "检测中…" : "检测连通性"}
                    </button>
                    <button
                      onClick={restoreDefaults}
                      disabled={provisioning || testing || restoring || !targetId}
                      className={`${GHOST_BTN} flex-1 ${confirmRestore ? "border-orange-400 text-orange-600 dark:text-orange-400" : ""}`}
                    >
                      {restoring ? "恢复中…" : confirmRestore ? "再点确认恢复" : "恢复官方默认"}
                    </button>
                  </div>
                  {/* 常驻一行：结果提示出现时不再压缩上方列表 */}
                  <p
                    title={notice?.text || undefined}
                    className={`mt-1.5 shrink-0 truncate text-xs ${
                      notice
                        ? notice.ok
                          ? "text-green-600 dark:text-green-400"
                          : "text-orange-600 dark:text-orange-400"
                        : "text-transparent"
                    }`}
                  >
                    {notice?.text || "\u00a0"}
                  </p>
                </>
              )}
            </section>
            )}

          </div>
        </div>
      </main>
    </div>
  );
}
