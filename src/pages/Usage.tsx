import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import {
  api,
  type UsageLogItem,
  type UsageSummary,
  type GroupOption,
  type BootstrapModel,
  type UsageDimension,
  type UsageDayBucket,
} from "../api/client";
import { summarizeNewApiLogs } from "../lib/newapi";
import { usageRange, usageTrend, UsageLoadError } from "../lib/usageLedger";
import { isNewApiAuth } from "../lib/station";
import { VENDORS, vendorOfGroup, vendorOfModel, type Vendor } from "../lib/vendor";
import { ArrowLeftIcon } from "../components/Icons";
import { friendlyDesktopError } from "../lib/copy";
import { fmtQuotaUSD } from "../lib/pricing";

const CARD = "nk-card";
const LABEL = "nk-label";
const TITLE = "nk-title";
const SELECT = "nk-select";
const PAGE_SIZE = 50;

const RANGES = [
  { id: "today", label: "今天" },
  { id: "7d", label: "近 7 天" },
  { id: "30d", label: "近 30 天" },
  { id: "all", label: "全部" },
] as const;

type RangeId = (typeof RANGES)[number]["id"] | "custom";

function modelName(item: BootstrapModel): string {
  return typeof item === "string" ? item : (item.name ?? item.model_name ?? item.id ?? "");
}

function num(n: number): string {
  return n.toLocaleString();
}

function dayStart(d: Date): number {
  const c = new Date(d);
  c.setHours(0, 0, 0, 0);
  return Math.floor(c.getTime() / 1000);
}

function toDateInput(ts: number): string {
  const d = new Date(ts * 1000);
  const m = `${d.getMonth() + 1}`.padStart(2, "0");
  const day = `${d.getDate()}`.padStart(2, "0");
  return `${d.getFullYear()}-${m}-${day}`;
}

// 卡片内嵌的迷你折线：按所选时间周期展示该指标的逐日变化
function Sparkline({
  buckets,
  pick,
  format,
}: {
  buckets: UsageDayBucket[];
  pick: (b: UsageDayBucket) => number;
  format: (v: number) => string;
}) {
  if (buckets.length === 0) return null;
  const values = buckets.map(pick);
  const peak = Math.max(...values);
  const max = Math.max(peak, 1);
  const y = (v: number) => (100 - (v / max) * 88 - 6).toFixed(2);
  const points =
    buckets.length === 1
      ? `0,${y(values[0])} 100,${y(values[0])}`
      : values.map((v, i) => `${((i / (values.length - 1)) * 100).toFixed(2)},${y(v)}`).join(" ");

  return (
    <div className="mt-3">
      <div className="relative h-10">
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-full w-full" aria-hidden="true">
          <polyline points={`0,100 ${points} 100,100`} fill="var(--nk-info-soft)" stroke="none" />
          <polyline
            points={points}
            fill="none"
            stroke="var(--nk-accent)"
            strokeWidth="1.5"
            strokeLinejoin="round"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
        <div className="absolute inset-0 flex">
          {buckets.map((b, i) => (
            <div
              key={b.date}
              title={`${b.date} · ${format(values[i])}`}
              className="flex-1 rounded transition hover:bg-black/5 dark:hover:bg-white/10"
            />
          ))}
        </div>
      </div>
      <div className="mt-1 flex justify-between text-[10px] text-gray-400 dark:text-gray-500">
        <span>{buckets[0].date.slice(5)}</span>
        <span>峰值 {format(peak)}</span>
        <span>{buckets[buckets.length - 1].date.slice(5)}</span>
      </div>
    </div>
  );
}

function DimensionList({ title, items, money }: { title: string; items: UsageDimension[]; money: (q: number) => string }) {
  if (items.length === 0) return null;
  const max = Math.max(...items.map((i) => i.quota), 1);
  return (
    <div className={CARD}>
      <p className={TITLE}>{title}</p>
      <ul className="mt-3 space-y-2">
        {items.slice(0, 8).map((item) => (
          <li key={item.name}>
            <div className="flex items-baseline justify-between gap-3 text-xs">
              <span className="truncate font-mono text-gray-700 dark:text-gray-300">{item.name}</span>
              <span className="shrink-0 text-gray-500 dark:text-gray-400">
                {num(item.requests)} 次 · {money(item.quota)}
              </span>
            </div>
            <div className="mt-1 h-1 rounded-full bg-black/5 dark:bg-white/10">
              <div
                className="h-1 rounded-full bg-indigo-500/70"
                style={{ width: `${Math.max(2, (item.quota / max) * 100)}%` }}
              />
            </div>
          </li>
        ))}
      </ul>
    </div>
  );
}

export default function Usage() {
  const auth = loadAuth();
  const navigate = useNavigate();
  const token = auth?.accessToken;
  const stationKindNewApi = isNewApiAuth(auth);

  const [groups, setGroups] = useState<GroupOption[]>([]);
  const [allModels, setAllModels] = useState<string[]>([]);

  const [range, setRange] = useState<RangeId>("7d");
  const [from, setFrom] = useState(toDateInput(dayStart(new Date()) - 6 * 86400));
  const [to, setTo] = useState(toDateInput(dayStart(new Date())));
  const [group, setGroup] = useState("");
  const [vendor, setVendor] = useState<Vendor | "">("");

  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [tableItems, setTableItems] = useState<UsageLogItem[]>([]);
  const [allRecords, setAllRecords] = useState<UsageLogItem[] | null>(null);
  const [tableMode, setTableMode] = useState<"server" | "local" | null>(null);
  const [total, setTotal] = useState(0);
  const [page, setPage] = useState(1);
  const [pageLoading, setPageLoading] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // 金额单位：本地已有就直接用；缺失时向站点补拉一次，仍缺失就不展示金额，
  // 避免用错误单位把消费算少一半。
  const [unit, setUnit] = useState<number | null>(() => {
    const parsed = Number(auth?.quotaPerUnit);
    return Number.isFinite(parsed) && parsed > 0 ? parsed : null;
  });
  const [unitError, setUnitError] = useState<string | null>(null);
  const money = useMemo(() => {
    if (unit === null) return (_: number) => "—";
    return (quota: number) => fmtQuotaUSD(quota, unit);
  }, [unit]);

  useEffect(() => {
    if (!token) {
      navigate("/login", { replace: true });
      return;
    }
    api
      .bootstrap(token)
      .then((data) => {
        setGroups(data.groups ?? []);
        setAllModels((data.models ?? []).map(modelName).filter(Boolean));
      })
      .catch(() => undefined);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!token || unit !== null) return;
    let active = true;
    api
      .status()
      .then((status) => {
        if (!active) return;
        const parsed = Number((status as { quota_per_unit?: number | string }).quota_per_unit);
        if (Number.isFinite(parsed) && parsed > 0) setUnit(parsed);
        else setUnitError("无法确认站点的金额单位，金额暂不展示。");
      })
      .catch(() => {
        if (active) setUnitError("金额单位暂时无法读取，金额暂不展示。");
      });
    return () => {
      active = false;
    };
  }, [token, unit]); // eslint-disable-line react-hooks/exhaustive-deps

  const rangeCalc = useMemo(() => {
    try {
      const [start, end] = usageRange(range === "custom" ? "custom" : range, from, to);
      return { start, end, error: null as string | null };
    } catch (err) {
      return {
        start: 0,
        end: 0,
        error: err instanceof UsageLoadError ? err.message : "所选时间范围无效。",
      };
    }
  }, [range, from, to]);
  const { start: startTimestamp, end: endTimestamp } = rangeCalc;

  // 厂商筛选转成模型名列表交给后端聚合；后端不支持时由本地完整账本兜底
  const vendorModels = useMemo(
    () => (vendor ? allModels.filter((m) => vendorOfModel(m) === vendor) : []),
    [vendor, allModels]
  );
  const vendorEmpty = vendor !== "" && vendorModels.length === 0;

  const tzOffset = -new Date().getTimezoneOffset();
  const queryKey = `${token ?? ""}|${startTimestamp}|${endTimestamp}|${group}|${vendor}|${vendorModels.join(",")}`;
  const versionRef = useRef(0);
  const firstPageKeyRef = useRef<string>("");
  // 当前筛选的账本是否已落地：翻页只能在账本就绪后进行，
  // 否则切换筛选瞬间会带着上一轮的分页模式多发请求、互相覆盖
  const ledgerKeyRef = useRef<string>("");

  // 主加载：换筛选就整体作废上一轮结果，旧请求晚到也不允许覆盖新选择；
  // 失败时清空数据，绝不把上一个范围的数字当成当前范围展示。
  useEffect(() => {
    if (!token || rangeCalc.error) return;
    const version = ++versionRef.current;
    const controller = new AbortController();
    setLoading(true);
    setError(null);
    ledgerKeyRef.current = "";
    setSummary(null);
    setTableItems([]);
    setAllRecords(null);
    setTableMode(null);
    setTotal(0);
    setPage(1);
    firstPageKeyRef.current = "";

    if (vendorEmpty) {
      setSummary(summarizeNewApiLogs([]));
      setAllRecords([]);
      setTableMode("local");
      ledgerKeyRef.current = queryKey;
      setLoading(false);
      return;
    }

    const query = {
      startTimestamp: startTimestamp || undefined,
      endTimestamp: endTimestamp || undefined,
      group: group || undefined,
      models: vendor ? vendorModels : undefined,
      tz: tzOffset,
    };
    // 第三方没有服务端汇总；本站选了厂商时旧服务端不认 models，也走本地完整账本，
    // 保证概览和明细永远来自同一批记录。
    const useLocalLedger = stationKindNewApi || vendor !== "";
    (async () => {
      try {
        if (useLocalLedger) {
          const records = await api.usageRecords(token, query, controller.signal);
          if (versionRef.current !== version) return;
          setSummary(summarizeNewApiLogs(records));
          setAllRecords(records);
          setTableMode("local");
          setTotal(records.length);
        } else {
          const [summaryData, firstPage] = await Promise.all([
            api.usageSummary(token, query),
            api.usage(token, { ...query, page: 1, pageSize: PAGE_SIZE }),
          ]);
          if (versionRef.current !== version) return;
          setSummary(summaryData);
          setTableMode("server");
          const pageTotal = firstPage.total ?? firstPage.items?.length ?? 0;
          setTotal(pageTotal);
          setTableItems(firstPage.items ?? []);
          firstPageKeyRef.current = queryKey;
        }
        ledgerKeyRef.current = queryKey;
      } catch (err) {
        if (versionRef.current !== version || controller.signal.aborted) return;
        setError(err instanceof UsageLoadError ? err.message : friendlyDesktopError(err));
        setSummary(null);
        setTableItems([]);
        setAllRecords(null);
        setTableMode(null);
        setTotal(0);
      } finally {
        if (versionRef.current === version) setLoading(false);
      }
    })();
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queryKey, rangeCalc.error, vendorEmpty]);

  // 翻页：本地账本直接切片；服务端分页按页取，且同样不允许旧页覆盖新页。
  useEffect(() => {
    if (!token || rangeCalc.error || ledgerKeyRef.current !== queryKey || tableMode === null) return;
    if (tableMode === "local") {
      setTableItems(allRecords?.slice((page - 1) * PAGE_SIZE, page * PAGE_SIZE) ?? []);
      return;
    }
    if (page === 1 && firstPageKeyRef.current === queryKey) return;
    const version = versionRef.current;
    setPageLoading(true);
    api
      .usage(token, {
        startTimestamp: startTimestamp || undefined,
        endTimestamp: endTimestamp || undefined,
        group: group || undefined,
        models: vendor ? vendorModels : undefined,
        tz: tzOffset,
        page,
        pageSize: PAGE_SIZE,
      })
      .then((result) => {
        if (versionRef.current !== version) return;
        setTableItems(result.items ?? []);
      })
      .catch((err) => {
        if (versionRef.current !== version) return;
        setTableItems([]);
        setError(err instanceof UsageLoadError ? err.message : friendlyDesktopError(err));
      })
      .finally(() => {
        if (versionRef.current === version) setPageLoading(false);
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, queryKey, tableMode, allRecords]);

  const groupOptions = useMemo(() => {
    const list = vendor ? groups.filter((g) => vendorOfGroup(g.name) === vendor) : groups;
    return list.map((g) => g.name);
  }, [groups, vendor]);

  const tokensTotal = (summary?.prompt_tokens ?? 0) + (summary?.completion_tokens ?? 0);
  const requests = summary?.requests ?? 0;
  const avgCost = requests > 0 ? money(summary!.quota / requests) : "—";
  const streamRate = requests > 0 ? `${Math.round((summary!.stream_requests / requests) * 100)}%` : "—";

  // 折线图按所选周期补齐没有消费的日期，避免时间轴被压缩
  const trendBuckets = useMemo(
    () => usageTrend(summary?.by_day ?? [], startTimestamp, endTimestamp),
    [summary, startTimestamp, endTimestamp]
  );

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));

  return (
    <div className="nk-shell">
      <header className="nk-header">
        <button
          onClick={() => navigate("/home")}
          className="nk-btn-ghost px-2.5"
          aria-label="返回首页"
        >
          <ArrowLeftIcon />
        </button>
        <h1 className={TITLE}>使用明细</h1>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-5xl space-y-3">
          <div className={CARD}>
            <div className="flex flex-wrap items-center gap-2">
              {RANGES.map((r) => (
                <button
                  key={r.id}
                  onClick={() => setRange(r.id)}
                  className={`nk-btn ${
                    range === r.id
                      ? "nk-btn-primary"
                      : "nk-btn-secondary"
                  }`}
                >
                  {r.label}
                </button>
              ))}
              <button
                onClick={() => setRange("custom")}
                className={`nk-btn ${
                  range === "custom"
                    ? "nk-btn-primary"
                    : "nk-btn-secondary"
                }`}
              >
                自定义
              </button>
            </div>

            {range === "custom" && (
              <div className="mt-3 flex flex-wrap items-center gap-2 text-xs">
                <input
                  type="date"
                  value={from}
                  onChange={(e) => setFrom(e.target.value)}
                  aria-label="开始日期"
                  className={SELECT}
                />
                <span className="text-gray-400">至</span>
                <input
                  type="date"
                  value={to}
                  onChange={(e) => setTo(e.target.value)}
                  aria-label="结束日期"
                  className={SELECT}
                />
              </div>
            )}

            <div className="mt-3 flex flex-wrap items-center gap-2">
              <select
                value={vendor}
                onChange={(e) => {
                  setVendor(e.target.value as Vendor | "");
                  setGroup("");
                }}
                aria-label="按模型厂商筛选"
                className={SELECT}
              >
                <option value="">全部模型厂商</option>
                {VENDORS.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
              <select
                value={group}
                onChange={(e) => setGroup(e.target.value)}
                aria-label="按模型服务筛选"
                className={SELECT}
              >
                <option value="">全部模型服务</option>
                {groupOptions.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {rangeCalc.error && <p className="nk-alert-danger text-center text-sm">{rangeCalc.error}</p>}
          {error && <p className="nk-alert-danger text-center text-sm">{error}</p>}
          {unitError && <p className="nk-alert-danger text-center text-sm">{unitError}</p>}
          {loading && (
            <div
              role="status"
              aria-live="polite"
              className="flex items-center justify-center gap-2 py-6"
            >
              <span className="nk-spinner" aria-hidden="true" />
                <span className="nk-muted">正在加载使用明细…</span>
            </div>
          )}

          {!loading && summary && (
            <>
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                <div className={CARD}>
                  <p className={LABEL}>累计花费</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {money(summary.quota)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">平均每次 {avgCost}</p>
                  <Sparkline buckets={trendBuckets} pick={(b) => b.quota} format={money} />
                </div>
                <div className={CARD}>
                  <p className={LABEL}>累计文字量</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {num(tokensTotal)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    你发出的内容 {num(summary.prompt_tokens)} · AI 回复的内容 {num(summary.completion_tokens)}
                  </p>
                  <p className="mt-1 text-[11px] text-gray-500 dark:text-gray-400">
                    token 是模型计算文字量的单位
                  </p>
                  <Sparkline buckets={trendBuckets} pick={(b) => b.tokens} format={num} />
                </div>
                <div className={CARD}>
                  <p className={LABEL}>使用次数</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {num(requests)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">连续输出占比 {streamRate}</p>
                  <Sparkline buckets={trendBuckets} pick={(b) => b.requests} format={num} />
                </div>
              </div>

              <div className="grid gap-3 sm:grid-cols-2">
                <DimensionList title="模型使用排行" items={summary.by_model ?? []} money={money} />
                <DimensionList title="模型服务使用排行" items={summary.by_group ?? []} money={money} />
              </div>
            </>
          )}

          {!loading && !error && !rangeCalc.error && (
            <div className={CARD}>
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className={TITLE}>使用明细</p>
                <p className="text-xs text-gray-500 dark:text-gray-400">共 {num(total)} 条记录</p>
              </div>
              {/* Claude Code / Codex 是 agent，一次提问内部会分成读文件、调工具、生成标题等多次
                  独立请求，条数远多于用户感知的对话轮数，不说明会被当成重复计费 */}
              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                一次提问通常对应多条记录：Claude Code 与 Codex 会在后台拆成读文件、调用工具等多次模型调用，各自单独计费。
              </p>
              {tableItems.length === 0 ? (
                <p className="nk-empty mt-3">
                  当前筛选条件下暂无使用记录
                </p>
              ) : (
                <div className="nk-table-wrap">
                <table className="nk-table">
                  <thead>
                    <tr>
                      <th>时间</th>
                      <th>模型</th>
                      <th>模型服务</th>
                      <th className="text-right">你发出的内容</th>
                      <th className="text-right">AI 回复的内容</th>
                      <th className="text-right">花费</th>
                    </tr>
                  </thead>
                  <tbody>
                    {tableItems.map((l, i) => (
                      <tr key={`${l.created_at}-${l.id}-${i}`}>
                        <td className="whitespace-nowrap text-gray-500 dark:text-gray-400">
                          {new Date(l.created_at * 1000).toLocaleString("zh-CN", {
                            month: "2-digit",
                            day: "2-digit",
                            hour: "2-digit",
                            minute: "2-digit",
                            second: "2-digit",
                            hour12: false,
                          })}
                        </td>
                        <td className="font-mono">{l.model_name}</td>
                        <td className="text-gray-500 dark:text-gray-400">{l.group || "—"}</td>
                        <td className="text-right">{num(l.prompt_tokens)}</td>
                        <td className="text-right">{num(l.completion_tokens)}</td>
                        <td className="text-right font-semibold text-indigo-600 dark:text-indigo-400">
                          {money(l.quota)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
                </div>
              )}
              <div className="mt-3 flex items-center justify-between gap-2 text-xs text-gray-500 dark:text-gray-400">
                <span>
                  第 {page} / {totalPages} 页
                </span>
                <div className="flex items-center gap-2">
                  <button
                    onClick={() => setPage((p) => Math.max(1, p - 1))}
                    disabled={page <= 1 || pageLoading}
                    className="nk-btn nk-btn-secondary"
                  >
                    上一页
                  </button>
                  <button
                    onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                    disabled={page >= totalPages || pageLoading}
                    className="nk-btn nk-btn-secondary"
                  >
                    下一页
                  </button>
                </div>
              </div>
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
