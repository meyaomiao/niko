// 模型与价格：按 厂家 → 模型 两层浏览的目录页
// 价格换算与网页端同一套公式（src/lib/pricing.ts）：
//   输入价（USD / 百万 token）= model_ratio × 2 × 分组倍率，输出价再乘 completion_ratio
// 计价分组只影响倍率，做成次要的小切换；实测延迟来自首页「⚡测速」的本地缓存。

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type ModelMetadata } from "../api/client";
import { buildPricingIndex, fmtUSD, priceOf, type ModelPrice } from "../lib/pricing";
import { VENDORS, vendorOfModel } from "../lib/vendor";
import { computeTags, topUsedModels, type ModelTag } from "../lib/modelTags";
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
function orderedModels(data: BootstrapData): string[] {
  const names = (data.models ?? []).map(modelName).filter(Boolean);
  const unique = Array.from(new Set(names));
  if (!data.model_order?.length) return unique.sort();
  const index = new Map(data.model_order.map((name, i) => [name, i]));
  return unique.sort((a, b) => {
    const ia = index.get(a);
    const ib = index.get(b);
    if (ia !== undefined && ib !== undefined) return ia - ib;
    if (ia !== undefined) return -1;
    if (ib !== undefined) return 1;
    return a.localeCompare(b);
  });
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
  const [topUsed, setTopUsed] = useState<Set<string>>(new Set());

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
        // 用户自己的用量聚合 → 「常用」标签；失败静默，不阻塞目录
        api
          .usageSummary(token)
          .then((s) => setTopUsed(topUsedModels(s)))
          .catch(() => undefined);
      })
      .catch((e) => setError(friendlyDesktopError(e)))
      .finally(() => setLoading(false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const groups: GroupOption[] = data?.groups ?? [];
  const ratio = groups.find((g) => g.name === group)?.ratio ?? 0;
  const pricingIndex = useMemo(() => buildPricingIndex(data?.pricing), [data]);

  const cards = useMemo(() => {
    if (!data) return [];
    const rows = orderedModels(data).map((name) => ({
      name,
      release: releaseDate(data.model_metadata?.[name], pricingIndex.get(name)?.release_date),
      price: priceOf(pricingIndex.get(name), ratio) as ModelPrice | null,
      level: 0.5,
      bench: bench[name],
    }));
    // 价位分位在全目录内统一计算，价格条才有可比性
    const inputs = rows
      .filter((r) => r.price && !r.price.perRequest)
      .map((r) => r.price!.input);
    const min = Math.min(...inputs, Infinity);
    const max = Math.max(...inputs, 0);
    const span = max > min ? max - min : 1;
    return rows.map((r) => ({
      ...r,
      level:
        r.price && !r.price.perRequest && Number.isFinite(min)
          ? Math.min(1, Math.max(0, (r.price.input - min) / span))
          : 0.5,
      tags: computeTags({
        name: r.name,
        release: r.release,
        level: r.price && !r.price.perRequest && Number.isFinite(min)
          ? Math.min(1, Math.max(0, (r.price.input - min) / span))
          : undefined,
        topUsed,
      }),
    }));
  }, [data, pricingIndex, ratio, bench, topUsed]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return cards.filter((c) => c.name.toLowerCase().includes(q));
  }, [cards, query]);

  const sections = VENDORS.map((vendor) => ({
    vendor,
    cards: filtered.filter((c) => vendorOfModel(c.name) === vendor),
  })).filter((s) => s.cards.length > 0);

  const total = sections.reduce((sum, s) => sum + s.cards.length, 0);

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

          <p className={`px-1 text-[11px] ${SUBTLE}`}>
            价格 = 官方基准 × 分组倍率（当前 {group || "—"}：{ratio || "—"}x）。价格条长度为该模型输入价在全部有价模型中的相对位置；
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
