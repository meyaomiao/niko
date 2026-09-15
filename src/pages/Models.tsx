// 模型与价格：按 厂家 → 模型 → 分组 三列浏览（与首页同一布局语言）
// 模型卡片显示「原价」（分组倍率 1x 的官方基准价），稳定不随分组切换变化；
// 分组折算价只在第三列跟随所选模型的分组列表展示。

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth } from "../store/auth";
import { api, type BootstrapData, type ModelMetadata, type PricingMeta, type UsageSummary, type VendorMeta } from "../api/client";
import { buildPricingIndex, fmtUSD, priceOf, type ModelPrice } from "../lib/pricing";
import { VENDORS } from "../lib/vendor";
import { buildVendorIndex, vendorNameOf } from "../lib/vendorCatalog";
import { buildGroupCatalog, type GroupInfo } from "../lib/groupCatalog";
import { compareModelsByRelease } from "../lib/modelOrder";
import { buildModelCatalog } from "../lib/catalog";
import { computeTags, vendorPriceLevels, vendorUsageRanks, type ModelTag } from "../lib/modelTags";
import { VendorIcon } from "../components/VendorIcon";
import { ArrowLeftIcon } from "../components/Icons";
import { friendlyDesktopError } from "../lib/copy";

const CARD = "nk-card";
const SUBTLE = "nk-muted";
const TITLE = "nk-title";

const BENCH_KEY = "niko_model_bench";

type BenchEntry = { median: number; samples: number; at: number };

/** 首页测速落地的缓存：model -> 最近一次实测 */
function loadBenchCache(): Record<string, BenchEntry> {
  try {
    return JSON.parse(localStorage.getItem(BENCH_KEY) ?? "{}") as Record<string, BenchEntry>;
  } catch {
    return {};
  }
}

function modelName(item: BootstrapData["models"][number]): string {
  return typeof item === "string" ? item : (item.name ?? item.model_name ?? item.id ?? "");
}

/** 与首页同一套排序：发布日期新→旧 → 服务端发布序 → 名称 */
function orderedModels(data: BootstrapData, candidates?: string[]): string[] {
  const source = candidates ?? (data.models ?? []).map(modelName).filter(Boolean);
  const unique = Array.from(new Set(source.filter(Boolean)));
  const index = new Map((data.model_order ?? []).map((name, i) => [name, i]));
  const priceByName = new Map((data.pricing ?? []).map((item) => [item.model_name, item]));
  const dateOf = (name: string): number | null => {
    const meta = data.model_metadata?.[name];
    const label = releaseDate(meta, priceByName.get(name)?.release_date);
    if (!label) return null;
    const ts = Date.parse(label);
    return Number.isFinite(ts) ? ts : null;
  };
  return unique.sort((a, b) =>
    compareModelsByRelease(a, b, {
      dateOf,
      orderOf: (name) => index.get(name),
    })
  );
}

function releaseDate(meta: ModelMetadata | undefined, fallback?: string): string {
  return (
    meta?.release_date ?? meta?.released_at ?? meta?.official_release_date ?? meta?.version_date ?? fallback ?? ""
  );
}

function delayChip(ms: number): { label: string; className: string } {
  if (ms <= 3000) return { label: "快", className: "bg-green-500/10 text-green-700 dark:text-green-400" };
  if (ms <= 8000) return { label: "正常", className: "bg-amber-500/10 text-amber-700 dark:text-amber-400" };
  return { label: "偏慢", className: "bg-red-500/10 text-red-600 dark:text-red-400" };
}

interface Card {
  name: string;
  release: string;
  /** 原价：分组倍率 1x 的官方基准价，稳定不随分组切换变化 */
  price: ModelPrice | null;
  /** 相对价位 0~1（按原价输入价在同厂家有价模型中的分位），仅用于价格条长度 */
  level: number;
  bench?: BenchEntry;
  tags: ModelTag[];
  /** 该模型的令牌分组（服务端 enable_groups） */
  groups: GroupInfo[];
  /** 服务端是否下发了该模型的 enable_groups */
  hasTokenGroups: boolean;
}

export default function Models() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const token = auth?.accessToken;

  const [data, setData] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [bench, setBench] = useState<Record<string, BenchEntry>>({});
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [vendorMetas] = useState<VendorMeta[]>([]);
  const [pricingMeta, setPricingMeta] = useState<PricingMeta | null>(null);
  const [activeVendor, setActiveVendor] = useState<string>("全部");
  const [selectedModel, setSelectedModel] = useState<string>("");
  const [selectedGroup, setSelectedGroup] = useState<string>("");
  const [benchmarks, setBenchmarks] = useState<Record<string, number | "loading" | null>>({});
  const [benchErrors, setBenchErrors] = useState<Record<string, string>>({});
  const [benchmarkRunning, setBenchmarkRunning] = useState(false);
  // 测速进度：仅 running 时展示「done/total」
  const [benchDone, setBenchDone] = useState(0);
  const [benchTotal, setBenchTotal] = useState(0);
  // 每组耗时打点：区分「申请密钥」与「测速请求」哪段慢
  const [benchTimings, setBenchTimings] = useState<Record<string, string>>({});
  // 测速轮次号：切模型时 +1 使旧循环自行中止，避免旧模型的分组结果写回
  const benchRunRef = useRef(0);

  // 切换模型即清空上一轮分组测速结果（结果只对单次测速的模型有效）
  useEffect(() => {
    benchRunRef.current += 1;
    setBenchmarks({});
    setBenchErrors({});
    setBenchmarkRunning(false);
    setBenchDone(0);
    setBenchTotal(0);
    setBench(loadBenchCache());
  }, [selectedModel]);

  useEffect(() => {
    setBench(loadBenchCache());
    if (!token) {
      navigate("/login", { replace: true });
      return;
    }
    api
      .bootstrap(token)
      .then((res) => {
        setData(res);
        // 用户自己的用量聚合 → 厂家内排名的「常用 / 性价比」；失败静默，不阻塞目录
        api
          .usageSummary(token)
          .then((s) => setUsage(s))
          .catch(() => undefined);
        // 厂商目录 + 全部分组说明（公开接口）；失败时回退本地启发式
        api
          .pricingMeta()
          .then((meta) => setPricingMeta(meta))
          .catch(() => undefined);
      })
      .catch((e) => setError(friendlyDesktopError(e)))
      .finally(() => setLoading(false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const groups = data?.groups ?? [];
  const pricingIndex = useMemo(() => buildPricingIndex(data?.pricing), [data]);
  // 新版 bootstrap 的 models/groups[].models 可能为空，目录改由 pricing 反查
  const catalog = useMemo(() => buildModelCatalog(data?.pricing, groups), [data?.pricing, groups]);

  // 分组目录：模型 → 令牌分组（服务端 enable_groups + 中文说明/倍率）
  const groupCatalog = useMemo(
    () =>
      buildGroupCatalog({
        accountGroups: groups,
        usableGroup: pricingMeta?.usableGroup,
        groupRatio: pricingMeta?.groupRatio,
      }),
    [groups, pricingMeta]
  );

  const cards = useMemo(() => {
    if (!data) return [];
    const names = catalog.models.length > 0 ? orderedModels(data, catalog.models.map((m) => m.name)) : [];
    // 原价 = 官方基准（分组倍率 1x）；分组折算价只在第三列按分组展示
    const priceByName = new Map<string, ModelPrice | null>();
    for (const name of names) {
      priceByName.set(name, priceOf(pricingIndex.get(name), 1) as ModelPrice | null);
    }
    // 价格分位与用量名次都按「厂家内」比较，价格条与标签口径一致（基于原价，稳定）
    const levels = vendorPriceLevels(names, (name) => {
      const price = priceByName.get(name);
      return price && !price.perRequest ? price.input : undefined;
    });
    const ranks = vendorUsageRanks(usage, data?.model_usage);

    return names.map((name) => {
      const price = priceByName.get(name) ?? null;
      const level = levels.get(name);
      const release = releaseDate(data.model_metadata?.[name], pricingIndex.get(name)?.release_date);
      const enableGroups =
        data.pricing?.find((item) => item.model_name === name)?.enable_groups ?? [];
      return {
        name,
        release,
        price,
        level: level ?? 0.5,
        bench: bench[name],
        tags: computeTags({ name, release, rank: ranks.get(name), level }),
        groups: catalog
          .groupsOf(name)
          .map((groupName) => {
            const info = groupCatalog.get(groupName);
            const account = groups.find((g) => g.name === groupName);
            return (
              info ?? {
                name: groupName,
                desc: account?.desc ?? "",
                ratio: account?.ratio ?? 1,
                usable: Boolean(account),
              }
            );
          }),
        hasTokenGroups: enableGroups.length > 0,
        // 目录来自 pricing 反查时，hasTokenGroups 恒真；这里保留字段供排查
      };
    });
  }, [data, pricingIndex, bench, usage, groupCatalog, groups, catalog, data?.model_usage]);

  // 服务端厂商目录：模型 → 厂家（服务端优先，缺失回退本地启发式）
  const vendorIndex = useMemo(
    () => buildVendorIndex(data?.pricing, pricingMeta?.vendors ?? vendorMetas),
    [data?.pricing, pricingMeta?.vendors, vendorMetas]
  );
  const vendorOfName = useCallback(
    (name: string) => vendorNameOf(vendorIndex, name),
    [vendorIndex]
  );
  /** 分区顺序：服务端厂商表顺序优先，其余按本地启发式表补齐 */
  const vendorOrder = useMemo(() => {
    const fromServer = (pricingMeta?.vendors ?? vendorMetas).map((v) => v.name).filter(Boolean);
    const seen = new Set(fromServer);
    return [...fromServer, ...VENDORS.filter((v) => !seen.has(v))];
  }, [pricingMeta?.vendors, vendorMetas]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return cards.filter((c) => c.name.toLowerCase().includes(q));
  }, [cards, query]);

  const counts = useMemo(() => {
    const map = new Map<string, number>();
    for (const c of cards) {
      const v = vendorOfName(c.name);
      map.set(v, (map.get(v) ?? 0) + 1);
    }
    return map;
  }, [cards, vendorOfName]);

  // 厂家列：按模型数量降序；「全部」置顶
  const orderedVendors = useMemo(
    () =>
      vendorOrder
        .filter((vendor) => (counts.get(vendor) ?? 0) > 0)
        .sort((a, b) => (counts.get(b) ?? 0) - (counts.get(a) ?? 0) || a.localeCompare(b)),
    [vendorOrder, counts]
  );

  // 模型列：全部=按厂家分区铺开；选中某家=只看该家
  const sections = useMemo(() => {
    const list = (activeVendor === "全部" ? orderedVendors : [activeVendor]).map((vendor) => ({
      vendor,
      cards: filtered.filter((c) => vendorOfName(c.name) === vendor),
    }));
    return list.filter((s) => s.cards.length > 0);
  }, [activeVendor, orderedVendors, filtered, vendorOfName]);

  const total = filtered.length;

  // 第三列：所选模型的分组列表
  const selected = useMemo(() => cards.find((c) => c.name === selectedModel), [cards, selectedModel]);

  // 分组测速：与首页同一套——对当前模型每个分组发 3 次最小流式请求取 TTFT 中位数；
  // 已保存密钥所属分组直接复用密钥，其余分组临时申请（不覆盖已保存密钥）。
  const runBenchmarks = async () => {
    if (!auth?.accessToken || !selectedModel || benchmarkRunning || !selected) return;
    const groupsToRun = selected.groups.filter((g) => g.usable);
    if (groupsToRun.length === 0) return;
    setBenchmarkRunning(true);
    setBenchErrors({});
    setBenchDone(0);
    setBenchTotal(groupsToRun.length);
    setBenchmarks(Object.fromEntries(groupsToRun.map((g) => [g.name, "loading" as const])));
    const RELAY_BASE_URL = "https://momotoken.win/v1";
    let done = 0;
    const runId = ++benchRunRef.current;
    for (const g of groupsToRun) {
      // 模型已切换：本轮结果作废，立即停止
      if (benchRunRef.current !== runId) return;
      const t0 = performance.now();
      let provMs = 0;
      let benchMs = 0;
      try {
        let apiKey = auth.apiKey && auth.apiKeyGroup === g.name ? auth.apiKey : null;
        if (!apiKey) {
          const res = await api.provision(auth.accessToken, g.name);
          apiKey = res.api_key;
          provMs = Math.round(performance.now() - t0);
        }
        const result = await invoke<{
          median_ttft_ms: number | null;
          samples: { ttft_ms: number | null; error: string | null }[];
        }>("benchmark_group", {
          // Tauri v2：Rust snake_case 参数在 JS 侧必须传 camelCase
          baseUrl: RELAY_BASE_URL,
          apiKey,
          model: selectedModel,
          samples: 3,
        });
        benchMs = Math.round(performance.now() - t0 - provMs);
        setBenchTimings((prev) => ({
          ...prev,
          [g.name]: `申请密钥 ${provMs || 0}ms（复用已存密钥时为 0） · 测速请求 ${benchMs}ms（含 3 次采样）`,
        }));
        if (result.median_ttft_ms == null) {
          // 采样失败的具体原因在 samples[].error，取第一条给用户看
          const detail = result.samples?.find((s) => s.error)?.error ?? "未取到首字延迟";
          setBenchErrors((prev) => ({ ...prev, [g.name]: detail }));
        }
        if (result.median_ttft_ms != null) {
          // 落一份本地缓存，与首页共用，供卡片「实测」展示
          try {
            const cache = JSON.parse(localStorage.getItem(BENCH_KEY) ?? "{}") as Record<string, unknown>;
            cache[selectedModel] = { median: result.median_ttft_ms, samples: 3, at: Date.now() };
            localStorage.setItem(BENCH_KEY, JSON.stringify(cache));
          } catch { /* 缓存失败不影响测速 */ }
        }
        setBenchmarks((prev) => ({ ...prev, [g.name]: result.median_ttft_ms }));
      } catch (e) {
        setBenchErrors((prev) => ({ ...prev, [g.name]: e instanceof Error ? e.message : String(e) }));
        setBenchmarks((prev) => ({ ...prev, [g.name]: null }));
      }
      done += 1;
      setBenchDone(done);
    }
    setBenchmarkRunning(false);
    // 刷新卡片上的「实测」缓存（首页同款 BENCH_KEY）
    setBench(loadBenchCache());
  };

  // 选模型时默认带出第一个可用分组
  useEffect(() => {
    if (!selectedModel) {
      setSelectedGroup("");
      return;
    }
    const card = cards.find((c) => c.name === selectedModel);
    const firstUsable = card?.groups.find((g) => g.usable)?.name ?? card?.groups[0]?.name ?? "";
    setSelectedGroup(firstUsable);
  }, [selectedModel, cards]);
  const selectedItem = selectedModel ? pricingIndex.get(selectedModel) : undefined;

  return (
    <div className="nk-shell">
      <header className="nk-header">
        <button onClick={() => navigate("/home")} aria-label="返回首页" className="nk-btn-ghost px-2.5">
          <ArrowLeftIcon />
        </button>
        <h1 className={TITLE}>模型与价格</h1>
      </header>

      {/* 与首页一致：桌面端锁住外层高度（overflow-hidden），三列各自内部滚动 */}
      <main className="flex min-h-0 flex-1 overflow-y-auto px-4 py-4 md:overflow-hidden md:px-5">
        <div className="mx-auto flex min-h-0 w-full max-w-7xl flex-col gap-3">
          {loading && <p className={CARD}>正在加载模型目录…</p>}
          {error && <p className={`${CARD} text-red-500`}>{error}</p>}

          {!loading && !error && (
            <div className="grid min-h-0 flex-1 gap-3 overflow-hidden md:grid-cols-[11rem_minmax(0,1fr)_17rem]">
              {/* 第一列：厂家 */}
              <div className={`${CARD} flex min-h-0 flex-col overflow-hidden`}>
                <p className={`shrink-0 px-1 pb-2 text-[11px] font-medium ${SUBTLE}`}>厂家</p>
                <div className="min-h-0 flex-1 space-y-0.5 overflow-y-auto pr-0.5">
                  {[{ name: "全部", count: cards.length }, ...orderedVendors.map((v) => ({ name: v, count: counts.get(v) ?? 0 }))].map(
                    (v) => (
                      <button
                        key={v.name}
                        onClick={() => setActiveVendor(v.name)}
                        aria-pressed={activeVendor === v.name}
                        className={`flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-xs transition ${
                          activeVendor === v.name
                            ? "bg-[var(--nk-accent)] font-medium text-white"
                            : "text-gray-700 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                        }`}
                      >
                        {v.name !== "全部" && <VendorIcon vendor={v.name} />}
                        <span className="min-w-0 flex-1 truncate">{v.name}</span>
                        <span className="shrink-0 tabular-nums opacity-60">{v.count}</span>
                      </button>
                    )
                  )}
                </div>
              </div>

              {/* 第二列：模型（搜索 + 富卡片，保留原模型页内容） */}
              <div className="flex min-h-0 flex-col gap-2 overflow-hidden">
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder={`搜索 ${total} 个模型…`}
                  aria-label="搜索模型"
                  className="nk-input shrink-0 py-1.5 text-xs"
                />
                <div className="min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
                  {sections.map(({ vendor, cards: list }) => (
                    <section key={vendor}>
                      {activeVendor === "全部" && (
                        <h2 className="mb-1.5 flex items-center gap-2 px-1 text-xs font-semibold text-gray-900 dark:text-gray-100">
                          <VendorIcon vendor={vendor} />
                          {vendor}
                          <span className={`text-[10px] font-normal tabular-nums ${SUBTLE}`}>{list.length}</span>
                        </h2>
                      )}
                      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2">
                        {list.map((card) => (
                          <ModelCard
                            key={card.name}
                            card={card}
                            selected={card.name === selectedModel}
                            onSelect={() => setSelectedModel(card.name === selectedModel ? "" : card.name)}
                          />
                        ))}
                      </div>
                    </section>
                  ))}
                  {sections.length === 0 && <p className={`${CARD} ${SUBTLE}`}>没有匹配的模型。</p>}
                </div>
              </div>

              {/* 第三列：所选模型的分组（分组折算价跟分组走） */}
              <div className={`${CARD} flex min-h-0 flex-col overflow-hidden`}>
                <div className="flex shrink-0 items-center justify-between gap-2 px-1 pb-2">
                  <p className={`text-[11px] font-medium ${SUBTLE}`}>分组</p>
                  {selected && selected.groups.some((g) => g.usable) && (
                    <button
                      onClick={runBenchmarks}
                      disabled={benchmarkRunning}
                      title="对当前模型的各分组发真实请求测首字延迟"
                      className="rounded-full border px-2 py-0.5 text-[10px] transition hover:bg-black/[0.04] disabled:opacity-50 [border-color:var(--nk-line)] dark:hover:bg-white/10"
                    >
                      {benchmarkRunning ? `测速中 ${benchDone}/${benchTotal}…` : "⚡ 测速"}
                    </button>
                  )}
                </div>
                {!selected ? (
                  <p className={`px-1 text-xs ${SUBTLE}`}>在中间选择一个模型后，这里显示它支持的分组与折算价。</p>
                ) : (
                  <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto pr-0.5">
                    <p className="truncate px-1 font-mono text-[11px] font-semibold text-gray-900 dark:text-gray-100" title={selected.name}>
                      {selected.name}
                    </p>
                    {selected.groups.length === 0 && (
                      <p className={`px-1 text-[11px] ${SUBTLE}`}>
                        {selected.hasTokenGroups ? "分组目录缺少该模型的令牌分组说明" : "服务端未下发该模型的令牌分组"}
                      </p>
                    )}
                    {selected.groups.map((g) => {
                      const folded = priceOf(selectedItem, g.ratio) as ModelPrice | null;
                      const active = g.name === selectedGroup;
                      return (
                        <button
                          key={g.name}
                          onClick={() => setSelectedGroup(g.name)}
                          aria-pressed={active}
                          title={benchTimings[g.name]}
                          className={`w-full rounded-xl border p-2 text-left transition [border-color:var(--nk-line)] ${
                            active ? "border-[var(--nk-accent)] bg-[var(--nk-accent)]/[0.06]" : "hover:bg-black/[0.03] dark:hover:bg-white/5"
                          }`}
                        >
                          <div className="flex min-w-0 items-center gap-1.5">
                            <span className="truncate font-mono text-[11px] font-semibold text-gray-900 dark:text-gray-100">{g.name}</span>
                            {!g.usable && (
                              <span className="shrink-0 rounded bg-black/[0.05] px-1 py-0.5 text-[9px] text-gray-400 dark:bg-white/10 dark:text-gray-500">
                                未开通
                              </span>
                            )}
                            <span className="ml-auto shrink-0 text-[10px] tabular-nums">
                              {benchmarks[g.name] === "loading" ? (
                                <span className={SUBTLE}>…</span>
                              ) : typeof benchmarks[g.name] === "number" ? (
                                <span
                                  className={
                                    (benchmarks[g.name] as number) <= 1500
                                      ? "text-green-600 dark:text-green-400"
                                      : (benchmarks[g.name] as number) <= 4000
                                        ? "text-amber-600 dark:text-amber-400"
                                        : "text-red-500 dark:text-red-400"
                                  }
                                  title="首字延迟中位数（3 次采样）"
                                >
                                  {benchmarks[g.name]}ms
                                </span>
                              ) : benchmarks[g.name] === null ? (
                                <span className={SUBTLE} title={benchErrors[g.name] || "测速失败"}>
                                  失败
                                </span>
                              ) : null}
                            </span>
                            <span className={`shrink-0 text-[10px] tabular-nums ${SUBTLE}`}>{g.ratio}x</span>
                          </div>
                          {g.desc && (
                            <p className={`mt-0.5 line-clamp-2 text-[10px] ${SUBTLE}`} title={g.desc}>
                              {g.desc}
                            </p>
                          )}
                          {benchTimings[g.name] && (
                            <p className={`mt-0.5 font-mono text-[9px] tabular-nums ${SUBTLE}`}>{benchTimings[g.name]}</p>
                          )}
                          <p className="mt-1 text-xs font-semibold tabular-nums text-gray-900 dark:text-gray-100">
                            {folded ? (
                              folded.perRequest ? (
                                <>
                                  {fmtUSD(folded.input)}
                                  <span className="ml-1 text-[10px] font-normal text-gray-500 dark:text-gray-400">/ 次调用</span>
                                </>
                              ) : (
                                <>
                                  {fmtUSD(folded.input)}
                                  <span className="ml-1 text-[10px] font-normal text-gray-500 dark:text-gray-400">输入 · 输出 {fmtUSD(folded.output)}</span>
                                </>
                              )
                            ) : (
                              <span className={`text-[11px] font-normal ${SUBTLE}`}>该分组暂无此模型价格</span>
                            )}
                          </p>
                        </button>
                      );
                    })}
                  </div>
                )}
              </div>
            </div>
          )}

          {/* 目录诊断：服务端到底下发了哪些字段，一眼可查（排查分组/厂商数据用） */}
          {!loading && !error && (
            <p className={`shrink-0 px-1 text-[10px] ${SUBTLE}`}>
              目录诊断：可用模型 {cards.length}（来源 {catalog.source}） · 定价 {data?.pricing?.length ?? 0} · 含令牌分组{" "}
              {cards.filter((c) => c.hasTokenGroups).length} · 账号分组 {groups.length} · 厂商表{" "}
              {(pricingMeta?.vendors ?? vendorMetas).length} · 分组说明{" "}
              {Object.keys(pricingMeta?.usableGroup ?? {}).length}
            </p>
          )}

          <p className={`shrink-0 px-1 text-[11px] ${SUBTLE}`}>
            模型卡片显示原价（官方基准，分组倍率 1x），不随分组变化；选中模型后在右侧分组列查看按分组倍率折算的实付价。
            价格条长度为该模型原价输入价在同厂家有价模型中的相对位置；
            标签规则：刚上新＝官方发布 14 天内，常用＝该厂家内用量前 3，性价比＝该厂家内用量前 5 且价格分位低于 1/5；
            「实测」来自首页 ⚡测速 的首字延迟中位数，未实测不显示。
          </p>
        </div>
      </main>
    </div>
  );
}

function ModelCard({
  card,
  selected,
  onSelect,
}: {
  card: Card & { level: number };
  selected: boolean;
  onSelect: () => void;
}) {
  const price = card.price;

  return (
    <button
      onClick={onSelect}
      aria-pressed={selected}
      className={`rounded-xl border p-3 text-left transition [border-color:var(--nk-line)] ${
        selected ? "border-[var(--nk-accent)] bg-[var(--nk-accent)]/[0.06]" : "hover:border-[var(--nk-accent)]"
      }`}
    >
      <div className="flex min-w-0 items-start justify-between gap-2">
        <div className="min-w-0">
          <p className="truncate font-mono text-sm font-semibold text-gray-900 dark:text-gray-100" title={card.name}>
            {card.name}
          </p>
          <div className="mt-1 flex flex-wrap items-center gap-1">
            {card.tags.map((tag) => (
              <span key={tag.id} className={`rounded-full px-1.5 py-0.5 text-[10px] ${tag.className}`}>
                {tag.label}
              </span>
            ))}
            {card.release && <span className={`text-[10px] tabular-nums ${SUBTLE}`}>{card.release} 发布</span>}
          </div>
          {/* 令牌分组：该模型支持的 API 分组（服务端 enable_groups） */}
          {card.groups.length > 0 ? (
            <div className="mt-1.5 flex flex-wrap items-center gap-1">
              <span className={`text-[10px] ${SUBTLE}`}>分组</span>
              {card.groups.slice(0, 4).map((g) => (
                <span
                  key={g.name}
                  title={`${g.desc || g.name}（${g.name}）${g.usable ? "" : " · 当前账号未开通"}`}
                  className={`rounded px-1 py-0.5 font-mono text-[10px] ${
                    g.usable
                      ? "bg-[var(--nk-info-soft)] text-[var(--nk-info)]"
                      : "bg-black/[0.04] text-gray-400 dark:bg-white/10 dark:text-gray-500"
                  }`}
                >
                  {g.name}
                </span>
              ))}
              {card.groups.length > 4 && (
                <span className={`text-[10px] ${SUBTLE}`}>+{card.groups.length - 4}</span>
              )}
            </div>
          ) : (
            <p className={`mt-1.5 text-[10px] ${SUBTLE}`}>
              {card.hasTokenGroups ? "分组目录缺少该模型的令牌分组说明" : "服务端未下发该模型的令牌分组"}
            </p>
          )}
        </div>
        <div className="flex shrink-0 flex-col items-end gap-1">
          {card.bench && (
            <span
              className={`rounded-full px-1.5 py-0.5 text-[10px] ${delayChip(card.bench.median).className}`}
              title={`实测首字延迟中位数 ${card.bench.median}ms（${card.bench.samples} 次）`}
            >
              {delayChip(card.bench.median).label} · {card.bench.median}ms
            </span>
          )}
        </div>
      </div>

      {price ? (
        price.perRequest ? (
          <p className="mt-2 text-lg font-semibold tabular-nums text-gray-900 dark:text-gray-100">
            {fmtUSD(price.input)}
            <span className="ml-1 text-[11px] font-normal text-gray-500 dark:text-gray-400">/ 次调用 · 原价</span>
          </p>
        ) : (
          <>
            <div className="mt-2 flex items-baseline gap-1.5">
              <span className="text-lg font-semibold tabular-nums text-gray-900 dark:text-gray-100">
                {fmtUSD(price.input)}
              </span>
              <span className="text-[11px] text-gray-500 dark:text-gray-400">
                输入 · 输出 {fmtUSD(price.output)} · 原价
              </span>
            </div>
            {/* 价格条：长度即全目录原价相对价位 */}
            <div className="mt-1.5 h-1 w-full rounded-full bg-black/[0.06] dark:bg-white/10">
              <div
                className="h-1 rounded-full bg-[var(--nk-accent)]"
                style={{ width: `${Math.max(6, card.level * 100).toFixed(0)}%` }}
              />
            </div>
          </>
        )
      ) : (
        <p className={`mt-2 text-xs ${SUBTLE}`}>暂无此模型价格</p>
      )}
    </button>
  );
}
