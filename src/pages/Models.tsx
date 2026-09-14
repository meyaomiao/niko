// 模型与价格：完整展示服务端目录里提供的模型与对应价格
// 价格换算公式与 src/lib/pricing.ts（即网页端）一致：
//   输入价（USD / 百万 token）= model_ratio × 2 × 分组倍率，输出价再乘 completion_ratio
// 展示顺序遵循服务端 model_order（发布顺序），按厂商分区浏览；
// 价格高低用区块内的相对价格条可视化；实测延迟来自首页「⚡测速」的结果缓存。

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type ModelMetadata } from "../api/client";
import { buildPricingIndex, fmtUSD, priceOf, type ModelPrice } from "../lib/pricing";
import { VENDORS, vendorOfModel } from "../lib/vendor";
import { ArrowLeftIcon } from "../components/Icons";
import { friendlyDesktopError } from "../lib/copy";

const CARD = "nk-card";
const SUBTLE = "nk-muted";
const TITLE = "nk-title";
const SELECT = "nk-select";

const BENCH_KEY = "niko_model_bench";

type BenchEntry = { median: number; samples: number; at: number };

/** 首页测速落地的缓存：model -> 最近一次实测 */
export function loadBenchCache(): Record<string, BenchEntry> {
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

/** 价格档：按输入价在「本页有价模型」中的分位划三档 */
function priceTier(ratio: number): { label: string; className: string } {
  if (ratio <= 1 / 3) return { label: "低价", className: "bg-green-500/10 text-green-700 dark:text-green-400" };
  if (ratio <= 2 / 3) return { label: "中档", className: "bg-amber-500/10 text-amber-700 dark:text-amber-400" };
  return { label: "高价", className: "bg-red-500/10 text-red-600 dark:text-red-400" };
}

function delayTier(ms: number): { label: string; className: string } {
  if (ms <= 3000) return { label: "快", className: "bg-green-500/10 text-green-700 dark:text-green-400" };
  if (ms <= 8000) return { label: "正常", className: "bg-amber-500/10 text-amber-700 dark:text-amber-400" };
  return { label: "偏慢", className: "bg-red-500/10 text-red-600 dark:text-red-400" };
}

interface Row {
  name: string;
  release: string;
  price: ModelPrice | null;
  /** 输入价（按次计费为每次价格），用于归一化价格条 */
  base: number;
  bench?: BenchEntry;
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
      })
      .catch((e) => setError(friendlyDesktopError(e)))
      .finally(() => setLoading(false));
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const groups: GroupOption[] = data?.groups ?? [];
  const ratio = groups.find((g) => g.name === group)?.ratio ?? 0;
  const pricingIndex = useMemo(() => buildPricingIndex(data?.pricing), [data]);

  // 全量行（不筛搜索词），价格条的分位要在同组内统一计算
  const allRows = useMemo(() => {
    if (!data) return [];
    return orderedModels(data).map((name) => ({
      name,
      release: releaseDate(data.model_metadata?.[name], pricingIndex.get(name)?.release_date),
      price: priceOf(pricingIndex.get(name), ratio) as ModelPrice | null,
      base: 0,
      bench: bench[name],
    }));
  }, [data, pricingIndex, ratio, bench]);

  const priced = allRows.filter((r) => r.price && !r.price.perRequest) as (Row & { price: ModelPrice })[];
  const minInput = Math.min(...priced.map((r) => r.price.input), Infinity);
  const maxInput = Math.max(...priced.map((r) => r.price.input), 0);
  const span = maxInput > minInput ? maxInput - minInput : 1;

  const rowsWithMeta = useMemo(
    () =>
      allRows.map((row) => {
        if (!row.price || row.price.perRequest) return { ...row, tierRatio: 1 };
        const ratio = maxInput > 0 ? (row.price.input - minInput) / span : 0.5;
        return { ...row, tierRatio: Math.min(1, Math.max(0, ratio)) };
      }),
    [allRows, minInput, maxInput, span]
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rowsWithMeta.filter((r) => r.name.toLowerCase().includes(q));
  }, [rowsWithMeta, query]);

  const sections = VENDORS.map((vendor) => ({
    vendor,
    rows: filtered.filter((r) => vendorOfModel(r.name) === vendor),
  })).filter((s) => s.rows.length > 0);

  return (
    <div className="nk-shell">
      <header className="nk-header">
        <button onClick={() => navigate("/home")} aria-label="返回首页" className="nk-btn-ghost px-2.5">
          <ArrowLeftIcon />
        </button>
        <h1 className={TITLE}>模型与价格</h1>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-3xl space-y-3">
          {/* 工具条：分组 + 搜索 */}
          <div className={`${CARD} flex flex-wrap items-center gap-2`}>
            <label className={`flex items-center gap-2 text-xs ${SUBTLE}`}>
              计价分组
              <select
                value={group}
                onChange={(e) => setGroup(e.target.value)}
                className={`${SELECT} max-w-48`}
                aria-label="选择计价分组"
              >
                {groups.map((g) => (
                  <option key={g.name} value={g.name}>
                    {g.name}（{g.ratio}x）
                  </option>
                ))}
              </select>
            </label>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="搜索模型"
              aria-label="搜索模型"
              className="nk-input ml-auto max-w-44 flex-1 py-1 text-xs"
            />
          </div>

          {loading && <p className={`${CARD} ${SUBTLE}`}>正在加载模型目录…</p>}
          {error && <p className={`${CARD} text-red-500`}>{error}</p>}

          {!loading && !error && sections.map(({ vendor, rows }) => (
            <section key={vendor} className={CARD}>
              <div className="flex items-baseline justify-between">
                <h2 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{vendor}</h2>
                <span className={`text-[11px] ${SUBTLE}`}>{rows.length} 个模型</span>
              </div>
              <div className="mt-2 space-y-1.5">
                {rows.map((row) => (
                  <ModelRow key={row.name} row={row} />
                ))}
              </div>
            </section>
          ))}

          {!loading && !error && sections.length === 0 && (
            <p className={`${CARD} ${SUBTLE}`}>没有匹配的模型。</p>
          )}

          <p className={`px-1 text-[11px] ${SUBTLE}`}>
            价格 = 官方基准 × 分组倍率（当前 {group || "—"}：{ratio || "—"}x）。价位档与价格条按本页有价模型的相对位置计算；
            「实测」来自首页 ⚡测速 的首字延迟中位数，未实测的模型不显示。
          </p>
        </div>
      </main>
    </div>
  );
}

function ModelRow({ row }: { row: Row & { tierRatio: number } }) {
  const [open, setOpen] = useState(false);
  const price = row.price;

  return (
    <div className="rounded-lg border px-3 py-2 [border-color:var(--nk-line)]">
      <button
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="flex w-full min-w-0 items-center gap-3 text-left"
      >
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <span className="truncate font-mono text-xs font-semibold text-gray-900 dark:text-gray-100">
              {row.name}
            </span>
            {row.release && (
              <span className={`shrink-0 text-[10px] tabular-nums ${SUBTLE}`}>{row.release}</span>
            )}
          </div>
          {/* 价格条：长度即本页相对价位 */}
          {price && !price.perRequest && (
            <div className="mt-1.5 h-1 w-full max-w-56 rounded-full bg-black/[0.06] dark:bg-white/10">
              <div
                className="h-1 rounded-full bg-[var(--nk-accent)]"
                style={{ width: `${Math.max(6, row.tierRatio * 100).toFixed(0)}%` }}
              />
            </div>
          )}
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {price && !price.perRequest && (
            <>
              <span
                className={`hidden rounded-full px-1.5 py-0.5 text-[10px] sm:inline ${priceTier(row.tierRatio).className}`}
              >
                {priceTier(row.tierRatio).label}
              </span>
              <span className="text-right tabular-nums text-xs">
                <span className="font-medium text-gray-900 dark:text-gray-100">{fmtUSD(price.input)}</span>
                <span className={SUBTLE}> / {fmtUSD(price.output)}</span>
              </span>
            </>
          )}
          {price?.perRequest && (
            <span className="rounded-full bg-blue-500/10 px-1.5 py-0.5 text-[10px] text-blue-700 dark:text-blue-400">
              {fmtUSD(price.input)} / 次
            </span>
          )}
          {row.bench && (
            <span
              className={`hidden rounded-full px-1.5 py-0.5 text-[10px] md:inline ${delayTier(row.bench.median).className}`}
              title={`实测首字延迟中位数 ${row.bench.median}ms`}
            >
              {delayTier(row.bench.median).label}
            </span>
          )}
          <span className={`w-3 text-center text-[10px] ${SUBTLE}`} aria-hidden="true">
            {open ? "▾" : "▸"}
          </span>
        </div>
      </button>

      {open && (
        <div className="mt-2 grid grid-cols-2 gap-2 border-t pt-2 text-[11px] sm:grid-cols-4 [border-color:var(--nk-line)]">
          <Detail label="输入 / 百万" value={price && !price.perRequest ? fmtUSD(price.input) : "—"} />
          <Detail label="输出 / 百万" value={price && !price.perRequest ? fmtUSD(price.output) : "—"} />
          <Detail label="缓存命中 / 百万" value={price?.cache !== undefined ? fmtUSD(price.cache) : "—"} />
          <Detail label="缓存写入 / 百万" value={price?.createCache !== undefined ? fmtUSD(price.createCache) : "—"} />
          <Detail
            label="实测首字延迟"
            value={row.bench ? `${row.bench.median}ms（${row.bench.samples} 次中位）` : "未实测"}
          />
          <Detail label="发布日期" value={row.release || "—"} />
        </div>
      )}
    </div>
  );
}

function Detail({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <p className={SUBTLE}>{label}</p>
      <p className="mt-0.5 tabular-nums text-gray-800 dark:text-gray-200">{value}</p>
    </div>
  );
}
