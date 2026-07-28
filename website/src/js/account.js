import {
  ApiError,
  apiRequest,
  createIdempotencyKey,
  displayMoney,
  formatDate,
  getPublicConfig,
  unwrap,
} from "./api.js";

const elements = {
  loading: document.querySelector("[data-account-loading]"),
  access: document.querySelector("[data-account-error]"),
  accessTitle: document.querySelector("[data-access-title]"),
  accessMessage: document.querySelector("[data-access-message]"),
  accessAction: document.querySelector("[data-access-action]"),
  content: document.querySelector("[data-account-content]"),
  logout: document.querySelector("[data-logout]"),
  notice: document.querySelector("[data-page-notice]"),
  balance: document.querySelector("[data-balance]"),
  balanceUpdated: document.querySelector("[data-balance-updated]"),
  topupDialog: document.querySelector("[data-topup-dialog]"),
  emailDialog: document.querySelector("[data-email-dialog]"),
};

const recordState = {
  topups: { loaded: false, loading: false, nextCursor: "" },
  consumptions: { loaded: false, loading: false, nextCursor: "" },
};

let account = null;
let emailTurnstileWidget;
let emailTurnstileRequired = true;
let codeCooldownTimer;

function setNotice(message = "", tone = "error") {
  elements.notice.textContent = message;
  elements.notice.dataset.tone = message ? tone : "";
}

function setDialogStatus(name, message = "", tone = "error") {
  const target = document.querySelector(`[data-${name}-status]`);
  target.textContent = message;
  target.dataset.tone = message ? tone : "";
}

function showAccess({ title, message, login = true }) {
  elements.loading.hidden = true;
  elements.content.hidden = true;
  elements.logout.hidden = true;
  elements.access.hidden = false;
  elements.accessTitle.textContent = title;
  elements.accessMessage.textContent = message;
  elements.accessAction.hidden = !login;
}

function requireLogin() {
  showAccess({
    title: "需要先登录",
    message: "登录后可查看同一账号的余额、充值记录和消费明细。",
  });
}

function accountData(payload) {
  const data = unwrap(payload) || {};
  return data.user && typeof data.user === "object" ? data.user : data;
}

function renderAccount(user) {
  account = user;
  const username = user.username || user.name || "用户";
  const email = user.email || user.email_masked || "";
  const emailBound = user.email_bound === true || Boolean(user.email);

  document.querySelector("[data-username]").textContent = username;
  document.querySelector("[data-profile-username]").textContent = username;
  document.querySelector("[data-user-id]").textContent = String(user.id || user.user_id || "—");
  document.querySelector("[data-profile-email]").textContent = email || "未绑定";
  document.querySelector("[data-created-at]").textContent = formatDate(user.created_at, {
    includeTime: false,
  });

  const emailAction = document.querySelector("[data-open-email]");
  emailAction.textContent = emailBound ? "更换邮箱" : "绑定邮箱";
  const emailInput = document.querySelector('[data-email-form] input[name="email"]');
  emailInput.value = user.email || "";

  elements.loading.hidden = true;
  elements.access.hidden = true;
  elements.content.hidden = false;
  elements.logout.hidden = false;
}

function renderBalance(payload) {
  const data = unwrap(payload) || {};
  const wallet = data.wallet && typeof data.wallet === "object" ? data.wallet : data;
  elements.balance.textContent = displayMoney(wallet, "balance");
  elements.balanceUpdated.textContent = formatDate(
    wallet.updated_at || wallet.as_of || data.updated_at,
  );
}

function setBalanceUnavailable() {
  elements.balance.textContent = "暂不可用";
  elements.balanceUpdated.textContent = "—";
}

function recordsPayload(payload) {
  const data = unwrap(payload);
  if (Array.isArray(data)) {
    return { items: data, nextCursor: "" };
  }
  return {
    items: Array.isArray(data?.items)
      ? data.items
      : Array.isArray(data?.records)
        ? data.records
        : [],
    nextCursor:
      typeof data?.next_cursor === "string"
        ? data.next_cursor
        : typeof data?.nextCursor === "string"
          ? data.nextCursor
          : "",
  };
}

function statusLabel(status) {
  const labels = {
    pending: "处理中",
    success: "成功",
    failed: "失败",
    expired: "已过期",
    partially_refunded: "部分退款",
    refunded: "已退款",
  };
  return labels[status] || status || "未知";
}

function statusTone(status) {
  if (status === "success") {
    return "success";
  }
  if (status === "pending") {
    return "pending";
  }
  if (["failed", "expired", "partially_refunded", "refunded"].includes(status)) {
    return "failed";
  }
  return "";
}

function textBlock(primary, secondary = "") {
  const wrapper = document.createElement("div");
  const main = document.createElement("span");
  main.className = "record-primary";
  main.textContent = primary;
  wrapper.append(main);
  if (secondary) {
    const detail = document.createElement("span");
    detail.className = "record-secondary";
    detail.textContent = secondary;
    wrapper.append(detail);
  }
  return wrapper;
}

function renderTopup(item) {
  const row = document.createElement("li");
  row.className = "record-row";

  const orderId = String(item.order_id || item.id || "—");
  row.append(
    textBlock(`充值订单 ${orderId}`, item.payment_channel || item.channel || ""),
  );

  const value = document.createElement("span");
  value.className = "record-value";
  value.textContent = displayMoney(item);
  row.append(value);

  const status = String(item.status || "").toLowerCase();
  const statusElement = document.createElement("span");
  statusElement.className = `status-chip ${statusTone(status)}`.trim();
  statusElement.textContent = statusLabel(status);
  row.append(statusElement);

  const time = document.createElement("time");
  time.className = "record-time";
  time.dateTime = item.paid_at || item.created_at || "";
  time.textContent = formatDate(item.paid_at || item.created_at);
  row.append(time);
  return row;
}

function renderConsumption(item) {
  const row = document.createElement("li");
  row.className = "record-row";

  row.append(textBlock(item.model || "模型调用", String(item.id || "")));

  const value = document.createElement("span");
  value.className = "record-value";
  value.textContent = displayMoney(item);
  row.append(value);

  const tokens = document.createElement("div");
  tokens.className = "record-meta";
  const promptTokens = item.prompt_tokens ?? "—";
  const completionTokens = item.completion_tokens ?? "—";
  tokens.append(textBlock(`输入 ${promptTokens}`, `输出 ${completionTokens}`));
  row.append(tokens);

  const time = document.createElement("time");
  time.className = "record-time";
  time.dateTime = item.created_at || "";
  time.textContent = formatDate(item.created_at);
  row.append(time);
  return row;
}

function renderRecordState(kind, heading, detail, retry = false) {
  const state = document.querySelector(`[data-state="${kind}"]`);
  state.replaceChildren();
  const strong = document.createElement("strong");
  strong.textContent = heading;
  const paragraph = document.createElement("p");
  paragraph.textContent = detail;
  state.append(strong, paragraph);
  if (retry) {
    const button = document.createElement("button");
    button.className = "command-button";
    button.type = "button";
    button.textContent = "重新加载";
    button.addEventListener("click", () => loadRecords(kind, false));
    state.append(button);
  }
  state.hidden = false;
}

async function loadRecords(kind, append) {
  const state = recordState[kind];
  if (state.loading) {
    return;
  }
  state.loading = true;

  const list = document.querySelector(`[data-list="${kind}"]`);
  const loadMore = document.querySelector(`[data-load-more="${kind}"]`);
  if (!append) {
    list.replaceChildren();
    renderRecordState(
      kind,
      kind === "topups" ? "正在加载充值记录" : "正在加载消费明细",
      "请稍候…",
    );
  }
  loadMore.disabled = true;

  const query = new URLSearchParams({ limit: "20" });
  if (append && state.nextCursor) {
    query.set("cursor", state.nextCursor);
  }

  try {
    const payload = await apiRequest(`/wallet/${kind}?${query.toString()}`);
    const result = recordsPayload(payload);
    const fragment = document.createDocumentFragment();
    for (const item of result.items) {
      fragment.append(kind === "topups" ? renderTopup(item) : renderConsumption(item));
    }
    list.append(fragment);

    state.loaded = true;
    state.nextCursor = result.nextCursor;
    document.querySelector(`[data-state="${kind}"]`).hidden = list.children.length > 0;
    list.hidden = list.children.length === 0;
    loadMore.hidden = !state.nextCursor;
    if (list.children.length === 0) {
      renderRecordState(
        kind,
        kind === "topups" ? "暂无充值记录" : "暂无消费明细",
        kind === "topups" ? "完成第一笔充值后，订单会显示在这里。" : "产生模型消费后，明细会显示在这里。",
      );
    }
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      requireLogin();
      return;
    }
    renderRecordState(
      kind,
      kind === "topups" ? "充值记录加载失败" : "消费明细加载失败",
      "账户数据没有丢失，请稍后重试。",
      true,
    );
  } finally {
    state.loading = false;
    loadMore.disabled = false;
  }
}

async function refreshSummary({ announce = false } = {}) {
  try {
    const payload = await apiRequest("/wallet/summary");
    renderBalance(payload);
    if (announce) {
      setNotice("余额和账户资料已刷新。", "success");
    }
  } catch (error) {
    setBalanceUnavailable();
    if (error instanceof ApiError && error.status === 401) {
      requireLogin();
    } else {
      setNotice("余额暂时无法读取，请稍后重试。");
    }
  }
}

async function refreshAccount({ announce = false } = {}) {
  setNotice();
  const refreshButton = document.querySelector("[data-refresh-account]");
  refreshButton.disabled = true;
  try {
    const payload = await apiRequest("/account/me");
    renderAccount(accountData(payload));
    await refreshSummary({ announce });
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      requireLogin();
    } else if (!account) {
      showAccess({
        title: "账户信息暂时无法读取",
        message: "服务可能正在维护，请稍后刷新页面。",
        login: false,
      });
    } else {
      setNotice("账户资料刷新失败，请稍后重试。");
    }
  } finally {
    refreshButton.disabled = false;
  }
}

function optionData(payload) {
  const data = unwrap(payload) || {};
  return {
    options: Array.isArray(data.options)
      ? data.options
      : Array.isArray(data.amount_options)
        ? data.amount_options
        : Array.isArray(data.amounts)
        ? data.amounts
        : [],
    channels: Array.isArray(data.channels)
      ? data.channels
      : Array.isArray(data.payment_channels)
        ? data.payment_channels
        : [],
  };
}

function renderTopupOptions(payload) {
  const { options, channels } = optionData(payload);
  const optionList = document.querySelector("[data-topup-options]");
  const channelSelect = document.querySelector("[data-payment-channels]");
  optionList.replaceChildren();
  channelSelect.replaceChildren();

  for (const [index, option] of options.entries()) {
    const label = document.createElement("label");
    label.className = "option-choice";
    const input = document.createElement("input");
    input.type = "radio";
    input.name = "topup_option";
    input.value = String(option.id || option.option_id || "");
    input.dataset.amount = String(option.amount || "");
    input.dataset.currency = String(option.currency || option.display_currency || "");
    input.required = true;
    input.checked = index === 0;
    const content = document.createElement("span");
    const amount = document.createElement("strong");
    amount.textContent = displayMoney(option);
    const note = document.createElement("small");
    note.textContent = option.label || option.description || "充值金额";
    content.append(amount, note);
    label.append(input, content);
    optionList.append(label);
  }

  for (const channel of channels) {
    const option = document.createElement("option");
    if (typeof channel === "string") {
      option.value = channel;
      option.textContent = channel;
    } else {
      option.value = String(channel.id || channel.code || "");
      option.textContent = channel.name || channel.label || option.value;
      option.disabled = channel.available === false;
    }
    if (option.value) {
      channelSelect.append(option);
    }
  }

  if (options.length === 0 || channelSelect.options.length === 0) {
    throw new Error("No top-up options");
  }

  document.querySelector("[data-topup-loading]").hidden = true;
  document.querySelector("[data-topup-fields]").hidden = false;
  document.querySelector("[data-topup-submit]").disabled = false;
}

async function openTopup() {
  setDialogStatus("topup");
  document.querySelector("[data-topup-loading]").hidden = false;
  document.querySelector("[data-topup-fields]").hidden = true;
  document.querySelector("[data-topup-submit]").disabled = true;
  elements.topupDialog.showModal();

  try {
    const payload = await apiRequest("/wallet/topup-options");
    renderTopupOptions(payload);
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      elements.topupDialog.close();
      requireLogin();
      return;
    }
    document.querySelector("[data-topup-loading]").hidden = true;
    setDialogStatus("topup", "充值方式暂时不可用，请稍后重试。");
  }
}

async function submitTopup(event) {
  event.preventDefault();
  const form = event.currentTarget;
  const selected = form.querySelector('input[name="topup_option"]:checked');
  const channel = form.elements.payment_channel.value;
  if (!selected || !channel) {
    setDialogStatus("topup", "请选择充值金额和支付方式。");
    return;
  }

  const body = { payment_channel: channel };
  if (selected.value) {
    body.option_id = selected.value;
  } else {
    body.amount = selected.dataset.amount;
    body.currency = selected.dataset.currency;
  }

  const submit = document.querySelector("[data-topup-submit]");
  submit.disabled = true;
  submit.textContent = "正在创建订单…";
  setDialogStatus("topup");

  try {
    const payload = await apiRequest("/wallet/topup-orders", {
      method: "POST",
      body,
      idempotencyKey: createIdempotencyKey(),
    });
    const data = unwrap(payload) || {};
    const paymentUrl = data.payment_url || data.checkout_url || data.order?.payment_url;
    if (typeof paymentUrl !== "string") {
      throw new Error("Missing payment URL");
    }
    const target = new URL(paymentUrl);
    if (target.protocol !== "https:") {
      throw new Error("Unsafe payment URL");
    }
    window.location.assign(target.href);
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      elements.topupDialog.close();
      requireLogin();
      return;
    }
    setDialogStatus(
      "topup",
      error instanceof ApiError ? error.message : "订单创建失败，请稍后重试。",
    );
    submit.disabled = false;
    submit.textContent = "前往支付";
  }
}

async function waitForTurnstile() {
  for (let attempt = 0; attempt < 80; attempt += 1) {
    if (window.turnstile?.render) {
      return window.turnstile;
    }
    await new Promise((resolve) => window.setTimeout(resolve, 100));
  }
  throw new Error("Turnstile failed to load");
}

async function initializeEmailTurnstile() {
  const container = document.querySelector("[data-email-turnstile]");
  if (!container || emailTurnstileWidget !== undefined) {
    return;
  }
  const config = await getPublicConfig();
  const siteKey =
    config.turnstile_site_key ||
    (config.turnstile && typeof config.turnstile.site_key === "string"
      ? config.turnstile.site_key
      : "");
  emailTurnstileRequired =
    config.turnstile_required !== false && config.turnstile?.required !== false;
  if (!emailTurnstileRequired) {
    container.hidden = true;
    return;
  }
  if (!siteKey) {
    throw new Error("Missing Turnstile site key");
  }
  const turnstile = await waitForTurnstile();
  container.replaceChildren();
  emailTurnstileWidget = turnstile.render(container, {
    sitekey: siteKey,
    action: "niko_email_code",
    theme: "light",
    size: "flexible",
  });
}

async function openEmailDialog() {
  setDialogStatus("email");
  elements.emailDialog.showModal();
  try {
    await initializeEmailTurnstile();
  } catch {
    setDialogStatus("email", "安全验证暂时不可用，请稍后重试。");
  }
}

function emailTurnstileToken() {
  if (!emailTurnstileRequired) {
    return "";
  }
  if (emailTurnstileWidget === undefined || !window.turnstile) {
    return "";
  }
  return window.turnstile.getResponse(emailTurnstileWidget);
}

function startCodeCooldown(seconds) {
  const button = document.querySelector("[data-send-email-code]");
  let remaining = seconds;
  window.clearInterval(codeCooldownTimer);
  button.disabled = true;
  button.textContent = `${remaining} 秒后重发`;
  codeCooldownTimer = window.setInterval(() => {
    remaining -= 1;
    if (remaining <= 0) {
      window.clearInterval(codeCooldownTimer);
      button.disabled = false;
      button.textContent = "发送验证码";
      return;
    }
    button.textContent = `${remaining} 秒后重发`;
  }, 1000);
}

async function sendEmailCode() {
  const email = document.querySelector('[data-email-form] input[name="email"]');
  if (!email.reportValidity()) {
    return;
  }
  const token = emailTurnstileToken();
  if (emailTurnstileRequired && !token) {
    setDialogStatus("email", "请先完成安全验证。");
    return;
  }

  const button = document.querySelector("[data-send-email-code]");
  button.disabled = true;
  setDialogStatus("email");
  try {
    await apiRequest("/account/email/send-code", {
      method: "POST",
      body: { email: email.value.trim(), turnstile_token: token },
    });
    setDialogStatus("email", "验证码已发送，请检查邮箱。", "success");
    startCodeCooldown(60);
    if (emailTurnstileWidget !== undefined && window.turnstile) {
      window.turnstile.reset(emailTurnstileWidget);
    }
  } catch (error) {
    setDialogStatus(
      "email",
      error instanceof ApiError ? error.message : "验证码发送失败，请稍后重试。",
    );
    button.disabled = false;
  }
}

async function bindEmail(event) {
  event.preventDefault();
  const form = event.currentTarget;
  if (!form.reportValidity()) {
    return;
  }
  const submit = document.querySelector("[data-email-submit]");
  submit.disabled = true;
  setDialogStatus("email");
  try {
    await apiRequest("/account/email/bind", {
      method: "POST",
      body: {
        email: form.elements.email.value.trim(),
        code: form.elements.code.value.trim(),
      },
    });
    const payload = await apiRequest("/account/me");
    renderAccount(accountData(payload));
    elements.emailDialog.close();
    setNotice("邮箱绑定成功。", "success");
    form.elements.code.value = "";
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) {
      elements.emailDialog.close();
      requireLogin();
      return;
    }
    setDialogStatus(
      "email",
      error instanceof ApiError ? error.message : "邮箱绑定失败，请检查验证码后重试。",
    );
  } finally {
    submit.disabled = false;
  }
}

async function logout() {
  elements.logout.disabled = true;
  try {
    await apiRequest("/auth/logout", { method: "POST", body: {} });
    window.location.assign("/");
  } catch (error) {
    setNotice(error instanceof ApiError ? error.message : "退出失败，请稍后重试。");
    elements.logout.disabled = false;
  }
}

function activateTab(kind) {
  for (const tab of document.querySelectorAll("[data-tab]")) {
    const active = tab.dataset.tab === kind;
    tab.setAttribute("aria-selected", String(active));
    tab.tabIndex = active ? 0 : -1;
  }
  for (const panel of document.querySelectorAll("[data-panel]")) {
    panel.hidden = panel.dataset.panel !== kind;
  }
  if (!recordState[kind].loaded) {
    loadRecords(kind, false);
  }
}

for (const tab of document.querySelectorAll("[data-tab]")) {
  tab.addEventListener("click", () => activateTab(tab.dataset.tab));
}
for (const button of document.querySelectorAll("[data-load-more]")) {
  button.addEventListener("click", () => loadRecords(button.dataset.loadMore, true));
}
for (const button of document.querySelectorAll("[data-close-dialog], [data-cancel-dialog]")) {
  button.addEventListener("click", () => button.closest("dialog")?.close());
}

document.querySelector("[data-open-topup]").addEventListener("click", openTopup);
document.querySelector("[data-open-email]").addEventListener("click", openEmailDialog);
document.querySelector("[data-refresh-account]").addEventListener("click", async () => {
  await refreshAccount({ announce: true });
  recordState.topups.loaded = false;
  recordState.consumptions.loaded = false;
  await loadRecords("topups", false);
});
document.querySelector("[data-topup-form]").addEventListener("submit", submitTopup);
document.querySelector("[data-email-form]").addEventListener("submit", bindEmail);
document.querySelector("[data-send-email-code]").addEventListener("click", sendEmailCode);
elements.logout.addEventListener("click", logout);

refreshAccount().then(() => {
  if (account) {
    loadRecords("topups", false);
  }
});
