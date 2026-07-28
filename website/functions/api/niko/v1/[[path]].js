import {
  CSRF_COOKIE,
  SESSION_COOKIE,
  HttpError,
  clearCsrfCookie,
  clearSessionCookie,
  createCsrfToken,
  createUpstreamRequest,
  csrfCookie,
  errorResponse,
  jsonResponse,
  parseCookies,
  removeAuthTokens,
  sessionCookie,
  sessionMaxAge,
  validCsrfToken,
  validSessionToken,
  validateMutationRequest,
  validatePaymentResponse,
} from "../../../_lib/security.js";
import {
  filteredSearch,
  idempotencyKey,
  matchRoute,
  parseBody,
} from "../../../_lib/routes.js";

function relativePath(requestUrl) {
  const prefix = "/api/niko/v1/";
  return requestUrl.pathname.startsWith(prefix)
    ? requestUrl.pathname.slice(prefix.length)
    : "";
}

function localHostname(hostname) {
  return ["localhost", "127.0.0.1", "::1"].includes(hostname);
}

function localTurnstileDisabled(request, env) {
  const requestUrl = new URL(request.url);
  return (
    localHostname(requestUrl.hostname) &&
    env.NIKO_LOCAL_TURNSTILE_DISABLED === "true"
  );
}

function configResponse(request, env, cookies) {
  const turnstileDisabled = localTurnstileDisabled(request, env);
  const siteKey = env.NIKO_TURNSTILE_SITE_KEY || "";
  if (!siteKey && !turnstileDisabled) {
    throw new HttpError(503, "TURNSTILE_NOT_CONFIGURED", "安全验证尚未完成配置。");
  }

  const existingToken = cookies.get(CSRF_COOKIE) || "";
  const csrfToken = validCsrfToken(existingToken) ? existingToken : createCsrfToken();
  const maxAge = sessionMaxAge(env, null);
  const headers = new Headers();
  headers.append("Set-Cookie", csrfCookie(csrfToken, maxAge));

  return jsonResponse(
    {
      data: {
        csrf_token: csrfToken,
        turnstile_site_key: siteKey,
        turnstile_required: !turnstileDisabled,
      },
    },
    { headers },
  );
}

async function parseUpstreamPayload(response) {
  const contentType = response.headers.get("Content-Type") || "";
  if (!contentType.toLowerCase().includes("application/json")) {
    throw new HttpError(502, "INVALID_UPSTREAM_RESPONSE", "账户服务返回了无效结果。");
  }
  try {
    return await response.json();
  } catch {
    throw new HttpError(502, "INVALID_UPSTREAM_RESPONSE", "账户服务返回了无效结果。");
  }
}

function responseHeaders(upstreamResponse) {
  const headers = new Headers();
  for (const name of ["Retry-After", "X-Request-Id"]) {
    const value = upstreamResponse.headers.get(name);
    if (value) {
      headers.set(name, value);
    }
  }
  return headers;
}

export async function onRequest(context) {
  const { request, env } = context;
  try {
    const method = request.method.toUpperCase();
    if (!["GET", "POST"].includes(method)) {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "请求方法不受支持。");
    }

    const requestUrl = new URL(request.url);
    const path = relativePath(requestUrl);
    const cookies = parseCookies(request);

    if (method === "GET" && path.replace(/\/+$/g, "") === "config") {
      if ([...requestUrl.searchParams].length > 0) {
        throw new HttpError(400, "INVALID_QUERY", "配置接口不接受查询参数。");
      }
      return configResponse(request, env, cookies);
    }

    const route = matchRoute(method, path);
    if (method !== "GET") {
      validateMutationRequest(request, cookies);
    }

    const sessionToken = cookies.get(SESSION_COOKIE) || "";
    if (route.protected && !validSessionToken(sessionToken)) {
      const headers = new Headers();
      headers.append("Set-Cookie", clearSessionCookie());
      headers.append("Set-Cookie", clearCsrfCookie());
      return jsonResponse(
        { error: { code: "UNAUTHENTICATED", message: "请先登录。" } },
        { status: 401, headers },
      );
    }

    const search = filteredSearch(requestUrl, route);
    const body = await parseBody(request, route, {
      allowMissingTurnstile: localTurnstileDisabled(request, env),
    });
    const requestIdempotencyKey = idempotencyKey(request, route);
    const upstreamSessionToken =
      (route.protected || route.id === "session") && validSessionToken(sessionToken)
        ? sessionToken
        : "";
    const upstreamCsrfToken =
      method !== "GET" && upstreamSessionToken
        ? cookies.get(CSRF_COOKIE) || ""
        : "";
    const upstreamRequest = await createUpstreamRequest({
      request,
      env,
      pathname: `/api/niko/v1/${route.path}`,
      search,
      method,
      body,
      sessionToken: upstreamSessionToken,
      csrfToken: upstreamCsrfToken,
      idempotencyKey: requestIdempotencyKey,
    });

    let upstreamResponse;
    try {
      upstreamResponse = await fetch(upstreamRequest);
    } catch {
      throw new HttpError(502, "UPSTREAM_UNAVAILABLE", "账户服务暂时不可用，请稍后重试。");
    }

    const originalPayload = await parseUpstreamPayload(upstreamResponse);
    const {
      payload,
      sessionToken: nextSessionToken,
      csrfToken: nextCsrfToken,
    } = removeAuthTokens(originalPayload);

    if (route.id === "login" && upstreamResponse.ok) {
      if (!validSessionToken(nextSessionToken) || !validCsrfToken(nextCsrfToken)) {
        throw new HttpError(502, "INVALID_AUTH_RESPONSE", "登录服务返回了无效结果。");
      }
    }
    if (route.id === "topupCreate" && upstreamResponse.ok) {
      validatePaymentResponse(payload, env);
    }

    const headers = responseHeaders(upstreamResponse);
    const maxAge = sessionMaxAge(env, originalPayload);
    if (validSessionToken(nextSessionToken)) {
      headers.append("Set-Cookie", sessionCookie(nextSessionToken, maxAge));
    }
    if (validCsrfToken(nextCsrfToken)) {
      headers.append("Set-Cookie", csrfCookie(nextCsrfToken, maxAge));
    }

    if (
      upstreamResponse.status === 401 ||
      (route.id === "logout" && upstreamResponse.ok)
    ) {
      headers.append("Set-Cookie", clearSessionCookie());
      headers.append("Set-Cookie", clearCsrfCookie());
    }

    return jsonResponse(payload, {
      status: upstreamResponse.status,
      headers,
    });
  } catch (error) {
    return errorResponse(error);
  }
}
