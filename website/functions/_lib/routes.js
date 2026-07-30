import { HttpError } from "./security.js";

const routes = new Map([
  ["POST auth/register", { id: "register", protected: false, body: "register" }],
  ["POST auth/login", { id: "login", protected: false, body: "login" }],
  ["GET auth/session", { id: "session", protected: false }],
  ["POST auth/logout", { id: "logout", protected: true, body: "empty" }],
  ["GET account/me", { id: "account", protected: true }],
  [
    "POST account/email/send-code",
    { id: "emailCode", protected: true, body: "emailCode" },
  ],
  ["POST account/email/bind", { id: "emailBind", protected: true, body: "emailBind" }],
  ["GET wallet/summary", { id: "walletSummary", protected: true }],
  ["GET wallet/topups", { id: "topups", protected: true, query: "cursor" }],
  [
    "GET wallet/consumptions",
    { id: "consumptions", protected: true, query: "cursor" },
  ],
  ["GET wallet/topup-options", { id: "topupOptions", protected: true }],
  [
    "POST wallet/topup-orders",
    { id: "topupCreate", protected: true, body: "topupCreate", idempotent: true },
  ],
]);

function normalizedPath(path) {
  const value = String(path || "").replace(/^\/+|\/+$/g, "");
  if (!value || value.includes("..") || value.includes("%2f") || value.includes("%2F")) {
    throw new HttpError(404, "ROUTE_NOT_FOUND", "接口不存在。");
  }
  return value;
}

export function matchRoute(method, rawPath) {
  const path = normalizedPath(rawPath);
  const staticRoute = routes.get(`${method} ${path}`);
  if (staticRoute) {
    return { ...staticRoute, path };
  }

  const orderMatch = /^wallet\/topup-orders\/([A-Za-z0-9_-]{1,128})$/.exec(path);
  if (method === "GET" && orderMatch) {
    return {
      id: "topupOrder",
      protected: true,
      path,
      orderId: orderMatch[1],
    };
  }

  throw new HttpError(404, "ROUTE_NOT_FOUND", "接口不存在。");
}

function assertAllowedKeys(body, allowed) {
  for (const key of Object.keys(body)) {
    if (!allowed.includes(key)) {
      throw new HttpError(400, "INVALID_REQUEST", `不支持字段：${key}`);
    }
  }
}

function requiredString(body, key, { min = 1, max = 255, trim = false } = {}) {
  if (typeof body[key] !== "string") {
    throw new HttpError(400, "INVALID_REQUEST", `字段 ${key} 格式无效。`);
  }
  const value = trim ? body[key].trim() : body[key];
  if (value.length < min || value.length > max || /[\u0000-\u001f\u007f]/.test(value)) {
    throw new HttpError(400, "INVALID_REQUEST", `字段 ${key} 格式无效。`);
  }
  return value;
}

async function readJsonObject(request) {
  const contentType = request.headers.get("Content-Type") || "";
  if (!contentType.toLowerCase().startsWith("application/json")) {
    throw new HttpError(415, "CONTENT_TYPE_REQUIRED", "请求必须使用 JSON。");
  }

  const length = Number.parseInt(request.headers.get("Content-Length") || "0", 10);
  if (Number.isFinite(length) && length > 32_768) {
    throw new HttpError(413, "REQUEST_TOO_LARGE", "请求内容过大。");
  }

  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > 32_768) {
    throw new HttpError(413, "REQUEST_TOO_LARGE", "请求内容过大。");
  }

  let value;
  try {
    value = text ? JSON.parse(text) : {};
  } catch {
    throw new HttpError(400, "INVALID_JSON", "JSON 格式无效。");
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new HttpError(400, "INVALID_REQUEST", "请求内容格式无效。");
  }
  return value;
}

function turnstileToken(body, allowMissingTurnstile) {
  if (allowMissingTurnstile && body.turnstile_token === "") {
    return "";
  }
  return requiredString(body, "turnstile_token", { min: 20, max: 4096 });
}

function registerBody(body, allowMissingTurnstile) {
  assertAllowedKeys(body, ["username", "password", "turnstile_token"]);
  return {
    username: requiredString(body, "username", { min: 2, max: 20, trim: true }),
    password: requiredString(body, "password", { min: 8, max: 20 }),
    turnstile_token: turnstileToken(body, allowMissingTurnstile),
  };
}

function loginBody(body, allowMissingTurnstile) {
  assertAllowedKeys(body, ["account", "password", "turnstile_token"]);
  return {
    username: requiredString(body, "account", { min: 1, max: 254, trim: true }),
    password: requiredString(body, "password", { min: 8, max: 128 }),
    turnstile_token: turnstileToken(body, allowMissingTurnstile),
  };
}

function emailCodeBody(body, allowMissingTurnstile) {
  assertAllowedKeys(body, ["email", "turnstile_token"]);
  const email = requiredString(body, "email", { min: 3, max: 254, trim: true });
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    throw new HttpError(400, "INVALID_REQUEST", "邮箱格式无效。");
  }
  return {
    email,
    turnstile_token: turnstileToken(body, allowMissingTurnstile),
  };
}

function emailBindBody(body) {
  assertAllowedKeys(body, ["email", "code"]);
  const email = requiredString(body, "email", { min: 3, max: 254, trim: true });
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
    throw new HttpError(400, "INVALID_REQUEST", "邮箱格式无效。");
  }
  return {
    email,
    code: requiredString(body, "code", { min: 4, max: 12, trim: true }),
  };
}

function topupBody(body) {
  assertAllowedKeys(body, [
    "option_id",
    "amount",
    "currency",
    "payment_channel",
  ]);
  const result = {
    payment_channel: requiredString(body, "payment_channel", {
      min: 1,
      max: 64,
      trim: true,
    }),
  };
  if (!/^[A-Za-z0-9_-]+$/.test(result.payment_channel)) {
    throw new HttpError(400, "INVALID_REQUEST", "支付渠道格式无效。");
  }

  const usesOption = Object.hasOwn(body, "option_id");
  const usesAmount = Object.hasOwn(body, "amount") || Object.hasOwn(body, "currency");
  if (usesOption === usesAmount) {
    throw new HttpError(400, "INVALID_REQUEST", "充值档位与自定义金额只能选择一种。");
  }

  if (usesOption) {
    result.option_id = requiredString(body, "option_id", {
      min: 1,
      max: 128,
      trim: true,
    });
    if (!/^[A-Za-z0-9_-]+$/.test(result.option_id)) {
      throw new HttpError(400, "INVALID_REQUEST", "充值档位格式无效。");
    }
    return result;
  }

  const amount = requiredString(body, "amount", { min: 1, max: 16, trim: true });
  if (!/^(?:0|[1-9]\d{0,7})(?:\.\d{1,2})?$/.test(amount) || Number(amount) <= 0) {
    throw new HttpError(400, "INVALID_REQUEST", "充值金额格式无效。");
  }
  const currency = requiredString(body, "currency", { min: 3, max: 3, trim: true });
  if (!/^[A-Z]{3}$/.test(currency)) {
    throw new HttpError(400, "INVALID_REQUEST", "币种格式无效。");
  }
  const [whole, fraction = ""] = amount.split(".");
  result.amount_minor = Number(whole) * 100 + Number(fraction.padEnd(2, "0"));
  result.currency = currency;
  return result;
}

export async function parseBody(
  request,
  route,
  { allowMissingTurnstile = false } = {},
) {
  if (!route.body) {
    return undefined;
  }
  const body = await readJsonObject(request);
  switch (route.body) {
    case "register":
      return registerBody(body, allowMissingTurnstile);
    case "login":
      return loginBody(body, allowMissingTurnstile);
    case "emailCode":
      return emailCodeBody(body, allowMissingTurnstile);
    case "emailBind":
      return emailBindBody(body);
    case "topupCreate":
      return topupBody(body);
    case "empty":
      assertAllowedKeys(body, []);
      return {};
    default:
      throw new HttpError(500, "BFF_ROUTE_ERROR", "接口配置无效。");
  }
}

export function filteredSearch(requestUrl, route) {
  if (!route.query) {
    if ([...requestUrl.searchParams].length > 0) {
      throw new HttpError(400, "INVALID_QUERY", "此接口不接受查询参数。");
    }
    return "";
  }

  const output = new URLSearchParams();
  for (const [key, value] of requestUrl.searchParams) {
    if (!["cursor", "limit"].includes(key)) {
      throw new HttpError(400, "INVALID_QUERY", `不支持查询参数：${key}`);
    }
    if (key === "cursor") {
      if (!value || value.length > 512 || /[\u0000-\u001f\u007f]/.test(value)) {
        throw new HttpError(400, "INVALID_QUERY", "分页游标格式无效。");
      }
      output.set(key, value);
    } else {
      const limit = Number.parseInt(value, 10);
      if (!/^\d+$/.test(value) || limit < 1 || limit > 50) {
        throw new HttpError(400, "INVALID_QUERY", "分页数量必须在 1 到 50 之间。");
      }
      output.set(key, String(limit));
    }
  }
  return output.toString() ? `?${output.toString()}` : "";
}

export function idempotencyKey(request, route) {
  const value = request.headers.get("Idempotency-Key") || "";
  if (!route.idempotent) {
    return "";
  }
  if (!/^[A-Za-z0-9_-]{16,128}$/.test(value)) {
    throw new HttpError(400, "IDEMPOTENCY_KEY_REQUIRED", "充值请求缺少有效幂等键。");
  }
  return value;
}
