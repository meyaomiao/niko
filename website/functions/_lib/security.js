export const SESSION_COOKIE = "__Host-niko_session";
export const CSRF_COOKIE = "__Host-niko_csrf";

const encoder = new TextEncoder();

export class HttpError extends Error {
  constructor(status, code, message) {
    super(message);
    this.name = "HttpError";
    this.status = status;
    this.code = code;
  }
}

function bytesToHex(bytes) {
  return [...bytes].map((value) => value.toString(16).padStart(2, "0")).join("");
}

function randomHex(byteLength) {
  const bytes = crypto.getRandomValues(new Uint8Array(byteLength));
  return bytesToHex(bytes);
}

export function createCsrfToken() {
  return randomHex(32);
}

export function parseCookies(request) {
  const result = new Map();
  const header = request.headers.get("Cookie") || "";
  for (const part of header.split(";")) {
    const separator = part.indexOf("=");
    if (separator <= 0) {
      continue;
    }
    const name = part.slice(0, separator).trim();
    const value = part.slice(separator + 1).trim();
    try {
      result.set(decodeURIComponent(name), decodeURIComponent(value));
    } catch {
      continue;
    }
  }
  return result;
}

function cookieValue(value) {
  return encodeURIComponent(value);
}

export function sessionCookie(value, maxAge) {
  return `${SESSION_COOKIE}=${cookieValue(value)}; Path=/; Max-Age=${maxAge}; Secure; HttpOnly; SameSite=Lax`;
}

export function csrfCookie(value, maxAge) {
  return `${CSRF_COOKIE}=${cookieValue(value)}; Path=/; Max-Age=${maxAge}; Secure; SameSite=Lax`;
}

export function clearSessionCookie() {
  return `${SESSION_COOKIE}=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Secure; HttpOnly; SameSite=Lax`;
}

export function clearCsrfCookie() {
  return `${CSRF_COOKIE}=; Path=/; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT; Secure; SameSite=Lax`;
}

function constantTimeEqual(left, right) {
  if (typeof left !== "string" || typeof right !== "string" || left.length !== right.length) {
    return false;
  }
  let difference = 0;
  for (let index = 0; index < left.length; index += 1) {
    difference |= left.charCodeAt(index) ^ right.charCodeAt(index);
  }
  return difference === 0;
}

export function validateMutationRequest(request, cookies) {
  const requestOrigin = request.headers.get("Origin");
  const expectedOrigin = new URL(request.url).origin;
  if (!requestOrigin || requestOrigin !== expectedOrigin) {
    throw new HttpError(403, "ORIGIN_REJECTED", "请求来源校验失败。");
  }

  const cookieToken = cookies.get(CSRF_COOKIE) || "";
  const headerToken = request.headers.get("X-CSRF-Token") || "";
  if (
    !/^[A-Za-z0-9_-]{32,256}$/.test(cookieToken) ||
    !constantTimeEqual(cookieToken, headerToken)
  ) {
    throw new HttpError(403, "CSRF_REJECTED", "页面安全校验已失效，请刷新后重试。");
  }
}

function clampInteger(value, fallback, minimum, maximum) {
  const parsed = Number.parseInt(String(value || ""), 10);
  if (!Number.isFinite(parsed)) {
    return fallback;
  }
  return Math.min(Math.max(parsed, minimum), maximum);
}

export function sessionMaxAge(env, upstreamPayload) {
  const source =
    upstreamPayload?.data && typeof upstreamPayload.data === "object"
      ? upstreamPayload.data
      : upstreamPayload;
  const configured = clampInteger(
    env.NIKO_SESSION_COOKIE_MAX_AGE,
    7 * 24 * 60 * 60,
    300,
    30 * 24 * 60 * 60,
  );
  return clampInteger(source?.session_expires_in, configured, 300, configured);
}

async function sha256Hex(value) {
  const digest = await crypto.subtle.digest("SHA-256", encoder.encode(value));
  return bytesToHex(new Uint8Array(digest));
}

async function hmacHex(secret, value) {
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode(secret),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const signature = await crypto.subtle.sign("HMAC", key, encoder.encode(value));
  return bytesToHex(new Uint8Array(signature));
}

function upstreamOrigin(env, requestUrl) {
  if (!env.MOMOTOKEN_NIKO_API_ORIGIN) {
    throw new HttpError(503, "BFF_NOT_CONFIGURED", "账户服务尚未完成配置。");
  }

  let origin;
  try {
    origin = new URL(env.MOMOTOKEN_NIKO_API_ORIGIN);
  } catch {
    throw new HttpError(503, "BFF_NOT_CONFIGURED", "账户服务地址配置无效。");
  }

  if (
    origin.username ||
    origin.password ||
    origin.search ||
    origin.hash ||
    !["https:", "http:"].includes(origin.protocol)
  ) {
    throw new HttpError(503, "BFF_NOT_CONFIGURED", "账户服务地址配置无效。");
  }

  const localRequest = ["localhost", "127.0.0.1", "::1"].includes(requestUrl.hostname);
  if (!localRequest && origin.protocol !== "https:") {
    throw new HttpError(503, "BFF_NOT_CONFIGURED", "账户服务必须使用 HTTPS。");
  }
  return origin;
}

function connectingIp(request) {
  const ip = request.headers.get("CF-Connecting-IP") || "unknown";
  return /^[0-9A-Fa-f:.]{2,64}$/.test(ip) ? ip : "unknown";
}

export async function createUpstreamRequest({
  request,
  env,
  pathname,
  search,
  method,
  body,
  sessionToken,
  idempotencyKey,
}) {
  const requestUrl = new URL(request.url);
  const base = upstreamOrigin(env, requestUrl);
  const upstreamUrl = new URL(pathname, base);
  upstreamUrl.search = search;

  const clientId = env.MOMOTOKEN_NIKO_CLIENT_ID || "";
  const secret = env.MOMOTOKEN_NIKO_CLIENT_SECRET || "";
  if (!clientId || !secret) {
    throw new HttpError(503, "BFF_NOT_CONFIGURED", "账户服务签名尚未完成配置。");
  }

  const bodyText = body === undefined ? "" : JSON.stringify(body);
  const bodyHash = await sha256Hex(bodyText);
  const timestamp = Math.floor(Date.now() / 1000).toString();
  const nonce = randomHex(16);
  const clientIp = connectingIp(request);
  const requestTarget = `${upstreamUrl.pathname}${upstreamUrl.search}`;
  const canonical = [
    "NIKO-HMAC-V1",
    clientId,
    timestamp,
    nonce,
    method,
    requestTarget,
    bodyHash,
    clientIp,
  ].join("\n");
  const signature = await hmacHex(secret, canonical);

  const headers = new Headers({
    Accept: "application/json",
    "Content-Type": "application/json",
    "User-Agent": "Niko-Website-BFF/1.0",
    "X-Niko-Client-Id": clientId,
    "X-Niko-Timestamp": timestamp,
    "X-Niko-Nonce": nonce,
    "X-Niko-Content-SHA256": bodyHash,
    "X-Niko-Client-IP": clientIp,
    "X-Niko-Signature": `v1=${signature}`,
  });
  if (sessionToken) {
    headers.set("Authorization", `Bearer ${sessionToken}`);
  }
  if (idempotencyKey) {
    headers.set("Idempotency-Key", idempotencyKey);
  }

  const timeout = clampInteger(env.NIKO_UPSTREAM_TIMEOUT_MS, 10_000, 2_000, 20_000);
  return new Request(upstreamUrl, {
    method,
    headers,
    body: body === undefined ? undefined : bodyText,
    redirect: "manual",
    signal: AbortSignal.timeout(timeout),
  });
}

function tokenContainer(payload) {
  if (payload?.data && typeof payload.data === "object" && !Array.isArray(payload.data)) {
    return payload.data;
  }
  return payload && typeof payload === "object" && !Array.isArray(payload) ? payload : null;
}

export function removeAuthTokens(payload) {
  const container = tokenContainer(payload);
  if (!container) {
    return { payload, sessionToken: "", csrfToken: "" };
  }

  const sessionToken =
    typeof container.session_token === "string" ? container.session_token : "";
  const csrfToken = typeof container.csrf_token === "string" ? container.csrf_token : "";
  delete container.session_token;
  delete container.csrf_token;

  return { payload, sessionToken, csrfToken };
}

export function validSessionToken(value) {
  return typeof value === "string" && /^[A-Za-z0-9._~-]{32,1024}$/.test(value);
}

export function validCsrfToken(value) {
  return typeof value === "string" && /^[A-Za-z0-9_-]{32,256}$/.test(value);
}

function allowedPaymentOrigins(env) {
  return new Set(
    String(env.NIKO_PAYMENT_ALLOWED_ORIGINS || "")
      .split(",")
      .map((value) => value.trim())
      .filter(Boolean)
      .map((value) => {
        try {
          return new URL(value).origin;
        } catch {
          return "";
        }
      })
      .filter(Boolean),
  );
}

export function validatePaymentResponse(payload, env) {
  const data =
    payload?.data && typeof payload.data === "object" ? payload.data : payload || {};
  const paymentUrl = data.payment_url || data.checkout_url || data.order?.payment_url;
  if (typeof paymentUrl !== "string") {
    throw new HttpError(502, "INVALID_PAYMENT_RESPONSE", "支付服务返回了无效结果。");
  }

  let target;
  try {
    target = new URL(paymentUrl);
  } catch {
    throw new HttpError(502, "INVALID_PAYMENT_RESPONSE", "支付服务返回了无效地址。");
  }

  const allowedOrigins = allowedPaymentOrigins(env);
  if (
    target.protocol !== "https:" ||
    target.username ||
    target.password ||
    !allowedOrigins.has(target.origin)
  ) {
    throw new HttpError(502, "PAYMENT_URL_REJECTED", "支付地址未通过安全校验。");
  }
}

export function jsonResponse(payload, { status = 200, headers } = {}) {
  const responseHeaders = new Headers(headers);
  responseHeaders.set("Content-Type", "application/json; charset=utf-8");
  responseHeaders.set("Cache-Control", "no-store");
  responseHeaders.set("Pragma", "no-cache");
  responseHeaders.set("X-Content-Type-Options", "nosniff");
  return new Response(JSON.stringify(payload), { status, headers: responseHeaders });
}

export function errorResponse(error) {
  const known = error instanceof HttpError;
  return jsonResponse(
    {
      error: {
        code: known ? error.code : "BFF_ERROR",
        message: known ? error.message : "账户服务暂时不可用，请稍后重试。",
      },
    },
    { status: known ? error.status : 500 },
  );
}
