import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import {
  api,
  type UsageLogItem,
  type UsageSummary,
  type GroupOption,
  type UsageDimension,
  type UsageDayBucket,
} from "../api/client";
import { VENDORS, vendorOfGroup, vendorOfModel, type Vendor } from "../lib/vendor";

const CARD = "rounded-2xl border border-black/5 bg-white p-4 shadow-sm dark:border-white/10 dark:bg-white/5";
const LABEL = "text-xs font-medium text-gray-500 dark:text-gray-400";
const TITLE = "text-sm font-semibold text-gray-900 dark:text-gray-100";
const SELECT =
  "rounded-full border border-black/10 bg-white px-3 py-1.5 text-xs text-gray-700 dark:border-white/15 dark:bg-white/5 dark:text-gray-200";

const RANGES = [
  { id: "today", label: "今天", days: 0 },
  { id: "7d", label: "近 7 天", days: 7 },
  { id: "30d", label: "近 30 天", days: 30 },
  { id: "all", label: "全部", days: -1 },
] as const;

type RangeId = (typeof RANGES)[number]["id"] | "custom";

function usd(quota: number): string {
  const v = quota / 1_000_000;
  return v >= 1 ? `$${v.toFixed(2)}` : `$${v.toFixed(4)}`;
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

function rangeToTimestamps(range: RangeId, from: string, to: string): [number, number] {
  const now = new Date();
  if (range === "custom") {
    const start = from ? dayStart(new Date(`${from}T00:00:00`)) : 0;
    const end = to ? dayStart(new Date(`${to}T00:00:00`)) + 86399 : 0;
    return [start, end];
  }
  const conf = RANGES.find((r) => r.id === range);
  if (!conf || conf.days < 0) return [0, 0];
  if (conf.days === 0) return [dayStart(now), dayStart(now) + 86399];
  const start = dayStart(new Date(now.getTime() - (conf.days - 1) * 86400_000));
  return [start, dayStart(now) + 86399];
}

const METRICS = [
  { id: "quota", label: "消费" },
  { id: "tokens", label: "token" },
  { id: "requests", label: "请求" },
] as const;

type MetricId = (typeof METRICS)[number]["id"];

function formatMetric(id: MetricId, v: number): string {
  return id === "quota" ? usd(v) : num(v);
}

function TrendChart({ buckets }: { buckets: UsageDayBucket[] }) {
  const [metric, setMetric] = useState<MetricId>("quota");
  if (buckets.length === 0) return null;
  const values = buckets.map((b) => b[metric]);
  const max = Math.max(...values, 1);
  const points = buckets.map((_, i) => {
    const x = buckets.length === 1 ? 50 : (i / (buckets.length - 1)) * 100;
    const y = 100 - (values[i] / max) * 92 - 4;
    return `${x.toFixed(2)},${y.toFixed(2)}`;
  });
  const line = buckets.length === 1 ? `0,${points[0].split(",")[1]} 100,${points[0].split(",")[1]}` : points.join(" ");

  return (
    <div className={CARD}>
      <div className="flex items-center justify-between gap-3">
        <p className={TITLE}>趋势变化</p>
        <div className="flex gap-1">
          {METRICS.map((m) => (
            <button
              key={m.id}
              onClick={() => setMetric(m.id)}
              className={`rounded-full px-2.5 py-1 text-xs transition ${
                metric === m.id
                  ? "bg-gray-900 text-white dark:bg-white dark:text-gray-900"
                  : "border border-black/10 text-gray-600 hover:bg-black/5 dark:border-white/15 dark:text-gray-300 dark:hover:bg-white/10"
              }`}
            >
              {m.label}
            </button>
          ))}
        </div>
      </div>

      <div className="relative mt-4 h-32">
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="h-full w-full" aria-hidden="true">
          <polyline
            points={`0,100 ${line} 100,100`}
            fill="rgb(99 102 241 / 0.12)"
            stroke="none"
          />
          <polyline
            points={line}
            fill="none"
            stroke="rgb(99 102 241)"
            strokeWidth="2"
            strokeLinejoin="round"
            strokeLinecap="round"
            vectorEffect="non-scaling-stroke"
          />
        </svg>
        <div className="absolute inset-0 flex">
          {buckets.map((b, i) => (
            <div
              key={b.date}
              title={`${b.date} · ${formatMetric(metric, values[i])}`}
              className="flex-1 rounded transition hover:bg-black/5 dark:hover:bg-white/10"
            />
          ))}
        </div>
      </div>

      <div className="mt-2 flex justify-between text-xs text-gray-500 dark:text-gray-400">
        <span>{buckets[0].date.slice(5)}</span>
        <span>峰值 {formatMetric(metric, max)}</span>
        <span>{buckets[buckets.length - 1].date.slice(5)}</span>
      </div>
    </div>
  );
}

function DimensionList({ title, items }: { title: string; items: UsageDimension[] }) {
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
                {num(item.requests)} 次 · {usd(item.quota)}
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

  const [groups, setGroups] = useState<GroupOption[]>([]);
  const [allModels, setAllModels] = useState<string[]>([]);

  const [range, setRange] = useState<RangeId>("7d");
  const [from, setFrom] = useState(toDateInput(dayStart(new Date()) - 6 * 86400));
  const [to, setTo] = useState(toDateInput(dayStart(new Date())));
  const [group, setGroup] = useState("");
  const [vendor, setVendor] = useState<Vendor | "">("");

  const [summary, setSummary] = useState<UsageSummary | null>(null);
  const [logs, setLogs] = useState<UsageLogItem[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!token) {
      navigate("/login", { replace: true });
      return;
    }
    api
      .bootstrap(token)
      .then((data) => {
        setGroups(data.groups ?? []);
        setAllModels(data.models ?? []);
      })
      .catch(() => undefined);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const [startTimestamp, endTimestamp] = useMemo(
    () => rangeToTimestamps(range, from, to),
    [range, from, to]
  );

  // 厂商筛选转成模型名列表交给后端聚合；日志列表侧在本地按厂商过滤
  const vendorModels = useMemo(
    () => (vendor ? allModels.filter((m) => vendorOfModel(m) === vendor) : []),
    [vendor, allModels]
  );

  useEffect(() => {
    if (!token) return;
    setLoading(true);
    setError(null);
    const query = {
      startTimestamp: startTimestamp || undefined,
      endTimestamp: endTimestamp || undefined,
      group: group || undefined,
      models: vendor ? vendorModels : undefined,
    };
    Promise.all([
      api.usageSummary(token, query),
      api.usage(token, { ...query, pageSize: 100 }),
    ])
      .then(([s, l]) => {
        setSummary(s);
        setLogs(l.items ?? []);
      })
      .catch((e) => setError(e instanceof Error ? e.message : String(e)))
      .finally(() => setLoading(false));
  }, [token, startTimestamp, endTimestamp, group, vendor, vendorModels]);

  const visibleLogs = useMemo(
    () => (vendor ? logs.filter((l) => vendorOfModel(l.model_name) === vendor) : logs),
    [logs, vendor]
  );

  const groupOptions = useMemo(() => {
    const list = vendor ? groups.filter((g) => vendorOfGroup(g.name) === vendor) : groups;
    return list.map((g) => g.name);
  }, [groups, vendor]);

  const tokensTotal = (summary?.prompt_tokens ?? 0) + (summary?.completion_tokens ?? 0);
  const requests = summary?.requests ?? 0;
  const avgCost = requests > 0 ? usd(summary!.quota / requests) : "—";
  const streamRate = requests > 0 ? `${Math.round((summary!.stream_requests / requests) * 100)}%` : "—";

  // 折线图按所选周期补齐没有消费的日期，避免时间轴被压缩
  const trendBuckets = useMemo(() => {
    const raw = summary?.by_day ?? [];
    if (!startTimestamp || !endTimestamp) return raw;
    const byDate = new Map(raw.map((d) => [d.date, d]));
    const out: UsageDayBucket[] = [];
    for (let ts = dayStart(new Date(startTimestamp * 1000)); ts <= endTimestamp; ts += 86400) {
      const date = toDateInput(ts);
      out.push(byDate.get(date) ?? { date, quota: 0, tokens: 0, requests: 0 });
      if (out.length > 366) break;
    }
    return out;
  }, [summary, startTimestamp, endTimestamp]);

  return (
    <div className="flex h-screen flex-col bg-transparent">
      <header className="flex items-center gap-3 border-b border-black/5 px-6 py-4 dark:border-white/10">
        <button
          onClick={() => navigate("/home")}
          className="text-gray-500 transition hover:text-gray-900 dark:text-gray-400 dark:hover:text-white"
        >
          ←
        </button>
        <h1 className="text-sm font-semibold text-gray-900 dark:text-white">用量明细</h1>
      </header>

      <main className="flex-1 overflow-y-auto p-6">
        <div className="mx-auto max-w-3xl space-y-4">
          <div className={CARD}>
            <div className="flex flex-wrap items-center gap-2">
              {RANGES.map((r) => (
                <button
                  key={r.id}
                  onClick={() => setRange(r.id)}
                  className={`rounded-full px-3 py-1.5 text-xs transition ${
                    range === r.id
                      ? "bg-gray-900 text-white dark:bg-white dark:text-gray-900"
                      : "border border-black/10 text-gray-600 hover:bg-black/5 dark:border-white/15 dark:text-gray-300 dark:hover:bg-white/10"
                  }`}
                >
                  {r.label}
                </button>
              ))}
              <button
                onClick={() => setRange("custom")}
                className={`rounded-full px-3 py-1.5 text-xs transition ${
                  range === "custom"
                    ? "bg-gray-900 text-white dark:bg-white dark:text-gray-900"
                    : "border border-black/10 text-gray-600 hover:bg-black/5 dark:border-white/15 dark:text-gray-300 dark:hover:bg-white/10"
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
                aria-label="按模型所属公司筛选"
                className={SELECT}
              >
                <option value="">全部公司</option>
                {VENDORS.map((v) => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </select>
              <select
                value={group}
                onChange={(e) => setGroup(e.target.value)}
                aria-label="按分组筛选"
                className={SELECT}
              >
                <option value="">全部分组</option>
                {groupOptions.map((name) => (
                  <option key={name} value={name}>
                    {name}
                  </option>
                ))}
              </select>
            </div>
          </div>

          {error && <p className="text-center text-sm text-red-600 dark:text-red-400">{error}</p>}
          {loading && <p className="text-center text-sm text-gray-500 dark:text-gray-400">加载中…</p>}

          {!loading && summary && (
            <>
              <div className="grid grid-cols-2 gap-3 sm:grid-cols-3">
                <div className={CARD}>
                  <p className={LABEL}>累计消费</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {usd(summary.quota)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">均次 {avgCost}</p>
                </div>
                <div className={CARD}>
                  <p className={LABEL}>累计 token</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {num(tokensTotal)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">
                    入 {num(summary.prompt_tokens)} · 出 {num(summary.completion_tokens)}
                  </p>
                </div>
                <div className={CARD}>
                  <p className={LABEL}>请求次数</p>
                  <p className="mt-1 text-xl font-semibold text-gray-900 dark:text-white">
                    {num(requests)}
                  </p>
                  <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">流式 {streamRate}</p>
                </div>
              </div>

              <TrendChart buckets={trendBuckets} />

              <div className="grid gap-3 sm:grid-cols-2">
                <DimensionList title="模型消费排行" items={summary.by_model ?? []} />
                <DimensionList title="分组消费排行" items={summary.by_group ?? []} />
              </div>
            </>
          )}

          {!loading && !error && (
            <div className={CARD}>
              <p className={TITLE}>明细记录</p>
              {visibleLogs.length === 0 ? (
                <p className="mt-3 text-center text-sm text-gray-500 dark:text-gray-400">
                  当前筛选条件下暂无用量记录
                </p>
              ) : (
                <table className="mt-3 w-full text-xs text-gray-700 dark:text-gray-300">
                  <thead>
                    <tr className="border-b border-black/5 text-left text-gray-500 dark:border-white/10 dark:text-gray-400">
                      <th className="pb-2 pr-3">时间</th>
                      <th className="pb-2 pr-3">模型</th>
                      <th className="pb-2 pr-3">分组</th>
                      <th className="pb-2 pr-3 text-right">输入</th>
                      <th className="pb-2 pr-3 text-right">输出</th>
                      <th className="pb-2 text-right">消费</th>
                    </tr>
                  </thead>
                  <tbody>
                    {visibleLogs.map((l) => (
                      <tr key={l.id} className="border-b border-black/5 dark:border-white/10">
                        <td className="py-2 pr-3 text-gray-500 dark:text-gray-400">
                          {new Date(l.created_at * 1000).toLocaleString("zh-CN", {
                            month: "2-digit",
                            day: "2-digit",
                            hour: "2-digit",
                            minute: "2-digit",
                          })}
                        </td>
                        <td className="py-2 pr-3 font-mono">{l.model_name}</td>
                        <td className="py-2 pr-3 text-gray-500 dark:text-gray-400">{l.group || "—"}</td>
                        <td className="py-2 pr-3 text-right">{num(l.prompt_tokens)}</td>
                        <td className="py-2 pr-3 text-right">{num(l.completion_tokens)}</td>
                        <td className="py-2 text-right text-indigo-600 dark:text-indigo-400">
                          {usd(l.quota)}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </div>
          )}
        </div>
      </main>
    </div>
  );
}
