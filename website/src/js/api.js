const API_PREFIX = "/api/niko/v1";
const CSRF_COOKIE = "__Host-niko_csrf";

let configPromise;
let csrfTokenFromConfig = "";

export class ApiError extends Error {
  constructor(message, { status = 0, code = "", retryAfter = 0, payload = null } = {}) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
    this.retryAfter = retryAfter;
    this.payload = payload;
  }
}

export function isRetryableApiError(error) {
  if (!(error instanceof ApiError)) {
    return false;
  }
  const code = String(error.code || "").toUpperCase();
  return (
    code === "NETWORK_ERROR" ||
    code === "TIMEOUT" ||
    code === "RATE_LIMITED" ||
    error.status === 408 ||
    error.status === 429 ||
    (error.status >= 500 && error.status <= 599)
  );
}

export function friendlyApiError(error, fallback = "操作无法完成，请联系支持。") {
  if (!(error instanceof ApiError)) {
    return fallback;
  }
  const code = String(error.code || "").toUpperCase();
  const text = `${code} ${error.message || ""}`.toLowerCase();
  if (["INVALID_CREDENTIALS", "AUTH_FAILED", "AUTH_INVALID_CREDENTIALS"].includes(code)) {
    return "账号或密码不正确，请检查后再试。";
  }
  if (error.status === 401 || /session|登录状态|未登录|过期/.test(text)) {
    return "登录状态已过期，请重新登录后再试。";
  }
  if (
    error.status === 402 ||
    /余额不足|余额|quota|insufficient|balance/.test(text)
  ) {
    return "余额不足，请先充值后再试。";
  }
  if (code === "NETWORK_ERROR" || code === "TIMEOUT") {
    return "网络连接失败，请检查网络后重试。";
  }
  if (error.status === 429 || code === "RATE_LIMITED") {
    return error.retryAfter > 0
      ? `操作过于频繁，请在 ${error.retryAfter} 秒后重试。`
      : "操作过于频繁，请稍后重试。";
  }
  if (["TURNSTILE_FAILED", "TURNSTILE_REQUIRED", "VERIFICATION_REQUIRED"].includes(code)) {
    return "安全验证未通过，请重新验证。";
  }
  if (["USERNAME_TAKEN", "ACCOUNT_CONFLICT"].includes(code)) {
    return "这个用户名暂不可用，请换一个再试。";
  }
  if (code === "PASSWORD_MISMATCH") {
    return "两次输入的密码不一致。";
  }
  if (code.startsWith("TOPUP") || /支付|订单|充值/.test(text)) {
    return isRetryableApiError(error)
      ? "充值服务暂时不可用，请稍后重试。"
      : "充值信息无法处理，请检查充值金额和支付方式。";
  }
  if (error.status === 400) {
    return "提交的信息不符合要求，请检查输入后重新提交。";
  }
  if (error.status === 403) {
    return "当前账号没有权限完成这项操作，请联系支持。";
  }
  if (error.status === 404) {
    return "没有找到这项内容，请回到个人中心查看最新记录。";
  }
  if (error.status === 409) {
    return "当前页面状态已变化，请刷新页面后再操作。";
  }
  if (error.status === 408) {
    return "请求超时，请稍后重试。";
  }
  if (error.status >= 500 && error.status <= 599) {
    return "账户服务暂时不可用，请稍后重试。";
  }
  if (error.status >= 400 && error.status <= 499) {
    return "请求无法完成，请联系支持确认这项操作。";
  }
  return "操作无法完成，请联系支持。";
}

function readCookie(name) {
  const prefix = `${encodeURIComponent(name)}=`;
  for (const part of document.cookie.split(";")) {
    const cookie = part.trim();
    if (cookie.startsWith(prefix)) {
      return decodeURIComponent(cookie.slice(prefix.length));
    }
  }
  return "";
}

async function parseResponse(response) {
  if (response.status === 204) {
    return null;
  }

  const contentType = response.headers.get("content-type") || "";
  if (!contentType.includes("application/json")) {
    const text = await response.text();
    return text ? { message: text } : null;
  }

  try {
    return await response.json();
  } catch {
    return null;
  }
}

function errorDetails(payload) {
  const source =
    payload && typeof payload === "object" && payload.error && typeof payload.error === "object"
      ? payload.error
      : payload;

  return {
    code: typeof source?.code === "string" ? source.code : "",
    message:
      (typeof source?.message === "string" && source.message) ||
      (typeof payload?.message === "string" && payload.message) ||
      "请求失败，请稍后重试。",
  };
}

async function loadConfig(force = false) {
  if (!configPromise || force) {
    configPromise = fetch(`${API_PREFIX}/config`, {
      method: "GET",
      credentials: "same-origin",
      headers: { Accept: "application/json" },
      cache: "no-store",
    }).then(async (response) => {
      const payload = await parseResponse(response);
      if (!response.ok) {
        const details = errorDetails(payload);
        throw new ApiError(details.message, {
          status: response.status,
          code: details.code,
          payload,
        });
      }
      const data = unwrap(payload) || {};
      csrfTokenFromConfig =
        typeof data.csrf_token === "string" ? data.csrf_token : csrfTokenFromConfig;
      return data;
    });
  }

  return configPromise;
}

export function unwrap(payload) {
  if (
    payload &&
    typeof payload === "object" &&
    !Array.isArray(payload) &&
    Object.prototype.hasOwnProperty.call(payload, "data")
  ) {
    return payload.data;
  }
  return payload;
}

export async function getPublicConfig() {
  return loadConfig();
}

export async function apiRequest(
  path,
  { method = "GET", body, headers = {}, idempotencyKey = "" } = {},
) {
  if (typeof path !== "string" || !path.startsWith("/") || path.startsWith("//")) {
    throw new TypeError("API path must be same-origin");
  }

  const normalizedMethod = method.toUpperCase();
  const requestHeaders = new Headers(headers);
  requestHeaders.set("Accept", "application/json");

  if (!["GET", "HEAD"].includes(normalizedMethod)) {
    await loadConfig();
    let csrfToken = readCookie(CSRF_COOKIE) || csrfTokenFromConfig;
    if (!csrfToken) {
      await loadConfig(true);
      csrfToken = readCookie(CSRF_COOKIE) || csrfTokenFromConfig;
    }
    if (!csrfToken) {
      throw new ApiError("安全校验初始化失败，请刷新页面后重试。", {
        code: "CSRF_UNAVAILABLE",
      });
    }
    requestHeaders.set("X-CSRF-Token", csrfToken);
  }

  let requestBody;
  if (body !== undefined) {
    requestHeaders.set("Content-Type", "application/json");
    requestBody = JSON.stringify(body);
  }
  if (idempotencyKey) {
    requestHeaders.set("Idempotency-Key", idempotencyKey);
  }

  let response;
  try {
    response = await fetch(`${API_PREFIX}${path}`, {
      method: normalizedMethod,
      credentials: "same-origin",
      headers: requestHeaders,
      body: requestBody,
      cache: "no-store",
    });
  } catch {
    throw new ApiError("网络连接失败，请检查连接后重试。", {
      code: "NETWORK_ERROR",
    });
  }

  const payload = await parseResponse(response);
  if (!response.ok) {
    const details = errorDetails(payload);
    const retryAfter = Number.parseInt(response.headers.get("retry-after") || "0", 10);
    throw new ApiError(details.message, {
      status: response.status,
      code: details.code,
      retryAfter: Number.isFinite(retryAfter) ? retryAfter : 0,
      payload,
    });
  }

  return payload;
}

export function createIdempotencyKey() {
  if (typeof crypto.randomUUID === "function") {
    return crypto.randomUUID();
  }
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 0x0f) | 0x40;
  bytes[8] = (bytes[8] & 0x3f) | 0x80;
  const hex = [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`;
}

export function safeInternalNext(rawValue, fallback = "/account/") {
  if (typeof rawValue !== "string" || !rawValue.startsWith("/") || rawValue.startsWith("//")) {
    return fallback;
  }
  try {
    const resolved = new URL(rawValue, window.location.origin);
    return resolved.origin === window.location.origin
      ? `${resolved.pathname}${resolved.search}${resolved.hash}`
      : fallback;
  } catch {
    return fallback;
  }
}

export function formatDate(value, { includeTime = true } = {}) {
  if (!value) {
    return "—";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "—";
  }
  return new Intl.DateTimeFormat("zh-CN", {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    ...(includeTime ? { hour: "2-digit", minute: "2-digit" } : {}),
  }).format(date);
}

export function displayMoney(source, amountKey = "amount") {
  if (!source || typeof source !== "object") {
    return "—";
  }

  const preformatted =
    source.display_amount ?? source.display_balance ?? source.formatted_amount ?? null;
  if (typeof preformatted === "string" && preformatted.trim()) {
    return preformatted.trim();
  }

  const amount = source[amountKey];
  const currency = source.display_currency || source.currency;
  if (
    (typeof amount === "string" || typeof amount === "number") &&
    typeof currency === "string" &&
    /^[A-Z]{3}$/.test(currency)
  ) {
    const numeric = Number(amount);
    if (Number.isFinite(numeric)) {
      return new Intl.NumberFormat("zh-CN", {
        style: "currency",
        currency,
      }).format(numeric);
    }
  }

  return "—";
}
