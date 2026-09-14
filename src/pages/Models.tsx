// 模型与价格：按 厂家 → 模型 两层浏览的目录页
// 价格换算与网页端同一套公式（src/lib/pricing.ts）：
//   输入价（USD / 百万 token）= model_ratio × 2 × 分组倍率，输出价再乘 completion_ratio
// 计价分组只影响倍率，做成次要的小切换；实测延迟来自首页「⚡测速」的本地缓存。

import { useEffect, useMemo, useState, useCallback } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type ModelMetadata, type PricingMeta, type UsageSummary, type VendorMeta } from "../api/client";
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

/** 服务端目录顺序：model_order 优先，其余按名称稳定兜底（与 modelSelection 的规则一致） */
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
  price: ModelPrice | null;
  /** 相对价位 0~1（按本页有价模型的输入价分位），仅用于价格条长度 */
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
  const token = loadAuth()?.accessToken;

  const [data, setData] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [group, setGroup] = useState("");
  const [query, setQuery] = useState("");
  const [bench, setBench] = useState<Record<string, BenchEntry>>({});
  const [usage, setUsage] = useState<UsageSummary | null>(null);
  const [vendorMetas] = useState<VendorMeta[]>([]);
  const [pricingMeta, setPricingMeta] = useState<PricingMeta | null>(null);

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
        setGroup(res.user?.group || res.groups?.[0]?.name || "");
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

  const groups: GroupOption[] = data?.groups ?? [];
  const ratio = groups.find((g) => g.name === group)?.ratio ?? 0;
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
    const priceByName = new Map<string, ModelPrice | null>();
    for (const name of names) {
      priceByName.set(name, priceOf(pricingIndex.get(name), ratio) as ModelPrice | null);
    }
    // 价格分位与用量名次都按「厂家内」比较，价格条与标签口径一致
    const levels = vendorPriceLevels(names, (name) => {
      const price = priceByName.get(name);
      return price && !price.perRequest ? price.input : undefined;
    });
    const ranks = vendorUsageRanks(usage);

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
  }, [data, pricingIndex, ratio, bench, usage, groupCatalog, groups]);

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
    for (const c of filtered) {
      const v = vendorOfName(c.name);
      map.set(v, (map.get(v) ?? 0) + 1);
    }
    return map;
  }, [filtered, vendorOfName]);

  // 厂家切换：全部=分区铺开；选中某家=只看该家；切换条按模型数量降序
  const [activeVendor, setActiveVendor] = useState<string>("全部");
  const orderedVendors = useMemo(
    () =>
      vendorOrder
        .filter((vendor) => (counts.get(vendor) ?? 0) > 0)
        .sort((a, b) => (counts.get(b) ?? 0) - (counts.get(a) ?? 0) || a.localeCompare(b)),
    [vendorOrder, counts]
  );
  const visible = (activeVendor === "全部" ? orderedVendors : [activeVendor]).map((vendor) => ({
    vendor,
    cards: filtered.filter((c) => vendorOfName(c.name) === vendor),
  }));
  const sections = visible.filter((s) => s.cards.length > 0);

  const total = filtered.length;

  return (
    <div className="nk-shell">
      <header className="nk-header">
        <button onClick={() => navigate("/home")} aria-label="返回首页" className="nk-btn-ghost px-2.5">
          <ArrowLeftIcon />
        </button>
        <h1 className={TITLE}>模型与价格</h1>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-4xl space-y-3">
          {/* 搜索 + 分组倍率：分组只是计价口径，做成次要的小胶囊切换 */}
          <div className={`${CARD} flex flex-wrap items-center gap-2`}>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={`搜索 ${total} 个模型…`}
              aria-label="搜索模型"
              className="nk-input min-w-40 flex-1 py-1 text-xs"
            />
            <div className="flex flex-wrap items-center gap-1" role="group" aria-label="计价分组">
              {groups.map((g) => (
                <button
                  key={g.name}
                  onClick={() => setGroup(g.name)}
                  aria-pressed={g.name === group}
                  title={`按 ${g.name} 的倍率（${g.ratio}x）计价`}
                  className={`rounded-full border px-2 py-0.5 text-[11px] transition ${
                    g.name === group
                      ? "border-transparent bg-[var(--nk-accent)] font-medium text-white"
                      : "[border-color:var(--nk-line)] text-gray-500 hover:bg-black/[0.04] dark:text-gray-400 dark:hover:bg-white/10"
                  }`}
                >
                  {g.name}
                </button>
              ))}
            </div>
          </div>

          {/* 厂家切换：sticky 吸顶，切走长滚动 */}
          {!loading && !error && (
            <div className="sticky top-0 z-10 -mx-1 flex items-center gap-1 overflow-x-auto rounded-xl border bg-[var(--nk-surface)] px-1 py-1 [border-color:var(--nk-line)]">
              {["全部", ...orderedVendors].map((vendor) => (
                <button
                  key={vendor}
                  onClick={() => setActiveVendor(vendor)}
                  aria-pressed={activeVendor === vendor}
                  className={`flex shrink-0 items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs transition ${
                    activeVendor === vendor
                      ? "bg-[var(--nk-accent)] font-medium text-white"
                      : "text-gray-600 hover:bg-black/[0.04] dark:text-gray-300 dark:hover:bg-white/10"
                  }`}
                >
                  {vendor !== "全部" && <VendorIcon vendor={vendor} />}
                  {vendor}
                  <span className="tabular-nums opacity-60">
                    {vendor === "全部" ? total : counts.get(vendor) ?? 0}
                  </span>
                </button>
              ))}
            </div>
          )}

          {loading && <p className={`${CARD} ${SUBTLE}`}>正在加载模型目录…</p>}
          {error && <p className={`${CARD} text-red-500`}>{error}</p>}

          {!loading && !error && sections.map(({ vendor, cards: list }) => (
            <section key={vendor} className={CARD}>
              <div className="flex items-baseline justify-between">
                <h2 className="flex items-center gap-2 text-sm font-semibold text-gray-900 dark:text-gray-100">
                  <VendorIcon vendor={vendor} />
                  {vendor}
                </h2>
                <span className={`text-[11px] ${SUBTLE}`}>{list.length} 个模型</span>
              </div>
              <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
                {list.map((card) => (
                  <ModelCard key={card.name} card={card} />
                ))}
              </div>
            </section>
          ))}

          {!loading && !error && sections.length === 0 && (
            <p className={`${CARD} ${SUBTLE}`}>没有匹配的模型。</p>
          )}

          {/* 目录诊断：服务端到底下发了哪些字段，一眼可查（排查分组/厂商数据用） */}
          {!loading && !error && (
            <p className={`px-1 text-[10px] ${SUBTLE}`}>
              目录诊断：可用模型 {cards.length}（来源 {catalog.source}） · 定价 {data?.pricing?.length ?? 0} · 含令牌分组{" "}
              {cards.filter((c) => c.hasTokenGroups).length} · 账号分组 {groups.length} · 厂商表{" "}
              {(pricingMeta?.vendors ?? vendorMetas).length} · 分组说明{" "}
              {Object.keys(pricingMeta?.usableGroup ?? {}).length}
            </p>
          )}

          <p className={`px-1 text-[11px] ${SUBTLE}`}>
            价格 = 官方基准 × 分组倍率（当前 {group || "—"}：{ratio || "—"}x）。价格条长度为该模型输入价在同厂家有价模型中的相对位置；
            标签规则：刚上新＝官方发布 14 天内，常用＝该厂家内用量前 3，性价比＝该厂家内用量前 5 且价格分位低于 1/5；
            「实测」来自首页 ⚡测速 的首字延迟中位数，未实测不显示。
          </p>
        </div>
      </main>
    </div>
  );
}

function ModelCard({ card }: { card: Card & { level: number } }) {
  const price = card.price;

  return (
    <div className="rounded-xl border p-3 transition hover:border-[var(--nk-accent)] [border-color:var(--nk-line)]">
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
            <span className="ml-1 text-[11px] font-normal text-gray-500 dark:text-gray-400">/ 次调用</span>
          </p>
        ) : (
          <>
            <div className="mt-2 flex items-baseline gap-1.5">
              <span className="text-lg font-semibold tabular-nums text-gray-900 dark:text-gray-100">
                {fmtUSD(price.input)}
              </span>
              <span className="text-[11px] text-gray-500 dark:text-gray-400">
                输入 · 输出 {fmtUSD(price.output)}
              </span>
            </div>
            {/* 价格条：长度即全目录相对价位 */}
            <div className="mt-1.5 h-1 w-full rounded-full bg-black/[0.06] dark:bg-white/10">
              <div
                className="h-1 rounded-full bg-[var(--nk-accent)]"
                style={{ width: `${Math.max(6, card.level * 100).toFixed(0)}%` }}
              />
            </div>
          </>
        )
      ) : (
        <p className={`mt-2 text-xs ${SUBTLE}`}>该分组暂无此模型价格</p>
      )}
    </div>
  );
}
