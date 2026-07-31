import { ApiError, apiRequest, friendlyApiError, isRetryableApiError, unwrap } from "./api.js";

const result = document.querySelector("[data-payment-result]");
const mark = document.querySelector("[data-result-mark]");
const title = document.querySelector("[data-result-title]");
const message = document.querySelector("[data-result-message]");
const action = document.querySelector("[data-result-action]");
const retry = document.querySelector("[data-retry]");

const orderId = new URLSearchParams(window.location.search).get("order_id") || "";
const validOrderId = /^[A-Za-z0-9_-]{1,128}$/.test(orderId);
const terminalStatuses = new Set([
  "success",
  "failed",
  "expired",
  "partially_refunded",
  "refunded",
]);

let stopped = false;

function render(state, heading, detail, symbol) {
  result.dataset.state = state;
  mark.textContent = symbol;
  title.textContent = heading;
  message.textContent = detail;
}

function orderStatus(payload) {
  const data = unwrap(payload) || {};
  return String(data.status || data.order?.status || "").toLowerCase();
}

function renderStatus(status) {
  switch (status) {
    case "success":
      render("success", "充值已到账", "这次充值已由支付结果确认，个人中心余额将显示最新结果。", "✓");
      return;
    case "failed":
      render("failed", "支付未完成", "这次充值没有完成，账户余额没有变化。你可以返回个人中心重新发起充值。", "×");
      return;
    case "expired":
      render("failed", "充值已过期", "这次充值已过期，账户余额没有变化。", "×");
      return;
    case "partially_refunded":
      render("success", "充值已部分退款", "退款结果已经记入统一账户，请前往个人中心查看余额和记录。", "✓");
      return;
    case "refunded":
      render("success", "充值已退款", "退款结果已经记入统一账户，请前往个人中心查看余额和记录。", "✓");
      return;
    default:
      render("pending", "正在确认充值结果", "支付页面返回不代表已经到账，正在等待支付结果确认。", "…");
  }
}

async function pollOrder() {
  if (!validOrderId) {
    render("failed", "无法识别这次充值", "回跳地址缺少有效信息，请从个人中心查看充值记录。", "×");
    retry.hidden = true;
    return;
  }

  stopped = false;
  retry.hidden = true;
  renderStatus("pending");

  for (let attempt = 0; attempt < 24 && !stopped; attempt += 1) {
    try {
      const payload = await apiRequest(`/wallet/topup-orders/${encodeURIComponent(orderId)}`);
      const status = orderStatus(payload);
      renderStatus(status);
      if (terminalStatuses.has(status)) {
        return;
      }
    } catch (error) {
      if (error instanceof ApiError && error.status === 401) {
        render("failed", "登录状态已失效", "请重新登录后再查看这次充值。", "×");
        action.href = `/login/?next=${encodeURIComponent(window.location.pathname + window.location.search)}`;
        action.textContent = "重新登录";
        return;
      }
      if (!isRetryableApiError(error)) {
        render("failed", "暂时无法查询充值结果", friendlyApiError(error, "请到个人中心查看最新记录。"), "×");
        retry.hidden = true;
        return;
      }
      if (attempt >= 2) {
        render("failed", "暂时无法查询充值结果", "支付结果不会由此页面决定，请稍后重试或查看充值记录。", "×");
        retry.hidden = false;
        return;
      }
    }
    await new Promise((resolve) => window.setTimeout(resolve, attempt < 5 ? 2500 : 5000));
  }

  if (!stopped) {
    render("pending", "充值仍在处理中", "支付结果确认可能需要一点时间。你可以稍后在个人中心查看最终结果。", "…");
    retry.hidden = false;
  }
}

retry.addEventListener("click", pollOrder);
window.addEventListener("pagehide", () => {
  stopped = true;
});

pollOrder();
