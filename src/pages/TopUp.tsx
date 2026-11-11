import { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { isCurrentAuthSession, loadAuth, refreshAuthMeta } from "../store/auth";
import { isNewApiAuth, stationOrigin } from "../lib/station";
import { api, type PayMethod, type TopUpInfo, type TopUpRecord } from "../api/client";
import { useSession } from "../hooks/useSession";
import { ArrowLeftIcon } from "../components/Icons";
import { formatBalanceUSD, parseBalanceSnapshot, type BalanceSnapshot } from "../lib/balance";
import { classifyDesktopError, friendlyDesktopError } from "../lib/copy";
import { epayTradeNo, isTopupPaid } from "../lib/topup";

const CARD = "nk-card";
const LABEL = "nk-label";
const TITLE = "nk-title";
const SUBTLE = "nk-muted";
const PRIMARY_BTN = "nk-btn-primary";
const GHOST_BTN = "nk-btn-secondary";

/** 这两种支付方式只在网页端可用，客户端不展示 */
const LAUNCHER_UNSUPPORTED_METHODS = new Set(["alipay_official", "paypal"]);

function statusLabel(status: string): { text: string; cls: string } {
  switch (status) {
    case "success":
      return { text: "已到账", cls: "text-emerald-600 dark:text-emerald-400" };
    case "pending":
      return { text: "待支付", cls: "text-amber-600 dark:text-amber-400" };
    default:
      return { text: "状态暂不可用，请重新读取", cls: "text-gray-400" };
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
  const [balance, setBalance] = useState<BalanceSnapshot | null>(() =>
    parseBalanceSnapshot(auth?.quota, auth?.quotaPerUnit, auth?.balanceUpdatedAt),
  );
  const balanceRef = useRef(balance);
  const balanceRequestRef = useRef(0);
  const [records, setRecords] = useState<TopUpRecord[]>([]);
  const [historyLoading, setHistoryLoading] = useState(true);
  const [historyError, setHistoryError] = useState("");
  const pollRef = useRef<number | null>(null);
  const pollVersionRef = useRef(0);
  const pendingOrderRef = useRef<string | null>(null);
  const mountedRef = useRef(false);
  const pageGenerationRef = useRef(0);

  const token = auth?.accessToken ?? "";
  const isCurrentPage = (generation: number) =>
    mountedRef.current && pageGenerationRef.current === generation && isCurrentAuthSession(auth);

  const stopPolling = () => {
    pollVersionRef.current += 1;
    if (pollRef.current !== null) window.clearTimeout(pollRef.current);
    pollRef.current = null;
  };

  const loadBalance = async (refreshUnit = false) => {
    const generation = pageGenerationRef.current;
    const request = ++balanceRequestRef.current;
    const shouldLoadUnit = refreshUnit || !balanceRef.current?.quotaPerUnit;
    const [boot, status] = await Promise.all([
      api.bootstrap(token, { force: true }),
      shouldLoadUnit ? api.status() : Promise.resolve(null),
    ]);
    if (!isCurrentPage(generation) || request !== balanceRequestRef.current) return;
    const snapshot = parseBalanceSnapshot(
      boot.user.quota,
      boot.site.quota_per_unit ?? status?.quota_per_unit ?? balanceRef.current?.quotaPerUnit,
    );
    if (!snapshot) throw new Error("余额单位暂时无法读取");
    if (!refreshAuthMeta({
      quota: snapshot.quota,
      quotaPerUnit: snapshot.quotaPerUnit,
      balanceUpdatedAt: snapshot.updatedAt,
      defaultGroup: boot.user.group,
    }, auth)) return;
    balanceRef.current = snapshot;
    setBalance(snapshot);
  };

  const confirmPayment = async (items: TopUpRecord[] | null, generation: number, version: number) => {
    const tradeNo = pendingOrderRef.current;
    const current = () => isCurrentPage(generation) && pollVersionRef.current === version;
    if (!current() || !isTopupPaid(tradeNo, items)) return;
    await loadBalance();
    if (!current() || pendingOrderRef.current !== tradeNo) return;
    stopPolling();
    pendingOrderRef.current = null;
    setWaiting(false);
    await invoke("close_cashier").catch(() => undefined);
  };

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
    const generation = pageGenerationRef.current;
    const version = pollVersionRef.current;
    setHistoryLoading(true);
    setHistoryError("");
    try {
      const res = await api.topupHistory(token);
      if (!isCurrentPage(generation)) return;
      setRecords(res.items ?? []);
      await confirmPayment(res.items, generation, version);
    } catch (e) {
      if (!isCurrentPage(generation)) return;
      if (classifyDesktopError(e) === "session") {
        stopPolling();
        handleSessionExpired();
        return;
      }
      setHistoryError(friendlyDesktopError(e));
    } finally {
      if (isCurrentPage(generation)) setHistoryLoading(false);
    }
  };

  useEffect(() => {
    mountedRef.current = true;
    const generation = ++pageGenerationRef.current;
    if (!token) {
      navigate("/login", { replace: true });
    } else if (isNewApiAuth(auth)) {
      setLoading(false);
      setHistoryLoading(false);
    } else {
      (async () => {
        try {
          const data = await api.topupInfo(token);
          if (!isCurrentPage(generation)) return;
          setInfo(data);
          const first = (data.pay_methods ?? []).find(
            (m) => !LAUNCHER_UNSUPPORTED_METHODS.has(m.type),
          );
          if (first) setMethod(first.type);
          const opts = data.amount_options ?? [];
          setAmount(opts.length ? opts[0] : data.min_topup);
        } catch (e) {
          if (!isCurrentPage(generation)) return;
          if (classifyDesktopError(e) === "session") handleSessionExpired();
          setError(friendlyDesktopError(e));
        } finally {
          if (isCurrentPage(generation)) setLoading(false);
        }
      })();
      void loadBalance(true).catch(() => undefined);
      void loadHistory();
    }
    return () => {
      mountedRef.current = false;
      pageGenerationRef.current += 1;
      stopPolling();
      pendingOrderRef.current = null;
    };
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  const effectiveAmount = custom ? Number(custom) : amount;
  const selectedMethod = methods.find((m) => m.type === method);
  const methodMin = selectedMethod?.min_topup ? Number(selectedMethod.min_topup) : minTopup;
  const amountValid =
    Number.isFinite(effectiveAmount) && effectiveAmount >= Math.max(minTopup, methodMin);
  const payable = amountValid
    ? (effectiveAmount * discountOf(effectiveAmount)).toFixed(2)
    : "—";

  /** 串行轮询当前订单，确认到账后再强制读取余额。 */
  const startPolling = (tradeNo: string) => {
    stopPolling();
    pendingOrderRef.current = tradeNo;
    setWaiting(true);
    const generation = pageGenerationRef.current;
    const version = pollVersionRef.current;
    const current = () => isCurrentPage(generation) && pollVersionRef.current === version;
    let ticks = 0;
    const poll = async () => {
      if (!current()) return;
      ticks += 1;
      try {
        const history = await api.topupHistory(token);
        if (!current()) return;
        setRecords(history.items ?? []);
        await confirmPayment(history.items, generation, version);
      } catch (e) {
        if (!current()) return;
        if (classifyDesktopError(e) === "session") {
          stopPolling();
          setWaiting(false);
          handleSessionExpired();
          return;
        }
        /* 短暂失败在下一轮重试，手动读取仍可展示错误。 */
      }
      if (!current()) return;
      if (ticks >= 120) {
        stopPolling();
        setWaiting(false);
        return;
      }
      pollRef.current = window.setTimeout(() => void poll(), 5000);
    };
    pollRef.current = window.setTimeout(() => void poll(), 5000);
  };

  const handlePay = async () => {
    if (submitting || !amountValid || !method) return;
    const generation = pageGenerationRef.current;
    stopPolling();
    pendingOrderRef.current = null;
    setWaiting(false);
    const version = pollVersionRef.current;
    const current = () => isCurrentPage(generation) && pollVersionRef.current === version;
    setError("");
    setSubmitting(true);
    try {
      const order = await api.requestEpay(token, effectiveAmount, method);
      if (!current()) return;
      const tradeNo = epayTradeNo(order);
      if (!tradeNo) throw new Error("暂时无法读取充值订单号，请重新下单。");
      await invoke("open_cashier", { url: order.url, params: order.params });
      if (!current()) return;
      setSubmitting(false);
      startPolling(tradeNo);
    } catch (e) {
      if (!current()) return;
      if (classifyDesktopError(e) === "session") handleSessionExpired();
      setError(friendlyDesktopError(e));
    } finally {
      if (current()) setSubmitting(false);
    }
  };

  if (isNewApiAuth(auth)) {
    return (
      <div className="nk-shell">
        <header className="nk-header">
          <button
            onClick={() => navigate("/home")}
            aria-label="返回首页"
            className="nk-btn-ghost px-2.5"
          >
            <ArrowLeftIcon />
          </button>
          <h1 className={TITLE}>充值</h1>
        </header>
        <main className="nk-page">
          <div className="mx-auto max-w-3xl">
            <div className={CARD}>
              <p className={TITLE}>请到中转站控制台充值</p>
              <p className={`mt-1 ${SUBTLE}`}>
                当前连接的是第三方 new-api 站点，Niko 不会代收或代下单。请打开 {stationOrigin(auth)} 完成充值。
              </p>
            </div>
          </div>
        </main>
      </div>
    );
  }

  return (
    <div className="nk-shell">
      <header className="nk-header">
        <button
          onClick={() => navigate("/home")}
          aria-label="返回首页"
          className="nk-btn-ghost px-2.5"
        >
          <ArrowLeftIcon />
        </button>
        <h1 className={TITLE}>充值</h1>
        <span className={`ml-auto ${SUBTLE}`}>可用余额 {formatBalanceUSD(balance)}</span>
      </header>

      <main className="nk-page">
        <div className="mx-auto max-w-3xl space-y-3">
          {loading ? (
            <div role="status" className={`${CARD} flex items-center justify-center gap-2 py-8`}>
              <span className="nk-spinner" aria-hidden="true" />
              <span className={SUBTLE}>正在加载充值方式…</span>
            </div>
          ) : !info?.enable_online_topup || methods.length === 0 ? (
            <div className={CARD}>
              <p className={TITLE}>在线充值暂时不可用</p>
              <p className={`mt-1 ${SUBTLE}`}>当前没有可用的支付方式，请稍后重新读取。</p>
            </div>
          ) : (
            <>
              <div className={CARD}>
                <p className={TITLE}>选择金额</p>
                <p className={`mt-0.5 ${SUBTLE}`}>充值金额会增加对应的可用余额，最低 ${minTopup}</p>
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
                            ? "border-transparent bg-[var(--nk-action)] text-[var(--nk-on-action)] shadow-sm"
                            : "border-[var(--nk-line)] bg-[var(--nk-surface-muted)] hover:border-[var(--nk-line-strong)] hover:bg-[var(--nk-surface-hover)]"
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
                    其他金额
                  </label>
                  <input
                    id="topup-custom"
                    type="number"
                    min={minTopup}
                    value={custom}
                    onChange={(e) => setCustom(e.target.value)}
                    placeholder={`≥ ${minTopup}`}
                    className="nk-input w-28 py-1.5 text-xs"
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
                          ? "border-[var(--nk-focus)] bg-[var(--nk-info-soft)]"
                          : "border-[var(--nk-line)] bg-[var(--nk-surface-muted)] hover:border-[var(--nk-line-strong)]"
                      }`}
                    >
                      <span
                        className="h-2 w-2 rounded-full"
                        style={{ backgroundColor: m.color || "#78C5DF" }}
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
                    重新读取到账状态
                  </button>
                )}
              </div>

              {!amountValid && (
                <p className="nk-alert-warning">
                  金额需不低于 {Math.max(minTopup, methodMin)}
                </p>
              )}
              {waiting && (
                <p className={SUBTLE}>
                  支付页面已打开，完成付款后余额会自动读取，请不要关闭本页。
                </p>
              )}
              {error && <p className="nk-alert-danger">{error}</p>}
            </>
          )}

          <div className={CARD}>
            <div className="flex items-center justify-between">
              <p className={TITLE}>最近充值</p>
              <button onClick={loadHistory} className={GHOST_BTN}>
                重新读取
              </button>
            </div>
            {historyLoading ? (
              <p className="nk-empty mt-3" role="status">正在读取充值记录…</p>
            ) : historyError ? (
              <p className="nk-alert-danger mt-3">{historyError}</p>
            ) : records.length === 0 ? (
              <p className="nk-empty mt-3">暂无充值记录</p>
            ) : (
              <ul className="mt-2 divide-y divide-black/5 dark:divide-white/10">
                {records.map((r) => {
                  const s = statusLabel(r.status);
                  return (
                    <li key={r.id} className="flex flex-wrap items-center gap-x-3 gap-y-1 py-2 text-xs">
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
