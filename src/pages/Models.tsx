// 模型与价格：完整展示服务端目录里提供的模型与对应价格
// 价格换算公式与 src/lib/pricing.ts（即网页端）一致：
//   输入价（USD / 百万 token）= model_ratio × 2 × 分组倍率，输出价再乘 completion_ratio
// 展示顺序遵循服务端 model_order（发布顺序），与首页模型选择一致。

import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { loadAuth } from "../store/auth";
import { api, type BootstrapData, type GroupOption, type ModelMetadata } from "../api/client";
import { buildPricingIndex, fmtUSD, priceOf, type ModelPrice } from "../lib/pricing";
import { vendorOfModel } from "../lib/vendor";
import { ArrowLeftIcon } from "../components/Icons";
import { friendlyDesktopError } from "../lib/copy";

const CARD = "nk-card";
const SUBTLE = "nk-muted";
const TITLE = "nk-title";
const SELECT = "nk-select";

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

export default function Models() {
  const navigate = useNavigate();
  const token = loadAuth()?.accessToken;

  const [data, setData] = useState<BootstrapData | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [group, setGroup] = useState("");
  const [query, setQuery] = useState("");

  useEffect(() => {
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

  const rows = useMemo(() => {
    if (!data) return [];
    return orderedModels(data)
      .filter((name) => name.toLowerCase().includes(query.trim().toLowerCase()))
      .map((name) => ({
        name,
        vendor: vendorOfModel(name),
        release: releaseDate(data.model_metadata?.[name], pricingIndex.get(name)?.release_date),
        price: priceOf(pricingIndex.get(name), ratio) as ModelPrice | null,
      }));
  }, [data, pricingIndex, ratio, query]);

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

          {!loading && !error && (
            <div className={`${CARD} overflow-x-auto`}>
              <table className="w-full min-w-[34rem] text-left text-xs">
                <thead>
                  <tr className={`${SUBTLE} border-b [border-color:var(--nk-line)]`}>
                    <th className="py-2 pr-3 font-medium">模型</th>
                    <th className="py-2 pr-3 font-medium">厂商</th>
                    <th className="py-2 pr-3 font-medium">发布</th>
                    <th className="py-2 pr-3 text-right font-medium">输入 / 百万</th>
                    <th className="py-2 pr-3 text-right font-medium">输出 / 百万</th>
                    <th className="py-2 text-right font-medium">缓存命中 / 百万</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map(({ name, vendor, release, price }) => (
                    <tr key={name} className="border-b last:border-0 [border-color:var(--nk-line)]">
                      <td className="py-2 pr-3 font-mono">{name}</td>
                      <td className="py-2 pr-3">{vendor}</td>
                      <td className={`py-2 pr-3 tabular-nums ${SUBTLE}`}>{release || "—"}</td>
                      {price ? (
                        price.perRequest ? (
                          <td className="py-2 pr-3 text-right tabular-nums" colSpan={3}>
                            {fmtUSD(price.input)} / 次
                          </td>
                        ) : (
                          <>
                            <td className="py-2 pr-3 text-right tabular-nums">{fmtUSD(price.input)}</td>
                            <td className="py-2 pr-3 text-right tabular-nums">{fmtUSD(price.output)}</td>
                            <td className="py-2 text-right tabular-nums">
                              {price.cache !== undefined ? fmtUSD(price.cache) : "—"}
                            </td>
                          </>
                        )
                      ) : (
                        <td className={`py-2 pr-3 ${SUBTLE}`} colSpan={3}>
                          该分组暂无此模型价格
                        </td>
                      )}
                    </tr>
                  ))}
                  {rows.length === 0 && (
                    <tr>
                      <td colSpan={6} className={`py-4 text-center ${SUBTLE}`}>
                        没有匹配的模型。
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          )}

          <p className={`px-1 text-[11px] ${SUBTLE}`}>
            价格 = 官方基准价 × 分组倍率（当前 {group || "—"}：{ratio || "—"}x）。按次计费的模型单独标注。
          </p>
        </div>
      </main>
    </div>
  );
}
