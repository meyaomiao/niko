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
