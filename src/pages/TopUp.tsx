import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { loadAuth, saveAuth } from "../store/auth";
import { api, type PayMethod, type TopUpInfo, type TopUpRecord } from "../api/client";
import { useSession } from "../hooks/useSession";

const CARD = "rounded-2xl border border-black/5 bg-white p-4 shadow-sm dark:border-white/10 dark:bg-white/5";
const LABEL = "text-xs font-medium text-gray-500 dark:text-gray-400";
const TITLE = "text-sm font-semibold text-gray-900 dark:text-gray-100";
const SUBTLE = "text-xs text-gray-500 dark:text-gray-400";
const PRIMARY_BTN =
  "rounded-full bg-gray-900 px-4 py-2 text-xs font-medium text-white transition hover:bg-gray-800 disabled:opacity-40 dark:bg-white dark:text-gray-900 dark:hover:bg-gray-200";
const GHOST_BTN =
  "rounded-full border border-black/10 px-3 py-1.5 text-xs text-gray-700 transition hover:bg-black/5 disabled:opacity-40 dark:border-white/15 dark:text-gray-200 dark:hover:bg-white/10";

/** 这两种支付方式只在网页端可用，客户端不展示 */
const LAUNCHER_UNSUPPORTED_METHODS = new Set(["alipay_official", "paypal"]);

/** 充值 1 单位 = 站内 1 美元额度，与网页端换算保持一致 */
function usd(quota: number): string {
  return (quota / 1_000_000).toFixed(2);
}

function statusLabel(status: string): { text: string; cls: string } {
  switch (status) {
    case "success":
      return { text: "已到账", cls: "text-emerald-600 dark:text-emerald-400" };
    case "pending":
      return { text: "待支付", cls: "text-amber-600 dark:text-amber-400" };
    default:
      return { text: "已失效", cls: "text-gray-400" };
  }
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

export default function TopUp() {
  const navigate = useNavigate();
  const auth = loadAuth();
  const { handleSessionExpired } = useSession();

  const [info, setInfo] = useState<TopUpInfo | null>(null);
  const [loading, setLoading] = useState(true);
  const [amount, setAmount] = useState(0);
  const [custom, setCustom] = useState("");
  const [method, setMethod] = useState("");
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState("");
  const [waiting, setWaiting] = useState(false);
  const [quota, setQuota] = useState(auth?.quota ?? 0);
  const [records, setRecords] = useState<TopUpRecord[]>([]);
  const pollRef = useRef<number | null>(null);

  const token = auth?.accessToken ?? "";

  // 客户端只实现了易支付收银台流程；支付宝官方（单笔≤50 元）与 PayPal 需要网页端的
  // 独立下单/回跳链路，放在这里点了必然失败，因此直接从可选项里剔除。
  const methods: PayMethod[] = useMemo(
    () => (info?.pay_methods ?? []).filter((m) => !LAUNCHER_UNSUPPORTED_METHODS.has(m.type)),
    [info],
  );
  const options = useMemo(() => info?.amount_options ?? [], [info]);
  const minTopup = info?.min_topup ?? 1;

  const discountOf = (value: number): number => {
    const map = info?.discount ?? {};
    return map[String(value)] ?? 1;
  };

  const loadHistory = async () => {
    if (!token) return;
    try {
      const res = await api.topupHistory(token);
      setRecords(res.items ?? []);
    } catch {
      /* 记录加载失败不阻塞充值 */
    }
  };

  useEffect(() => {
    if (!token) {
      navigate("/login", { replace: true });
      return;
    }
    (async () => {
      try {
        const data = await api.topupInfo(token);
        setInfo(data);
        const first = (data.pay_methods ?? []).find(
          (m) => !LAUNCHER_UNSUPPORTED_METHODS.has(m.type),
        );
        if (first) setMethod(first.type);
        const opts = data.amount_options ?? [];
        setAmount(opts.length ? opts[0] : data.min_topup);
      } catch (e) {
        const msg = e instanceof Error ? e.message : "加载失败";
        if (msg.includes("登录") || msg.includes("过期")) handleSessionExpired();
        setError(msg);
      } finally {
        setLoading(false);
      }
    })();
    loadHistory();
    return () => {
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const effectiveAmount = custom ? Number(custom) : amount;
  const selectedMethod = methods.find((m) => m.type === method);
  const methodMin = selectedMethod?.min_topup ? Number(selectedMethod.min_topup) : minTopup;
  const amountValid =
    Number.isFinite(effectiveAmount) && effectiveAmount >= Math.max(minTopup, methodMin);
  const payable = (effectiveAmount * discountOf(effectiveAmount)).toFixed(2);

  /** 支付在收银台窗口完成，入账靠服务端回调，因此这里轮询订单状态与余额 */
  const startPolling = () => {
    setWaiting(true);
    if (pollRef.current) window.clearInterval(pollRef.current);
    let ticks = 0;
    pollRef.current = window.setInterval(async () => {
      ticks += 1;
      if (ticks > 120) {
        if (pollRef.current) window.clearInterval(pollRef.current);
        setWaiting(false);
        return;
      }
      try {
        const [boot, history] = await Promise.all([
          api.bootstrap(token),
          api.topupHistory(token),
        ]);
        setRecords(history.items ?? []);
        if (boot.user.quota !== quota) {
          setQuota(boot.user.quota);
          const cur = loadAuth();
          if (cur) saveAuth({ ...cur, quota: boot.user.quota, group: boot.user.group });
          if (pollRef.current) window.clearInterval(pollRef.current);
          setWaiting(false);
          await invoke("close_cashier").catch(() => undefined);
        }
      } catch {
        /* 轮询失败静默重试 */
      }
    }, 5000);
  };

  const handlePay = async () => {
    setError("");
    setSubmitting(true);
    try {
      const order = await api.requestEpay(token, effectiveAmount, method);
      await invoke("open_cashier", { url: order.url, params: order.params });
      startPolling();
    } catch (e) {
      setError(e instanceof Error ? e.message : "下单失败");
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="flex h-screen flex-col bg-transparent">
      <header className="flex items-center gap-3 border-b border-black/5 px-5 py-3 dark:border-white/10">
        <button
          onClick={() => navigate("/home")}
          aria-label="返回首页"
          className="text-gray-500 transition hover:text-gray-900 dark:text-gray-400 dark:hover:text-white"
        >
          ←
        </button>
        <h1 className="text-sm font-semibold text-gray-900 dark:text-white">充值</h1>
        <span className={`ml-auto ${SUBTLE}`}>可用余额 ${usd(quota)}</span>
      </header>

      <main className="flex-1 overflow-y-auto px-5 py-4">
        <div className="mx-auto max-w-3xl space-y-3">
          {loading ? (
            <div className={`${CARD} ${SUBTLE}`}>加载中…</div>
          ) : !info?.enable_online_topup || methods.length === 0 ? (
            <div className={CARD}>
              <p className={TITLE}>站内充值暂不可用</p>
              <p className={`mt-1 ${SUBTLE}`}>管理员未开启在线支付，请稍后再试。</p>
            </div>
          ) : (
            <>
              <div className={CARD}>
                <p className={TITLE}>选择金额</p>
                <p className={`mt-0.5 ${SUBTLE}`}>1 单位 = 站内 $1 额度，最低 {minTopup}</p>
                <div className="mt-3 grid grid-cols-3 gap-2 sm:grid-cols-4">
                  {options.map((opt) => {
                    const d = discountOf(opt);
                    const active = !custom && amount === opt;
                    return (
                      <button
                        key={opt}
                        onClick={() => {
                          setAmount(opt);
                          setCustom("");
                        }}
                        className={`rounded-xl border px-3 py-2 text-left transition ${
                          active
                            ? "border-gray-900 bg-gray-900 text-white dark:border-white dark:bg-white dark:text-gray-900"
                            : "border-black/10 hover:bg-black/5 dark:border-white/15 dark:hover:bg-white/10"
                        }`}
                      >
                        <span className="block text-sm font-semibold">${opt}</span>
                        {d < 1 && (
                          <span className="block text-[11px] opacity-80">
                            {(d * 10).toFixed(1)} 折
                          </span>
                        )}
                      </button>
                    );
                  })}
                </div>
                <div className="mt-3 flex items-center gap-2">
                  <label className={LABEL} htmlFor="topup-custom">
                    自定义
                  </label>
                  <input
                    id="topup-custom"
                    type="number"
                    min={minTopup}
                    value={custom}
                    onChange={(e) => setCustom(e.target.value)}
                    placeholder={`≥ ${minTopup}`}
                    className="w-28 rounded-full border border-black/10 bg-white px-3 py-1.5 text-xs text-gray-700 dark:border-white/15 dark:bg-white/5 dark:text-gray-200"
                  />
                </div>
              </div>

              <div className={CARD}>
                <p className={TITLE}>支付方式</p>
                <div className="mt-3 flex flex-wrap gap-2">
                  {methods.map((m) => (
                    <button
                      key={m.type}
                      onClick={() => setMethod(m.type)}
                      className={`flex items-center gap-2 rounded-xl border px-3 py-2 text-xs transition ${
                        method === m.type
                          ? "border-gray-900 dark:border-white"
                          : "border-black/10 hover:bg-black/5 dark:border-white/15 dark:hover:bg-white/10"
                      }`}
                    >
                      <span
                        className="h-2 w-2 rounded-full"
                        style={{ backgroundColor: m.color || "#6366F1" }}
                      />
                      <span className="font-medium text-gray-900 dark:text-gray-100">{m.name}</span>
                      {m.tag && <span className={SUBTLE}>{m.tag}</span>}
                    </button>
                  ))}
                </div>
              </div>

              <div className={`${CARD} flex flex-wrap items-center gap-3`}>
                <div>
                  <p className={LABEL}>应付</p>
                  <p className="text-lg font-semibold text-gray-900 dark:text-white">¥{payable}</p>
                </div>
                <button
                  onClick={handlePay}
                  disabled={!amountValid || !method || submitting}
                  className={`ml-auto ${PRIMARY_BTN}`}
                >
                  {submitting ? "下单中…" : "去支付"}
                </button>
                {waiting && (
                  <button onClick={loadHistory} className={GHOST_BTN}>
                    刷新到账状态
                  </button>
                )}
              </div>

              {!amountValid && (
                <p className="text-xs text-amber-600 dark:text-amber-400">
                  金额需不低于 {Math.max(minTopup, methodMin)}
                </p>
              )}
              {waiting && (
                <p className={SUBTLE}>
                  已打开支付窗口，完成付款后余额会自动刷新，请勿关闭本页。
                </p>
              )}
              {error && <p className="text-xs text-rose-600 dark:text-rose-400">{error}</p>}
            </>
          )}

          <div className={CARD}>
            <div className="flex items-center justify-between">
              <p className={TITLE}>最近充值</p>
              <button onClick={loadHistory} className={GHOST_BTN}>
                刷新
              </button>
            </div>
            {records.length === 0 ? (
              <p className={`mt-2 ${SUBTLE}`}>暂无充值记录</p>
            ) : (
              <ul className="mt-2 divide-y divide-black/5 dark:divide-white/10">
                {records.map((r) => {
                  const s = statusLabel(r.status);
                  return (
                    <li key={r.id} className="flex items-center gap-3 py-2 text-xs">
                      <span className="font-medium text-gray-900 dark:text-gray-100">${r.amount}</span>
                      <span className={SUBTLE}>¥{r.money.toFixed(2)}</span>
                      <span className={SUBTLE}>{r.payment_method}</span>
                      <span className={`ml-auto ${SUBTLE}`}>{formatTime(r.create_time)}</span>
                      <span className={s.cls}>{s.text}</span>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </div>
      </main>
    </div>
  );
}
