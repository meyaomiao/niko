import { useCallback, useEffect, useMemo, useReducer, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, refreshAuthMeta, saveAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type DeviceItem, type PricingMeta, type UsageSummary } from "../api/client";
import { useSession } from "../hooks/useSession";
import { useTheme } from "../hooks/useTheme";
import { baselineFor, COMPAT_LABEL, COMPAT_STYLE, NATIVE_VENDOR } from "../lib/compat";
import { buildVendorModelTabs, type VendorModelChoice } from "../lib/modelSelection";
import { buildPricingIndex, priceOf, fmtUSD } from "../lib/pricing";
import { computeTags, vendorPriceLevels, vendorUsageRanks } from "../lib/modelTags";
import { buildVendorIndex } from "../lib/vendorCatalog";
import { buildGroupCatalog, groupsForModel } from "../lib/groupCatalog";
import { vendorOfGroup, vendorOfModel, VENDORS, type Vendor } from "../lib/vendor";
import Logo from "../components/Logo";
import { VendorIcon } from "../components/VendorIcon";
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
  normalizeEffectiveSelectionStatuses,
  summarizeEffectiveSelections,
  type ActiveSelection,
  type EffectiveSelectionStatus,
} from "../lib/activeGroup";
import {
  displayDeviceLabel,
  friendlyConnectivityDetail,
  friendlyDesktopError,
} from "../lib/copy";
import { loadDraftSelection, saveDraftSelection } from "../lib/selectionState";

const RELAY_BASE_URL = "https://momotoken.win/v1";
const DESKTOP_APPLY_TIMEOUT_MS = 30_000;

function withDesktopApplyTimeout<T>(promise: Promise<T>): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const timeoutId = setTimeout(() => {
      reject(new Error("桌面应用配置写入超时，请重试；会话同步需要在会话管理页面单独执行"));
    }, DESKTOP_APPLY_TIMEOUT_MS);
    promise.then(resolve, reject).finally(() => clearTimeout(timeoutId));
  });
}
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
  warning?: string;
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
const CARD_TIGHT = "nk-card-tight";
const LABEL = "nk-label";
const TITLE = "nk-title";
const SUBTLE = "nk-muted";
const GHOST_BTN = "nk-btn-secondary";
const PRIMARY_BTN = "nk-btn-primary";

export default function Home() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const { handleSessionExpired } = useSession();
  const { theme, toggle } = useTheme();

  const [bootstrap, setBootstrap] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  // 用户用量聚合（厂家内排名用），驱动「常用 / 性价比」标签
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  // 服务端厂商目录 + 分组说明（公开接口）
  const [pricingMeta, setPricingMeta] = useState<PricingMeta | null>(null);
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
  // 搜索默认收起为一个图标，避免常驻输入框挤占页签宽度
  const [searchOpen, setSearchOpen] = useState(false);
  const [provisioning, setProvisioning] = useState(false);
  const [notice, setNotice] = useState<{ ok: boolean; text: string } | null>(null);

  const [targets, setTargets] = useState<TargetInfo[]>([]);
  const [targetId, setTargetId] = useState("");
  const [targetsLoading, setTargetsLoading] = useState(true);
  const [targetsError, setTargetsError] = useState("");
  const [results, setResults] = useState<Record<string, ApplyResult>>({});
  const [activeStatuses, setActiveStatuses] = useState<Record<string, EffectiveSelectionStatus>>({});
  const [detecting, setDetecting] = useState(false);
  const [detectNonce, setDetectNonce] = useState(0);
  const [draftSource, setDraftSource] = useState<"recommendation" | "saved" | "manual">("recommendation");
  const groupTouchedRef = useRef(false);
  const requestGuardRef = useRef(initialRequestGuard());
  // 分组测速：group 名 → TTFT 中位数（ms）|"loading"（进行中）|null（失败）
  const [benchmarks, setBenchmarks] = useState<Record<string, number | "loading" | null>>({});
  const [benchmarkRunning, setBenchmarkRunning] = useState(false);

  useEffect(() => {
    requestGuardRef.current = mountRequests(requestGuardRef.current);
    return () => {
      requestGuardRef.current = unmountRequests(requestGuardRef.current);
    };
  }, []);
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
      })
      .finally(() => setLoading(false));

    // 常用/性价比标签：取当前账号用量聚合，按厂家内排名判定；失败静默
    api
      .usageSummary(auth.accessToken)
      .then((s) => setUsage(s))
      .catch(() => undefined);

    // 厂商目录 + 全部分组说明：公开接口，失败时回退到本地启发式
    api
      .pricingMeta()
      .then((meta) => setPricingMeta(meta))
      .catch(() => undefined);

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

  // 服务端厂商目录：模型 → 厂家（新版 pricing 带 vendor_id，公开 /api/pricing 给名称）
  const vendorIndex = useMemo(
    () => buildVendorIndex(bootstrap?.pricing, pricingMeta?.vendors),
    [bootstrap?.pricing, pricingMeta?.vendors]
  );

  // 分组目录：模型 → 令牌分组（pricing.enable_groups）+ 中文说明/倍率
  const groupCatalog = useMemo(
    () =>
      buildGroupCatalog({
        accountGroups: groups,
        usableGroup: pricingMeta?.usableGroup,
        groupRatio: pricingMeta?.groupRatio,
      }),
    [groups, pricingMeta]
  );

  // 按供应商汇总模型，再由模型反查可用分组。新服务端以 model_order 给出完整显示顺序；
  // 元数据只作为旧响应的排序辅助，不能覆盖服务端顺序。
  const vendorTabs = useMemo(() => {
    const modelMetadata = { ...(bootstrap?.model_metadata ?? {}) };
    for (const item of bootstrap?.pricing ?? []) {
      if (item.release_date) {
        modelMetadata[item.model_name] = {
          ...modelMetadata[item.model_name],
          name: modelMetadata[item.model_name]?.name ?? item.model_name,
          release_date: modelMetadata[item.model_name]?.release_date ?? item.release_date,
          release_source: modelMetadata[item.model_name]?.release_source ?? item.release_source,
        };
      }
    }
    const tabs = buildVendorModelTabs({
      groups,
      models: bootstrap?.models,
      modelMetadata,
      modelOrder: bootstrap?.model_order,
      recommendVendor,
      // 服务端厂商目录优先，缺失的模型按模型名回退本地启发式
      vendorOf: (model) => vendorIndex.get(model)?.name ?? vendorOfModel(model),
    });
    // 页签顺序按模型数量降序，数量相同按名称；原生厂商不置顶，改为页签小徽标
    return [...tabs].sort(
      (a, b) => b.models.length - a.models.length || a.vendor.localeCompare(b.vendor)
    );
  }, [groups, bootstrap?.models, bootstrap?.model_metadata, bootstrap?.model_order, bootstrap?.pricing, recommendVendor, vendorIndex]);

  const modelByGroup = useMemo(() => {
    const index = new Map<string, string>();
    for (const tab of vendorTabs) {
      for (const choice of tab.models) {
        for (const option of choice.groups) {
          if (!index.has(option.name)) index.set(option.name, choice.name);
        }
      }
    }
    return index;
  }, [vendorTabs]);

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
    setActiveStatuses({});

    void invoke<unknown>("detect_effective_selections", {
      availableGroups: groups.length > 0 ? groups.map((item) => item.name) : null,
    })
      .then((rawStatuses) => {
        if (!acceptsResponse(requestGuardRef.current, "detect", request.generation)) return;
        const map = normalizeEffectiveSelectionStatuses(rawStatuses);
        setActiveStatuses(map);
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

  const activeGroupView = summarizeEffectiveSelections(activeStatuses, detectionTargetIds, detecting);

  // 应用选定后自动落到推荐厂商的第一个分组；用户手动挑过分组后不再干预
  useEffect(() => {
    if (groups.length === 0) return;
    if (groupTouchedRef.current) return;
    const storageTargetId = targetId === ALL_TARGETS ? installedTargets[0]?.id : targetId;
    const saved = storageTargetId ? loadDraftSelection(storageTargetId) : null;
    if (
      saved
      && groups.some((item) => item.name === saved.group)
      && saved.provider === vendorOfGroup(saved.group)
      && vendorTabs.some((tab) => tab.models.some((choice) => choice.name === saved.model
        && choice.groups.some((item) => item.name === saved.group)))
    ) {
      groupTouchedRef.current = true;
      setGroup(saved.group);
      setModel(saved.model);
      setDraftSource("saved");
      return;
    }
    const preferredGroup = groups.find((item) => item.name === auth?.defaultGroup);
    const preferredModel = preferredGroup
      ? modelByGroup.get(preferredGroup.name)
      : vendorTabs[0]?.models[0]?.name;
    const fallbackGroup = preferredGroup?.name ?? vendorTabs[0]?.models[0]?.groups[0]?.name;
    if (preferredModel && fallbackGroup) {
      setModel(preferredModel);
      setGroup(fallbackGroup);
      setDraftSource("recommendation");
    }
  }, [auth?.defaultGroup, groups, installedTargets, modelByGroup, targetId, vendorTabs]);

  const recommendedSelection = useMemo(() => {
    const recommendedGroup = groups.find((item) => item.name === auth?.defaultGroup);
    const recommendedModel = recommendedGroup
      ? modelByGroup.get(recommendedGroup.name)
      : vendorTabs[0]?.models[0]?.name;
    const fallbackGroup = recommendedGroup?.name ?? vendorTabs[0]?.models[0]?.groups[0]?.name;
    return recommendedModel && fallbackGroup
      ? {
          provider: vendorOfGroup(fallbackGroup),
          model: recommendedModel,
          group: fallbackGroup,
        }
      : null;
  }, [auth?.defaultGroup, groups, modelByGroup, vendorTabs]);

  const draftSelection: ActiveSelection | null = draftSource !== "recommendation" && model && group
    ? { provider: vendorOfGroup(group), model, group }
    : null;

  const activeSelectionRows = useMemo(() => detectionTargetIds.flatMap((id) => {
    const status = activeStatuses[id];
    if (status?.status !== "active" || !status.group || !status.model) return [];
    return [{
      target: targets.find((item) => item.id === id)?.name ?? "应用",
      provider: vendorOfGroup(status.group),
      model: status.model,
      group: status.group,
    }];
  }), [activeStatuses, detectionTargetIds, targets]);

  useEffect(() => {
    if (!group || !model || draftSource === "recommendation") return;
    const storageTargetId = targetId === ALL_TARGETS ? installedTargets[0]?.id : targetId;
    if (!storageTargetId) return;
    saveDraftSelection(storageTargetId, {
      provider: vendorOfGroup(group),
      model,
      group,
    });
  }, [draftSource, group, installedTargets, model, targetId]);

  const selectedTarget = targets.find((t) => t.id === targetId) ?? null;
  const targetLabel =
    targetId === ALL_TARGETS ? `${installedTargets.length} 个应用` : selectedTarget?.name ?? "";
  // 兼容等级按所选应用判断；选「全部」时以第一个已装应用为准
  const compatTargetId = targetId === ALL_TARGETS ? installedTargets[0]?.id ?? "" : targetId;
  const compatOf = (name: string) => (compatTargetId ? baselineFor(compatTargetId, name) : null);

  const currentGroup = groups.find((g) => g.name === group);
  // 页签跟随当前模型所属厂家；没有模型时按当前分组所属厂家推断，最后兜底第一个页签
  const activeVendor: string | null = (() => {
    if (model) {
      const tab = vendorTabs.find((item) => item.models.some((choice) => choice.name === model));
      if (tab) return tab.vendor;
    }
    if (currentGroup) {
      const guess = vendorOfGroup(currentGroup.name);
      const tab = vendorTabs.find((item) => item.vendor === guess);
      if (tab) return tab.vendor;
    }
    return vendorTabs[0]?.vendor ?? null;
  })();
  const activeVendorTab = vendorTabs.find((tab) => tab.vendor === activeVendor) ?? vendorTabs[0] ?? null;
  const vendorModels = activeVendorTab?.models ?? [];
  // 模型的令牌分组（不是账号可用分组）：来自服务端 pricing.enable_groups
  const modelGroups = useMemo(
    () => (model ? groupsForModel(groupCatalog, bootstrap?.pricing, model, groups) : []),
    [groupCatalog, bootstrap?.pricing, model, groups]
  );
  // 服务端是否真的下发了该模型的令牌分组（enable_groups）
  const tokenGroupNames = useMemo(
    () => bootstrap?.pricing?.find((item) => item.model_name === model)?.enable_groups ?? [],
    [bootstrap?.pricing, model]
  );
  const hasTokenGroups = tokenGroupNames.length > 0;
  const pricingIndex = useMemo(() => buildPricingIndex(bootstrap?.pricing), [bootstrap]);
  const models = useMemo(() => {
    const kw = modelFilter.trim().toLowerCase();
    const list = kw ? vendorModels.filter((m) => m.name.toLowerCase().includes(kw)) : vendorModels;
    // 默认按发布时间从新到旧；没有发布时间的排最后（按名称稳定兜底）
    return [...list].sort((a, b) => {
      const ta = a.releaseTime ?? Number.NEGATIVE_INFINITY;
      const tb = b.releaseTime ?? Number.NEGATIVE_INFINITY;
      if (ta !== tb) return tb - ta;
      return a.name.localeCompare(b.name);
    });
  }, [vendorModels, modelFilter]);

  useEffect(() => {
    if (!activeVendorTab || groups.length === 0) return;
    const selected = activeVendorTab.models.find((choice) => choice.name === model) ?? activeVendorTab.models[0];
    if (!selected) return;
    if (selected.name !== model) {
      setModel(selected.name);
      return;
    }
    // 分组必须是账号可用的：模型列出的令牌分组里挑第一个可用项
    const usable = modelGroups.filter((g) => g.usable);
    if (usable.length === 0) return;
    if (!usable.some((g) => g.name === group)) setGroup(usable[0].name);
  }, [activeVendorTab, groups.length, group, model, modelGroups]);

  const groupPriceLabel = (name: string, ratio: number) => {
    const p = priceOf(pricingIndex.get(name), ratio);
    if (!p) return "价格暂不可用";
    if (p.perRequest) return `${fmtUSD(p.input)}/次`;
    return `${fmtUSD(p.input)} / ${fmtUSD(p.output)}`;
  };

  // 模型卡标签：全部按厂家内比较
  // 刚上新（发布 14 天内）/ 常用（厂家内用量前 3）/ 性价比（厂家内用量前 5 且价格分位 < 1/5）
  const tagLevels = useMemo(() => {
    const names = vendorTabs.flatMap((tab) => tab.models.map((choice) => choice.name));
    const ratio = currentGroup?.ratio ?? 0;
    return vendorPriceLevels(names, (name) => {
      const p = priceOf(pricingIndex.get(name), ratio);
      return p && !p.perRequest ? p.input : undefined;
    });
  }, [vendorTabs, pricingIndex, currentGroup?.ratio]);

  const tagRanks = useMemo(() => vendorUsageRanks(usage), [usage]);

  const modelTags = (name: string) => {
    const m = bootstrap?.model_metadata?.[name];
    const release =
      m?.release_date ?? m?.released_at ?? m?.official_release_date ?? m?.version_date;
    return computeTags({
      name,
      release,
      rank: tagRanks.get(name),
      level: tagLevels.get(name),
    });
  };

  /** 模型卡片右侧的发布日期（YYYY-MM-DD，缺失则留空） */
  const releaseLabelOf = (name: string) => {
    const m = bootstrap?.model_metadata?.[name];
    return m?.release_date ?? m?.released_at ?? m?.official_release_date ?? m?.version_date ?? "";
  };

  const pickGroup = (name: string) => {
    groupTouchedRef.current = true;
    setGroup(name);
    setDraftSource("manual");
    setNotice(null);
  };

  // 分组测速：对当前模型的每个分组发 3 次最小流式请求，取 TTFT 中位数。
  // 当前保存密钥所属分组直接复用密钥；其余分组临时申请（不覆盖已保存密钥）。
  const runBenchmarks = async () => {
    if (!auth?.accessToken || !model || benchmarkRunning) return;
    setBenchmarkRunning(true);
    setBenchmarks(Object.fromEntries(modelGroups.map((g) => [g.name, "loading" as const])));
    for (const g of modelGroups) {
      try {
        let apiKey = auth.apiKey && auth.apiKeyGroup === g.name ? auth.apiKey : null;
        if (!apiKey) {
          const res = await api.provision(auth.accessToken, g.name);
          apiKey = res.api_key;
        }
        const result = await invoke<{ median_ttft_ms: number | null }>("benchmark_group", {
          base_url: RELAY_BASE_URL,
          api_key: apiKey,
          model,
          samples: 3,
        });
        if (result.median_ttft_ms != null) {
          // 落一份本地缓存，供「模型与价格」页展示实测延迟
          try {
            const cache = JSON.parse(localStorage.getItem("niko_model_bench") ?? "{}") as Record<string, unknown>;
            cache[model] = { median: result.median_ttft_ms, samples: 3, at: Date.now() };
            localStorage.setItem("niko_model_bench", JSON.stringify(cache));
          } catch { /* 缓存失败不影响测速 */ }
        }
        setBenchmarks((prev) => ({ ...prev, [g.name]: result.median_ttft_ms }));
      } catch {
        setBenchmarks((prev) => ({ ...prev, [g.name]: null }));
      }
    }
    setBenchmarkRunning(false);
  };

  const pickModel = (choice: VendorModelChoice) => {
    groupTouchedRef.current = true;
    setModel(choice.name);
    if (!choice.groups.some((g) => g.name === group)) {
      setGroup(choice.groups[0]?.name ?? "");
    }
    setDraftSource("manual");
    setNotice(null);
  };

  const pickVendor = (tab: NonNullable<typeof activeVendorTab>) => {
    const choice = tab.models[0];
    groupTouchedRef.current = true;
    setModel(choice?.name ?? "");
    setGroup(choice?.groups[0]?.name ?? "");
    setDraftSource("manual");
    setNotice(null);
  };

  const pickTarget = (id: string) => {
    groupTouchedRef.current = false;
    setTargetId(id);
    setResults({});
    setNotice(null);
    setActiveStatuses({});
    setGroup("");
    setModel("");
    setDraftSource("recommendation");
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
      setNotice({ ok: true, text: "正在申请模型密钥…" });
      let apiKey = auth.apiKey;
      let reusedSavedApiKey = false;
      try {
        const res = await api.provision(auth.accessToken, group);
        apiKey = res.api_key;
        saveAuth({ ...auth, apiKey, apiKeyGroup: res.group });
      } catch (error) {
        if (!apiKey || auth.apiKeyGroup !== group) throw error;
        reusedSavedApiKey = true;
        setNotice({ ok: true, text: "模型服务暂时无法刷新密钥，使用当前已保存密钥写入配置…" });
      }
      if (!apiKey) throw new Error("没有可用的模型密钥，请待模型服务恢复后重试");
      if (!reusedSavedApiKey) setNotice({ ok: true, text: "模型密钥已取得，正在写入桌面配置…" });

      if (targetId === ALL_TARGETS) {
        const applied = await withDesktopApplyTimeout(invoke<
          Array<{ id: string; ok: boolean; changed?: string[]; error?: string; warning?: string }>
        >(
          "apply_all_targets",
          {
            baseUrl: RELAY_BASE_URL,
            apiKey,
            modelGroup: group,
            model: model || null,
            codexMixed: codexMixed,
          }
        ));
        const map: Record<string, ApplyResult> = {};
        applied.forEach((r) => {
          map[r.id] = {
            ok: r.ok,
            changed: r.changed,
            error: r.error ? friendlyDesktopError(r.error) : undefined,
            warning: r.warning,
          };
        });
        setResults(map);
        const okCount = applied.filter((r) => r.ok).length;
        const errors = applied
          .filter((result) => !result.ok && result.error)
          .map((result) => friendlyDesktopError(result.error));
        const warnings = applied
          .filter((result) => result.ok && result.warning)
          .map((result) => result.warning);
        setNotice(
          applied.length === 0
            ? { ok: false, text: "没有找到已安装的应用，请先安装 ChatGPT 或 Claude。" }
            : {
                ok: okCount === applied.length,
                text: errors.length > 0
                  ? `已为 ${okCount}/${applied.length} 个应用接入 ${model || group}；${errors.join("；")}`
                  : warnings.length > 0
                    ? `已为 ${okCount}/${applied.length} 个应用接入 ${model || group}；${warnings.join("；")}`
                    : `已为 ${okCount}/${applied.length} 个应用接入 ${model || group}`,
              }
        );
      } else {
        const applied = await withDesktopApplyTimeout(invoke<{ changed: string[]; warning?: string }>("apply_target", {
          req: {
            target_id: targetId,
            base_url: RELAY_BASE_URL,
            api_key: apiKey,
            model_group: group || null,
            model: model || null,
            codex_mixed: codexMixed,
          },
        }));
        setResults({ [targetId]: { ok: true, changed: applied.changed, warning: applied.warning } });
        setNotice({
          ok: true,
          text: applied.warning
            ? `已为 ${targetLabel} 接入 ${model || group}；${applied.warning}`
            : `已为 ${targetLabel} 接入 ${model || group}`,
        });
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
    try {
      await invoke("clear_remembered_login");
    } catch {
      /* ignore */
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
          <button onClick={() => navigate("/settings")} className={GHOST_BTN} aria-label="设置" title="设置">
            <SettingsIcon />
            <span className="hidden sm:inline">设置</span>
          </button>
          <button onClick={() => navigate("/sessions")} className={GHOST_BTN} aria-label="ChatGPT 会话" title="ChatGPT 会话">
            <BookOpenIcon />
            <span className="hidden sm:inline">ChatGPT 会话</span>
          </button>
          <button onClick={() => navigate("/models")} className={GHOST_BTN} aria-label="模型与价格" title="模型与价格">
            <span aria-hidden="true" className="text-[13px] leading-none">¥</span>
            <span className="hidden sm:inline">模型</span>
          </button>
          <button onClick={logout} className={GHOST_BTN} aria-label="退出" title="退出">
            <LogOutIcon />
            <span className="hidden sm:inline">退出</span>
          </button>
        </div>
      </header>

      <main className="flex min-h-0 flex-1 overflow-y-auto px-4 py-4 md:overflow-hidden md:px-5">
        <div className="mx-auto flex min-h-0 w-full max-w-[120rem] flex-1 flex-col gap-4">
          {/* 上方摘要：用户状态 + 本机应用 + 登录设备，三卡一行不换行 */}
          <div className="grid shrink-0 grid-cols-1 items-start gap-3 md:grid-cols-[19rem_minmax(0,1fr)_15rem]">
            {/* 余额 */}
            <section className={CARD_TIGHT}>
              <div className="flex items-start justify-between gap-2">
                <div className="min-w-0">
                  <p className={`${LABEL} truncate`}>{auth?.username ?? "已登录"}</p>
                  <div className="mt-0.5 flex items-center gap-1.5">
                    <p className="text-xl font-semibold text-gray-900 dark:text-white" aria-live="polite">
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
                  <p className={`mt-0.5 text-[11px] ${SUBTLE}`}>
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
            <section className={`${CARD_TIGHT} min-w-0`}>
              <div className="mb-2 flex items-center justify-between">
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
                  {/* 目标用横向胶囊排一行，详细说明只给当前选中项，避免撑高顶栏 */}
                  <div className="flex flex-wrap items-center gap-1.5">
                    {targets.map((t) => {
                      const active = t.id === targetId;
                      return (
                        <button
                          key={t.id}
                          onClick={() => pickTarget(t.id)}
                          disabled={!t.installed}
                          title={t.installed ? t.name : `${t.name}（未安装）`}
                          className={`flex items-center gap-1.5 rounded-full border px-2 py-1 text-[11px] transition ${
                            active
                              ? "border-transparent bg-[var(--nk-accent)] font-medium text-white"
                              : t.installed
                                ? "[border-color:var(--nk-line)] text-gray-600 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                                : "cursor-not-allowed border-dashed [border-color:var(--nk-line)] text-gray-400 opacity-60 dark:text-gray-500"
                          }`}
                        >
                          <TargetAppIcon targetId={t.id} name={t.name} icon={t.icon} />
                          <span className="max-w-[7.5rem] truncate">{t.name}</span>
                          {active && <span aria-hidden="true">✓</span>}
                        </button>
                      );
                    })}
                    {installedTargets.length > 1 && (
                      <button
                        onClick={() => pickTarget(ALL_TARGETS)}
                        title="对所有已安装应用同时接入"
                        className={`flex items-center gap-1 rounded-full border px-2 py-1 text-[11px] transition ${
                          targetId === ALL_TARGETS
                            ? "border-transparent bg-[var(--nk-accent)] font-medium text-white"
                            : "[border-color:var(--nk-line)] text-gray-600 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                        }`}
                      >
                        全部
                        <span className="opacity-60">{installedTargets.length}</span>
                      </button>
                    )}
                    <button
                      onClick={() => navigate("/install-guide")}
                      className="ml-1 text-[11px] text-[var(--nk-info)] hover:underline"
                    >
                      安装指引
                    </button>
                  </div>

                  {/* 当前应用的一句话说明 + Codex 订阅模式（都压成一行内） */}
                  {(() => {
                    const active = targets.find((t) => t.id === targetId);
                    if (!active?.installed) return null;
                    const result = results[active.id];
                    if (active.id === "codex") {
                      return (
                        <div className="mt-2 flex flex-wrap items-center gap-1.5">
                          <span className={`text-[10px] ${SUBTLE}`}>ChatGPT 订阅</span>
                          {[
                            { mixed: false, label: "未订阅" },
                            { mixed: true, label: "已订阅" },
                          ].map((opt) => (
                            <button
                              key={String(opt.mixed)}
                              onClick={() => pickCodexMixed(opt.mixed)}
                              title={
                                opt.mixed
                                  ? "保留 ChatGPT 登录态，官方额度照常，模型走 momo"
                                  : "只用 momo 额度，不需要 ChatGPT 账号"
                              }
                              className={`rounded-full border px-2 py-0.5 text-[11px] transition ${
                                codexMixed === opt.mixed
                                  ? "border-transparent bg-[var(--nk-accent)] font-medium text-white"
                                  : "[border-color:var(--nk-line)] text-gray-500 hover:bg-black/[0.04] dark:text-gray-400 dark:hover:bg-white/10"
                              }`}
                            >
                              {opt.label}
                            </button>
                          ))}
                          {result && (
                            <span
                              title={result.ok ? "设置已更新" : result.error}
                              className={`ml-1 truncate text-[10px] ${
                                result.ok ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"
                              }`}
                            >
                              {result.ok
                                ? result.changed?.length
                                  ? `✓ 已更新 ${result.changed.length} 项`
                                  : "✓ 已是最新"
                                : `✗ ${result.error}`}
                            </span>
                          )}
                        </div>
                      );
                    }
                    const note =
                      active.id === "claude-desktop"
                        ? "仅作用于内置 Claude Code 面板，桌面端普通对话仍用 Anthropic 账号"
                        : active.id === "claude-cli"
                          ? "写入 ~/.claude/settings.json，终端里的 claude 直接生效"
                          : active.id === "grok"
                            ? "写入 ~/.grok/config.toml 的 model.momotoken 段"
                            : active.id === "antigravity"
                              ? "仅支持 Gemini 系模型；环境变量写入后需重开终端生效"
                              : "";
                    return (
                      <div className="mt-2 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                        {note && <span className={`min-w-0 truncate text-[10px] ${SUBTLE}`}>{note}</span>}
                        {result && (
                          <span
                            title={result.ok ? "设置已更新" : result.error}
                            className={`min-w-0 truncate text-[10px] ${
                              result.ok ? "text-green-600 dark:text-green-400" : "text-red-600 dark:text-red-400"
                            }`}
                          >
                            {result.ok
                              ? result.changed?.length
                                ? `✓ 已更新 ${result.changed.length} 项`
                                : "✓ 已是最新"
                              : `✗ ${result.error}`}
                          </span>
                        )}
                      </div>
                    );
                  })()}
                </>
              )}
            </section>

            {/* 设备（折叠） */}
            <section className={CARD_TIGHT}>
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
                <div className="mt-2 max-h-36 space-y-2 overflow-y-auto pr-1">
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

          {/* 下方三段：厂家 → 模型 → 分组（从左到右） */}
          <div className="flex min-h-0 min-w-0 flex-1 flex-col md:overflow-hidden">
            {/* 分组 + 模型选择（跟随所选应用推荐） */}
            {installedTargets.length > 0 && (
            <section className={`${CARD} flex min-h-[22rem] flex-1 flex-col md:min-h-0`}>
              <div className="mb-1.5 flex shrink-0 flex-col gap-1 sm:flex-row sm:items-center sm:justify-between">
                <h2 className={TITLE}>{targetLabel ? `为 ${targetLabel} 选择模型` : "选择模型"}</h2>
                {currentGroup && (
                  <span className={`relative flex items-center gap-1 self-start ${SUBTLE} sm:self-auto`}>
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
                <div
                  role="status"
                  aria-live="polite"
                  className={`mb-1.5 shrink-0 text-[11px] leading-4 ${
                    activeGroupView.kind === "active"
                      ? "text-green-700 dark:text-green-400"
                      : activeGroupView.kind === "changed"
                        ? "text-orange-700 dark:text-orange-400"
                      : "text-gray-500 dark:text-gray-400"
                  }`}
                >
                  <div className="flex min-w-0 flex-wrap items-baseline gap-x-3 gap-y-0">
                    {!detecting && activeSelectionRows.length === detectionTargetIds.length && activeSelectionRows.length > 0 ? (
                      <>
                        <span className="font-medium">已生效：</span>
                        {activeSelectionRows.map((selection) => (
                          <span key={selection.target} className="break-words">
                            {activeSelectionRows.length > 1 && `${selection.target}：`}
                            {selection.provider} · {selection.model} · {selection.group}
                          </span>
                        ))}
                      </>
                    ) : (
                      <span>{activeGroupView.text}</span>
                    )}
                    {/* 草稿/推荐只在和当前生效不一致时才出现，平时不占地方 */}
                    {draftSource === "manual" && draftSelection && (
                      <span className="text-gray-500 dark:text-gray-400">
                        草稿：{draftSelection.provider} · {draftSelection.model} · {draftSelection.group}
                      </span>
                    )}
                    {draftSource !== "recommendation" && recommendedSelection && (
                      <span className="text-gray-500 dark:text-gray-400">
                        推荐：{recommendedSelection.provider} · {recommendedSelection.model} · {recommendedSelection.group}
                      </span>
                    )}
                  </div>
                </div>
              )}

              {groups.length === 0 ? (
                <p className={SUBTLE}>当前账号没有可用模型服务，请联系管理员开通。</p>
              ) : (
                <>
                  {/* 三列：厂家 → 模型 → 分组，从左到右 */}
                  <div className="grid min-h-0 flex-1 gap-3 overflow-hidden md:grid-cols-[13rem_minmax(0,1fr)_17rem]">
                    {/* 第一列：厂家 */}
                    <div className="flex min-h-0 flex-col overflow-hidden">
                      <p className={`${LABEL} shrink-0 px-1`}>
                        厂家
                        <span className="ml-1.5 opacity-70">{vendorTabs.length}</span>
                      </p>
                      <div className="mt-1.5 min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
                        {vendorTabs.map((tab) => (
                          <button
                            key={tab.vendor}
                            onClick={() => pickVendor(tab)}
                            title={`${tab.vendor} · ${tab.models.length} 个模型`}
                            className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition ${
                              tab.vendor === activeVendor
                                ? "bg-[var(--nk-accent)] font-medium text-white"
                                : "text-gray-600 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                            }`}
                          >
                            <VendorIcon vendor={tab.vendor} />
                            <span className="min-w-0 flex-1 truncate">{tab.vendor}</span>
                            <span className="shrink-0 tabular-nums opacity-70">{tab.models.length}</span>
                            {recommendVendor === tab.vendor && (
                              <span
                                className={`shrink-0 rounded px-1 text-[9px] ${
                                  tab.vendor === activeVendor
                                    ? "bg-white/20"
                                    : "bg-[var(--nk-info-soft)] text-[var(--nk-info)]"
                                }`}
                              >
                                原生
                              </span>
                            )}
                          </button>
                        ))}
                      </div>
                    </div>

                    {/* 第二列：模型（默认按发布时间从新到旧） */}
                    <div className="flex min-h-0 min-w-0 flex-col overflow-hidden border-t pt-2 md:border-l md:border-t-0 md:pl-3 md:pt-0 [border-color:var(--nk-line)]">
                      <div className="flex shrink-0 items-center justify-between gap-2">
                        <p className={LABEL}>
                          模型
                          <span className="ml-1.5 opacity-70">{models.length}</span>
                          <span className={`ml-2 text-[10px] font-normal ${SUBTLE}`}>按发布时间</span>
                        </p>
                        {searchOpen ? (
                          <div className="flex shrink-0 items-center gap-0.5">
                            <input
                              autoFocus
                              value={modelFilter}
                              onChange={(e) => setModelFilter(e.target.value)}
                              onKeyDown={(e) => {
                                if (e.key === "Escape") {
                                  setModelFilter("");
                                  setSearchOpen(false);
                                }
                              }}
                              placeholder="搜索模型"
                              aria-label="搜索模型"
                              className="nk-input w-28 py-1 text-xs sm:w-40"
                            />
                            <button
                              onClick={() => {
                                setModelFilter("");
                                setSearchOpen(false);
                              }}
                              aria-label="收起搜索"
                              title="收起搜索"
                              className="nk-btn-ghost px-1.5 py-1 text-xs"
                            >
                              ✕
                            </button>
                          </div>
                        ) : (
                          <button
                            onClick={() => setSearchOpen(true)}
                            aria-label="搜索模型"
                            title="搜索模型"
                            className="nk-btn-ghost shrink-0 px-2 py-1"
                          >
                            <svg viewBox="0 0 20 20" className="h-4 w-4" fill="none" stroke="currentColor" strokeWidth="1.6">
                              <circle cx="8.5" cy="8.5" r="5" />
                              <path d="M12.5 12.5 17 17" strokeLinecap="round" />
                            </svg>
                          </button>
                        )}
                      </div>
                      <div className="nk-model-scroll mt-1.5 min-h-0 flex-1">
                        {models.length === 0 && (
                          <p className="nk-empty">
                            {modelFilter ? "没有匹配的模型，请换一个关键词。" : "当前没有可用模型。"}
                          </p>
                        )}
                        <div className="nk-model-grid">
                          {models.map((choice) => {
                            const compat = compatOf(choice.name);
                            return (
                              <button
                                key={choice.name}
                                onClick={() => pickModel(choice)}
                                aria-pressed={choice.name === model}
                                className={`nk-model-card w-full text-left ${
                                  choice.name === model
                                    ? "nk-model-card-selected text-gray-900 dark:text-white"
                                    : "text-gray-600 dark:text-gray-300"
                                }`}
                              >
                                <span className="flex min-w-0 flex-col justify-center gap-0.5">
                                  <span className="flex min-w-0 items-baseline justify-between gap-2">
                                    <span className="min-w-0 truncate font-mono text-sm font-semibold">
                                      {choice.name}
                                    </span>
                                    <span className={`shrink-0 text-[10px] tabular-nums ${SUBTLE}`}>
                                      {releaseLabelOf(choice.name)}
                                    </span>
                                  </span>
                                  <span className="flex min-w-0 flex-wrap items-center gap-1">
                                    {modelTags(choice.name).map((tag) => (
                                      <span
                                        key={tag.id}
                                        className={`rounded-full px-1.5 py-0.5 text-[9px] leading-3 ${tag.className}`}
                                      >
                                        {tag.label}
                                      </span>
                                    ))}
                                    <span className="tabular-nums text-[11px] text-gray-500 dark:text-gray-400">
                                      {choice.groups.length} 组
                                    </span>
                                  </span>
                                  <span className="flex min-w-0 items-center justify-between gap-1.5">
                                    <span className="flex min-w-0 items-center gap-1.5">
                                      {compat && (
                                        <span
                                          title={compat.note}
                                          className={`truncate rounded-full px-1.5 py-0.5 text-[10px] ${COMPAT_STYLE[compat.level]}`}
                                        >
                                          {COMPAT_LABEL[compat.level]}
                                        </span>
                                      )}
                                    </span>
                                    {choice.name === model && <span aria-hidden="true">✓</span>}
                                  </span>
                                </span>
                              </button>
                            );
                          })}
                        </div>
                      </div>
                    </div>

                    {/* 第三列：分组（选了模型才出现） */}
                    <div className="flex min-h-0 flex-col overflow-hidden border-t pt-2 md:border-l md:border-t-0 md:pl-3 md:pt-0 [border-color:var(--nk-line)]">
                      <p className={`${LABEL} shrink-0`}>
                        令牌分组
                        <span className="ml-1.5 opacity-70">{modelGroups.length}</span>
                      </p>
                      {!model ? (
                        <p className={`mt-2 text-[11px] ${SUBTLE}`}>先选择模型</p>
                      ) : (
                        <>
                          {modelGroups.length > 0 && (
                            <button
                              onClick={runBenchmarks}
                              disabled={benchmarkRunning}
                              title="对当前模型的各分组发真实请求测首字延迟"
                              className={`mt-1 shrink-0 self-start rounded-lg px-2 py-0.5 text-[11px] transition ${
                                benchmarkRunning
                                  ? "cursor-wait text-gray-400 dark:text-gray-500"
                                  : "text-[var(--nk-info)] hover:bg-black/[0.04] dark:hover:bg-white/10"
                              }`}
                            >
                              {benchmarkRunning ? "测速中…" : "⚡ 测速"}
                            </button>
                          )}
                          {!hasTokenGroups && (
                            <p className={`mt-1 text-[10px] ${SUBTLE}`}>
                              服务端未下发该模型的令牌分组（enable_groups），当前只能列出账号可用的分组
                            </p>
                          )}
                          <div className="mt-1.5 min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
                            {modelGroups.map((g) => (
                              <button
                                key={g.name}
                                onClick={() => g.usable && pickGroup(g.name)}
                                disabled={!g.usable}
                                aria-pressed={g.name === group}
                                title={
                                  g.usable
                                    ? `${g.desc || g.name}（${g.name}）`
                                    : `${g.desc || g.name}：当前账号未开通该分组`
                                }
                                className={`w-full rounded-lg border px-2 py-1.5 text-left text-[11px] transition ${
                                  g.name === group
                                    ? "border-transparent bg-[var(--nk-accent)] font-medium text-white"
                                    : g.usable
                                      ? "[border-color:var(--nk-line)] text-gray-600 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                                      : "cursor-not-allowed border-dashed [border-color:var(--nk-line)] text-gray-400 opacity-70 dark:text-gray-500"
                                }`}
                              >
                                <span className="flex items-center justify-between gap-2">
                                  <span className="min-w-0 truncate font-medium">
                                    {g.desc?.trim() || g.name}
                                  </span>
                                  <span className="shrink-0 tabular-nums opacity-70">{g.ratio}x</span>
                                </span>
                                <span className="mt-0.5 flex items-center justify-between gap-2 text-[10px] opacity-80">
                                  <span className="min-w-0 truncate font-mono">{g.name}</span>
                                  <span className="shrink-0 tabular-nums">
                                    {g.usable ? groupPriceLabel(model, g.ratio) : "未开通"}
                                  </span>
                                </span>
                                {typeof benchmarks[g.name] === "number" && (
                                  <span className="mt-0.5 block text-[10px] tabular-nums opacity-80">
                                    实测首字 {benchmarks[g.name]}ms
                                  </span>
                                )}
                              </button>
                            ))}
                          </div>
                        </>
                      )}
                    </div>
                  </div>
                  <div className="mt-1.5 shrink-0 border-t pt-1.5 [border-color:var(--nk-line)]">
                  <div className="grid grid-cols-4 gap-1">
                    <button
                      onClick={enable}
                      disabled={provisioning || !group || !model || !targetId}
                      title={targetLabel ? `接入到 ${targetLabel}` : "选择应用后接入"}
                      aria-label="接入到应用"
                      className="nk-btn-primary w-full min-w-0 px-1 text-center text-[11px]"
                    >
                      {provisioning ? "接入中…" : "接入"}
                    </button>
                    <button
                      onClick={restartTargets}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title={targetLabel ? `启动 / 重启 ${targetLabel}，让刚接入的设置生效` : "选择应用后可重启"}
                      aria-label="重启应用"
                      className={`${GHOST_BTN} w-full min-w-0 px-1 text-center text-[11px]`}
                    >
                      {restarting ? "重启中…" : "重启"}
                    </button>
                    <button
                      onClick={testConnectivity}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title="检查当前设置是否能正常使用"
                      aria-label="检查连接"
                      className={`${GHOST_BTN} w-full min-w-0 px-1 text-center text-[11px]`}
                    >
                      {testing ? "检查中…" : "检查"}
                    </button>
                    <button
                      onClick={restoreDefaults}
                      disabled={provisioning || testing || restoring || restarting || !targetId}
                      title="移除 Niko 的设置，恢复用官方账号登录"
                      aria-label="恢复到官方"
                      className={`${GHOST_BTN} w-full min-w-0 px-1 text-center text-[11px] ${confirmRestore ? "border-orange-400 text-orange-600 dark:text-orange-400" : ""}`}
                    >
                      {restoring ? "恢复中…" : confirmRestore ? "确认" : "恢复"}
                    </button>
                  </div>
                  {notice && (
                    <p
                      title={notice.text}
                      className={`mt-1.5 truncate text-xs ${
                        notice.ok
                          ? "text-green-600 dark:text-green-400"
                          : "text-orange-600 dark:text-orange-400"
                      }`}
                    >
                      {notice.text}
                    </p>
                  )}
                  </div>
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
