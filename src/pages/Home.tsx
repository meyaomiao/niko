import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, refreshAuthMeta, saveAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type DeviceItem } from "../api/client";
import { useSession } from "../hooks/useSession";
import { useTheme } from "../hooks/useTheme";
import { baselineFor, COMPAT_LABEL, COMPAT_STYLE, NATIVE_VENDOR } from "../lib/compat";
import { buildPricingIndex, priceOf, fmtUSD } from "../lib/pricing";
import { vendorOfGroup, VENDORS, type Vendor } from "../lib/vendor";
import Logo from "../components/Logo";
import { BookOpenIcon, LogOutIcon, MoonIcon, SettingsIcon, SunIcon } from "../components/Icons";
import TargetAppIcon from "../components/TargetAppIcon";
import {
  balanceReducer,
  formatBalanceUSD,
  formatBalanceUpdatedAt,
  parseBalanceSnapshot,
  type BalanceSnapshot,
} from "../lib/balance";
import {
  acceptsResponse,
  beginRequest,
  initialRequestGuard,
  mountRequests,
  safeFailure,
  unmountRequests,
} from "../lib/codexSessions";
import {
  commonActiveGroup,
  normalizeActiveGroupStatuses,
  summarizeActiveGroups,
  type ActiveGroupStatus,
} from "../lib/activeGroup";
import {
  displayDeviceLabel,
  friendlyConnectivityDetail,
  friendlyDesktopError,
} from "../lib/copy";

const RELAY_BASE_URL = "https://momotoken.win/v1";
/// 记住上次配置的应用，多应用用户不必每次重选
const TARGET_STORAGE_KEY = "niko_last_target";
/// 应用选择里代表「全部已安装应用」的哨兵值
const ALL_TARGETS = "__all__";
/// 记住 Codex 是否用混用模式（有 ChatGPT 订阅时保留官方登录态）
const CODEX_MIXED_STORAGE_KEY = "niko_codex_mixed";

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
  "token 是模型计算文字量的单位。你发出的内容和 AI 回复的内容分别计价，页面上的价格按 100 万 token 计算。";

const CARD = "nk-card";
const LABEL = "nk-label";
const TITLE = "nk-title";
const SUBTLE = "nk-muted";
const INPUT = "nk-input py-1 text-xs";
const SELECT = "nk-select w-full";
const GHOST_BTN = "nk-btn-secondary";
const PRIMARY_BTN = "nk-btn-primary";

export default function Home() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const { handleSessionExpired } = useSession();
  const { theme, toggle } = useTheme();

  const [bootstrap, setBootstrap] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  const initialBalance = parseBalanceSnapshot(
    auth?.quota,
    auth?.quotaPerUnit,
    auth?.balanceUpdatedAt,
  );
  const [balance, dispatchBalance] = useReducer(balanceReducer, {
    snapshot: initialBalance,
    refreshing: false,
    error: "",
  });
  const balanceRequestRef = useRef<Promise<BootstrapData | null> | null>(null);
  const [group, setGroup] = useState(auth?.defaultGroup ?? "");
  const [model, setModel] = useState("");
  const [modelFilter, setModelFilter] = useState("");
  const [provisioning, setProvisioning] = useState(false);
  const [notice, setNotice] = useState<{ ok: boolean; text: string } | null>(null);

  const [targets, setTargets] = useState<TargetInfo[]>([]);
  const [targetId, setTargetId] = useState("");
  const [targetsLoading, setTargetsLoading] = useState(true);
  const [targetsError, setTargetsError] = useState("");
  const [results, setResults] = useState<Record<string, ApplyResult>>({});
  const [activeStatuses, setActiveStatuses] = useState<Record<string, ActiveGroupStatus>>({});
  const [detecting, setDetecting] = useState(false);
  const [detectedGroupApplied, setDetectedGroupApplied] = useState(false);
  const [detectNonce, setDetectNonce] = useState(0);
  const groupTouchedRef = useRef(false);
  const requestGuardRef = useRef(initialRequestGuard());

  useEffect(() => {
    requestGuardRef.current = mountRequests(requestGuardRef.current);
    return () => {
      requestGuardRef.current = unmountRequests(requestGuardRef.current);
    };
  }, []);
  // 用户一旦手动挑过分组，就不再按所选应用自动推荐
  const [groupTouched, setGroupTouched] = useState(false);
  // Codex 专属：有 ChatGPT 订阅的用户走混用模式，保留官方登录态
  const [codexMixed, setCodexMixed] = useState(
    () => localStorage.getItem(CODEX_MIXED_STORAGE_KEY) === "1"
  );

  // 连通性测试 / 恢复默认：都直读磁盘配置，只在有已配置目标时可用
  const [testing, setTesting] = useState(false);
  const [restoring, setRestoring] = useState(false);
  const [restarting, setRestarting] = useState(false);
  const [confirmRestore, setConfirmRestore] = useState(false);

  const [devices, setDevices] = useState<DeviceItem[]>([]);
  const [devicesOpen, setDevicesOpen] = useState(false);
  const [tokenTipOpen, setTokenTipOpen] = useState(false);
  const [revoking, setRevoking] = useState<number | "others" | null>(null);

  const persistBalance = useCallback((snapshot: BalanceSnapshot, groupName: string) => {
    refreshAuthMeta({
      quota: snapshot.quota,
      quotaPerUnit: snapshot.quotaPerUnit,
      balanceUpdatedAt: snapshot.updatedAt,
      defaultGroup: groupName,
    });
  }, []);

  const loadTargets = useCallback(async () => {
    setTargetsLoading(true);
    setTargetsError("");
    try {
      const list = await invoke<TargetInfo[]>("list_targets");
      setTargets(list);
      const installed = list.filter((t) => t.installed);
      const last = localStorage.getItem(TARGET_STORAGE_KEY);
      const pick =
        installed.find((t) => t.id === last)?.id ??
        (installed.length === 1 ? installed[0].id : installed[0]?.id ?? "");
      setTargetId(pick);
    } catch {
      setTargets([]);
      setTargetId("");
      setTargetsError("未能读取本机应用状态，请重新检查。");
    } finally {
      setTargetsLoading(false);
    }
  }, []);

  const refreshBalance = useCallback(async () => {
    if (!auth?.accessToken || balanceRequestRef.current) return balanceRequestRef.current;
    dispatchBalance({ type: "refresh-started" });
    const request = (async (): Promise<BootstrapData | null> => {
      let data: BootstrapData | null = null;
      try {
        const [bootstrapResult, statusResult] = await Promise.allSettled([
          api.bootstrap(auth.accessToken),
          api.status(),
        ]);
        if (bootstrapResult.status === "rejected") throw bootstrapResult.reason;

        data = bootstrapResult.value;
        setBootstrap(data);
        const quotaPerUnit =
          data.site.quota_per_unit ??
          (statusResult.status === "fulfilled" ? statusResult.value.quota_per_unit : undefined);
        const snapshot = parseBalanceSnapshot(data.user.quota, quotaPerUnit);
        if (!snapshot) throw new Error("余额单位暂时无法读取");

        dispatchBalance({ type: "refresh-succeeded", snapshot });
        persistBalance(snapshot, data.user.group);
      } catch {
        dispatchBalance({ type: "refresh-failed", error: "余额刷新失败，请稍后重试" });
      }
      return data;
    })().finally(() => {
      balanceRequestRef.current = null;
    });
    balanceRequestRef.current = request;
    return request;
  }, [auth?.accessToken, persistBalance]);

  useEffect(() => {
    if (!auth?.accessToken) {
      navigate("/login", { replace: true });
      return;
    }
    void refreshBalance()
      .then((data) => {
        if (!data) return;
        // 分组不在这里定：等选好应用后按应用推荐（见下方 effect）
        const groups = data.groups ?? [];
        const remembered = groups.find((g) => g.name === auth.defaultGroup);
        if (remembered && !groupTouchedRef.current) {
          setGroup(remembered.name);
          setModel(remembered.models[0] ?? "");
        }
      })
      .finally(() => setLoading(false));

    // 先选应用：只装了一个就直接选中，装了多个则沿用上次
    void loadTargets();
    api.listDevices(auth.accessToken).then(setDevices).catch(() => {});
  }, [loadTargets]); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    const refreshVisibleBalance = () => {
      if (document.visibilityState === "visible") void refreshBalance();
    };
    window.addEventListener("focus", refreshVisibleBalance);
    document.addEventListener("visibilitychange", refreshVisibleBalance);
    return () => {
      window.removeEventListener("focus", refreshVisibleBalance);
      document.removeEventListener("visibilitychange", refreshVisibleBalance);
    };
  }, [refreshBalance]);

  const groups: GroupOption[] = useMemo(() => bootstrap?.groups ?? [], [bootstrap]);
  const deviceLimit = bootstrap?.device_limit ?? 0;
  const installedTargets = useMemo(() => targets.filter((t) => t.installed), [targets]);
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

  const detectionTargetIds = useMemo(() => {
    if (targetId === ALL_TARGETS) return installedTargets.map((target) => target.id);
    return installedTargets.some((target) => target.id === targetId) ? [targetId] : [];
  }, [targetId, installedTargets]);

  useEffect(() => {
    if (!auth?.accessToken || detectionTargetIds.length === 0) {
      setDetecting(false);
      return;
    }
    const request = beginRequest(requestGuardRef.current, "detect");
    requestGuardRef.current = request.state;
    setDetecting(true);

    void invoke<unknown>("detect_active_groups", {
      availableGroups: groups.length > 0 ? groups.map((item) => item.name) : null,
    })
      .then((rawStatuses) => {
        if (!acceptsResponse(requestGuardRef.current, "detect", request.generation)) return;
        const map = normalizeActiveGroupStatuses(rawStatuses);
        setActiveStatuses(map);
        const detected = commonActiveGroup(map, detectionTargetIds);
        if (detected && !groupTouchedRef.current) {
          setGroup(detected);
          setModel(groups.find((item) => item.name === detected)?.models[0] ?? "");
          setDetectedGroupApplied(true);
        } else if (!groupTouchedRef.current) {
          setDetectedGroupApplied(false);
        }
      })
      .catch(() => {
        if (!acceptsResponse(requestGuardRef.current, "detect", request.generation)) return;
        setActiveStatuses({});
      })
      .finally(() => {
        if (acceptsResponse(requestGuardRef.current, "detect", request.generation)) {
          setDetecting(false);
        }
      });

    return () => {
      const invalidated = beginRequest(requestGuardRef.current, "detect");
      requestGuardRef.current = invalidated.state;
    };
  }, [auth?.accessToken, detectionTargetIds, groups, detectNonce]);

  const activeGroupView = summarizeActiveGroups(activeStatuses, detectionTargetIds, detecting);

  // 应用选定后自动落到推荐厂商的第一个分组；用户手动挑过分组后不再干预
  useEffect(() => {
    if (groupTouched || detectedGroupApplied || groups.length === 0) return;
    const preferred = groups.find((item) => item.name === auth?.defaultGroup) ?? vendorTabs[0]?.[1][0];
    if (preferred) {
      setGroup(preferred.name);
      setModel(preferred.models[0] ?? "");
    }
  }, [auth?.defaultGroup, detectedGroupApplied, groupTouched, groups, vendorTabs]);

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
    if (!p) return "价格暂不可用";
    if (p.perRequest) return `${fmtUSD(p.input)}/次`;
    return `${fmtUSD(p.input)} 发出 · ${fmtUSD(p.output)} 回复`;
  };

  const selectedPrice = priceOf(pricingIndex.get(model), groupRatio);

  const pickGroup = (name: string) => {
    groupTouchedRef.current = true;
    setGroup(name);
    setGroupTouched(true);
    setNotice(null);
    const g = groups.find((x) => x.name === name);
    setModel(g?.models[0] ?? "");
  };

  const pickTarget = (id: string) => {
    groupTouchedRef.current = false;
    setTargetId(id);
    setResults({});
    setNotice(null);
    setActiveStatuses({});
    setDetectedGroupApplied(false);
    const recommended = groups.find((item) => item.name === auth?.defaultGroup);
    setGroup(recommended?.name ?? "");
    setModel(recommended?.models[0] ?? "");
    setGroupTouched(false);
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
      saveAuth({ ...auth, apiKey: res.api_key });

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
          map[r.id] = { ok: r.ok, changed: r.changed, error: r.error ? friendlyDesktopError(r.error) : undefined };
        });
        setResults(map);
        const okCount = applied.filter((r) => r.ok).length;
        const errors = applied
          .filter((result) => !result.ok && result.error)
          .map((result) => friendlyDesktopError(result.error));
        setNotice(
          applied.length === 0
            ? { ok: false, text: "没有找到已安装的应用，请先安装 ChatGPT 或 Claude。" }
            : {
                ok: okCount === applied.length,
                text: errors.length > 0
                  ? `已为 ${okCount}/${applied.length} 个应用接入 ${model || group}；${errors.join("；")}`
                  : `已为 ${okCount}/${applied.length} 个应用接入 ${model || group}`,
              }
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
        setNotice({ ok: true, text: `已为 ${targetLabel} 接入 ${model || group}` });
      }
      localStorage.setItem(TARGET_STORAGE_KEY, targetId);
      setDetectNonce((value) => value + 1);
    } catch (e) {
      setNotice({ ok: false, text: friendlyDesktopError(e) });
    } finally {
      setProvisioning(false);
    }
  };

  // 应用到「全部」时，逐个已安装应用处理
  const actionTargetIds = () =>
    targetId === ALL_TARGETS ? installedTargets.map((t) => t.id) : targetId ? [targetId] : [];

  // 目标应用只在启动时读一次配置，所以改完必须重启才生效
  const restartTargets = async () => {
    const ids = actionTargetIds();
    if (ids.length === 0) return;
    setRestarting(true);
    setNotice(null);
    try {
      const lines: string[] = [];
      let okCount = 0;
      for (const id of ids) {
        const name = targets.find((t) => t.id === id)?.name ?? id;
        try {
          const detail = await invoke<{ status: string; message: string }>("restart_target", { targetId: id });
          okCount += 1;
          lines.push(`${name}：${detail.message}`);
        } catch (e) {
          lines.push(`${name}：${safeFailure(e).message}`);
        }
      }
      setNotice({ ok: okCount === ids.length, text: lines.join("；") });
    } finally {
      setRestarting(false);
    }
  };

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
          lines.push(`${name}：${r.ok ? r.detail : friendlyConnectivityDetail(r.detail)}`);
        } catch (e) {
          lines.push(`${name}：${friendlyConnectivityDetail(e)}`);
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
      setNotice({ ok: false, text: "将移除 Niko 的设置并恢复官方账号登录，再点一次确认。" });
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
      setDetectNonce((value) => value + 1);
      setNotice({
        ok: true,
        text: total === 0 ? "本来就是官方登录方式，无需改动" : "已恢复官方登录方式，重启应用后用官方账号登录",
      });
    } catch (e) {
      setNotice({ ok: false, text: friendlyDesktopError(e) });
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
      <div
        role="status"
        aria-live="polite"
        className="flex h-screen flex-col items-center justify-center gap-3"
      >
        <span className="nk-spinner" aria-hidden="true" />
        <span className={SUBTLE}>正在同步账户信息…</span>
      </div>
    );
  }

  const otherDevices = devices.filter((d) => !d.is_current).length;

  return (
    <div className="nk-shell">
      <header className="nk-header justify-between">
        <Logo size={24} />
        <div className="flex items-center gap-2">
          <button onClick={toggle} className="nk-btn-ghost px-2.5" aria-label="切换主题">
            {theme === "dark" ? <SunIcon /> : <MoonIcon />}
          </button>
          <button onClick={() => navigate("/settings")} className={GHOST_BTN}>
            <SettingsIcon />
            设置
          </button>
          <button onClick={() => navigate("/sessions")} className={GHOST_BTN}>
            <BookOpenIcon />
            会话
          </button>
          <button onClick={logout} className={GHOST_BTN}>
            <LogOutIcon />
            退出
          </button>
        </div>
      </header>

      <main className="flex-1 overflow-y-auto px-4 py-4 md:overflow-hidden md:px-5">
        {/* 双列：左侧账户与应用，右侧模型选择，避免宽窗口下大量留白 */}
        <div className="mx-auto grid min-h-full max-w-5xl grid-cols-1 gap-4 md:h-full md:min-h-0 md:grid-cols-[minmax(0,19rem)_minmax(0,1fr)]">
          <div className="flex min-h-0 flex-col gap-3 pr-0.5 md:overflow-y-auto">
            {/* 余额 */}
            <section className={CARD}>
              <div className="flex items-end justify-between">
                <div>
                  <p className={LABEL}>{auth?.username ?? "已登录"}</p>
                  <div className="mt-1 flex items-center gap-1.5">
                    <p className="text-2xl font-semibold text-gray-900 dark:text-white" aria-live="polite">
                      {formatBalanceUSD(balance.snapshot)}
                    </p>
                    <button
                      type="button"
                      onClick={() => void refreshBalance()}
                      disabled={balance.refreshing}
                      aria-label="刷新余额"
                      title="刷新余额"
                      className="nk-btn-ghost min-h-7 px-2"
                    >
                      <span
                        aria-hidden="true"
                        className={`text-base leading-none ${balance.refreshing ? "animate-spin motion-reduce:animate-none" : ""}`}
                      >
                        ↻
                      </span>
                    </button>
                  </div>
                  <p className={`mt-1 ${SUBTLE}`}>
                    可用余额
                    {balance.snapshot ? ` · ${formatBalanceUpdatedAt(balance.snapshot)}` : ""}
                  </p>
                  {balance.error && (
                    <p className="mt-1 text-[11px] text-[var(--nk-danger)]" role="status">
                      {balance.error}
                    </p>
                  )}
                </div>
                <div className="flex items-center gap-2">
                  <button onClick={() => navigate("/topup")} className={PRIMARY_BTN}>
                    充值
                  </button>
                  <button onClick={() => navigate("/usage")} className={GHOST_BTN}>
                    使用明细
                  </button>
                </div>
              </div>
            </section>

            {/* 接入应用（先选应用，再按应用推荐模型） */}
            <section className={CARD}>
              <div className="mb-3 flex items-center justify-between">
                <h2 className={TITLE}>接入应用</h2>
                <span className={SUBTLE}>
                  {targetsLoading ? "正在检查…" : targetsError ? "检查失败" : `已安装 ${installedTargets.length}/${targets.length}`}
                </span>
              </div>
              {targetsLoading ? (
                <div className="flex items-center gap-2" role="status">
                  <span className="nk-spinner" aria-hidden="true" />
                  <p className={SUBTLE}>正在检查本机应用，请稍候。</p>
                </div>
              ) : targetsError ? (
                <div>
                  <p className="nk-alert-danger">{targetsError}</p>
                  <button onClick={() => void loadTargets()} className={`mt-3 ${GHOST_BTN}`}>
                    重新检查
                  </button>
                </div>
              ) : installedTargets.length === 0 ? (
                <div>
                  <p className={SUBTLE}>
                    没有找到支持的应用。先安装 ChatGPT 桌面端或 Claude 桌面端，再回来接入。
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
                          className={`nk-row w-full text-left ${
                            active
                              ? "nk-row-selected"
                              : t.installed
                                ? ""
                                : "opacity-60"
                          }`}
                        >
                          <div className="flex items-center justify-between gap-3">
                            <div className="flex min-w-0 items-center gap-2">
                              <TargetAppIcon
                                targetId={t.id}
                                name={t.name}
                                icon={t.icon}
                              />
                              <div className="min-w-0">
                                <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                                  {t.name}
                                </p>
                                <p className={SUBTLE}>{t.installed ? "已安装" : "还没有安装"}</p>
                              </div>
                            </div>
                            {active && <span className="shrink-0 text-xs">✓</span>}
                          </div>
                          {result && (
                            /* 改动项是一串很长的配置路径，全列会把卡片撑爆，这里只给条数，明细放 title */
                            <p
                              title={result.ok ? "设置已更新" : undefined}
                              className={`mt-1.5 truncate text-xs ${
                                result.ok ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"
                              }`}
                            >
                              {result.ok
                                ? result.changed && result.changed.length > 0
                                  ? `✓ 已更新 ${result.changed.length} 项设置`
                                  : "✓ 设置已是最新"
                                : `✗ ${result.error}`}
                            </p>
                          )}
                        </button>
                        {/* Claude 的作用范围说明单独成子卡片：常驻在选项里会把卡片撑高，
                            且未选中时并不需要这条信息。 */}
                        {t.id === "claude-desktop" && active && t.installed && (
                          <div className="nk-inset mt-1.5 p-2">
                            <p className={SUBTLE}>
                              仅作用于内置 Claude Code 面板，桌面端普通对话仍用你的 Anthropic 账号
                            </p>
                          </div>
                        )}
                        {/* Codex 独有：有 ChatGPT 订阅时保留官方登录态，密钥走 provider 段 */}
                        {t.id === "codex" && active && t.installed && (
                          <div className="nk-inset mt-1.5 p-2">
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
                        className={`nk-row w-full text-left text-xs ${
                          targetId === ALL_TARGETS
                            ? "nk-row-selected"
                            : ""
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
                    <p className="nk-alert-warning">
                      已用 {devices.length} / {deviceLimit} 台，达到上限后新设备将无法登录，建议清理不用的设备。
                    </p>
                  )}
                  {devices.length === 0 && <p className={SUBTLE}>暂无设备记录</p>}
                  {devices.map((d) => (
                    <div
                      key={d.id}
                      className="nk-row flex items-center justify-between gap-3"
                    >
                      <div className="min-w-0">
                        <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">
                          {displayDeviceLabel(d.device_name, d.platform)}
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

          </div>

          {/* 右列不设滚动：滚动只交给内部的模型列表，避免出现嵌套双层滚动条 */}
          <div className="flex min-h-0 flex-col md:overflow-hidden">
            {/* 分组 + 模型选择（跟随所选应用推荐） */}
            {installedTargets.length > 0 && (
            <section className={`${CARD} flex min-h-[28rem] flex-1 flex-col md:min-h-0`}>
              <div className="mb-2.5 flex shrink-0 items-center justify-between">
                <h2 className={TITLE}>{targetLabel ? `为 ${targetLabel} 选择模型` : "选择模型"}</h2>
                {currentGroup && (
                  <span className={`relative flex items-center gap-1 ${SUBTLE}`}>
                    价格按 100 万 token 计算
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
                        className="absolute right-0 top-6 z-20 w-[min(18rem,calc(100vw-3rem))] rounded-xl border bg-white p-3 text-left text-[11px] font-normal leading-relaxed text-gray-600 shadow-lg [border-color:var(--nk-line)] dark:bg-gray-900 dark:text-gray-300"
                      >
                        {TOKEN_TIP}
                      </span>
                    )}
                  </span>
                )}
              </div>
              {targetId && (
                <p
                  role="status"
                  aria-live="polite"
                  className={`mb-2 shrink-0 text-xs ${
                    activeGroupView.kind === "active"
                      ? "text-green-600 dark:text-green-400"
                      : activeGroupView.kind === "changed"
                        ? "text-orange-600 dark:text-orange-400"
                        : "text-gray-500 dark:text-gray-400"
                  }`}
                >
                  {activeGroupView.text}
                </p>
              )}

              {groups.length === 0 ? (
                <p className={SUBTLE}>当前账号没有可用模型服务，请联系管理员开通。</p>
              ) : (
                <>
                  <div className="flex shrink-0 gap-1 overflow-x-auto border-b [border-color:var(--nk-line)]">
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
                      模型服务
                      <span className="ml-1.5 opacity-70">{vendorGroups.length}</span>
                    </p>
                    <div className="relative min-w-0 flex-1">
                      <select
                        value={group}
                        onChange={(e) => pickGroup(e.target.value)}
                        aria-label="选择模型服务"
                        className={SELECT}
                      >
                        {vendorGroups.map((g) => (
                          <option key={g.name} value={g.name} className="text-gray-900">
                            {g.name}{g.desc ? ` · ${g.desc}` : ""}
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
                  <div className="mt-2 min-h-0 flex-1 space-y-1 overflow-y-auto pr-1 max-md:max-h-80">
                    {models.length === 0 && (
                      <p className="nk-empty">
                        {modelFilter ? "没有匹配的模型，请换一个关键词。" : "当前模型服务没有可用模型。"}
                      </p>
                    )}
                    {models.map((m) => {
                      const compat = compatOf(m);
                      return (
                      <button
                        key={m}
                        onClick={() => setModel(m)}
                        className={`nk-row flex w-full items-center justify-between text-left text-xs ${
                          m === model
                            ? "nk-row-selected text-gray-900 dark:text-white"
                            : "text-gray-600 dark:text-gray-300"
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

                  {/* 价格明细常驻，避免选中模型后列表跳动。 */}
                  <div className="nk-inset mt-2.5 min-h-[3.25rem] shrink-0 px-3 py-2">
                    {selectedPrice ? (
                      <>
                        {selectedPrice.perRequest ? (
                          <p className="text-xs text-gray-700 dark:text-gray-200">
                            每次请求 <span className="tabular-nums font-medium">{fmtUSD(selectedPrice.input)}</span> / 次
                          </p>
                        ) : (
                          <div className="flex flex-wrap gap-x-4 gap-y-0.5 text-xs text-gray-700 dark:text-gray-200">
                            <span>
                              你发出的内容：<span className="tabular-nums font-medium">{fmtUSD(selectedPrice.input)}</span> / 100 万 token
                            </span>
                            <span>
                              AI 回复的内容：<span className="tabular-nums font-medium">{fmtUSD(selectedPrice.output)}</span> / 100 万 token
                            </span>
                            {selectedPrice.cache !== undefined && (
                              <span>
                                读取已保存内容：<span className="tabular-nums font-medium">{fmtUSD(selectedPrice.cache)}</span> / 100 万 token
                              </span>
                            )}
                            {selectedPrice.createCache !== undefined && (
                              <span>
                                保存内容：<span className="tabular-nums font-medium">{fmtUSD(selectedPrice.createCache)}</span> / 100 万 token
                              </span>
                            )}
                          </div>
                        )}
                        <p className={`mt-1 truncate ${SUBTLE}`}>
                          {selectedCompat?.note || "\u00a0"}
                        </p>
                      </>
                    ) : (
                      <p className={`${SUBTLE} leading-9`}>选择模型后显示价格和计费单位。</p>
                    )}
                  </div>

                  {/* 四个动作挤在一行：主操作占宽，其余三个短名等分，避免堆四行把卡片撑高 */}
                  <div className="mt-2.5 flex shrink-0 flex-wrap items-center gap-1.5">
                    <button
                      onClick={enable}
                      disabled={provisioning || !group || !model || !targetId}
                      title={targetLabel ? `接入到 ${targetLabel}` : "选择应用后接入"}
                      className="nk-btn-primary min-w-24 flex-1"
                    >
                      {provisioning ? "接入中…" : "接入到应用"}
                    </button>
                    <button
                      onClick={restartTargets}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title={targetLabel ? `启动 / 重启 ${targetLabel}，让刚接入的设置生效` : "选择应用后可重启"}
                      className={GHOST_BTN}
                    >
                      {restarting ? "重启中…" : "重启应用"}
                    </button>
                    <button
                      onClick={testConnectivity}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title="检查当前设置是否能正常使用"
                      className={GHOST_BTN}
                    >
                      {testing ? "检查中…" : "检查是否能正常使用"}
                    </button>
                    <button
                      onClick={restoreDefaults}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title="移除 Niko 的设置，恢复用官方账号登录"
                      className={`${GHOST_BTN} ${confirmRestore ? "border-orange-400 text-orange-600 dark:text-orange-400" : ""}`}
                    >
                      {restoring ? "恢复中…" : confirmRestore ? "再点确认" : "恢复到官方"}
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
