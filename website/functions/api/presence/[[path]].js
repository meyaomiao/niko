import {
  errorResponse,
  HttpError,
  jsonResponse,
} from "../../_lib/security.js";
import {
  authorizeOps,
  buildStats,
  recordHeartbeat,
} from "../../_lib/presence.js";

function relativePath(requestUrl) {
  const prefix = "/api/presence/";
  return requestUrl.pathname.startsWith(prefix)
    ? requestUrl.pathname.slice(prefix.length).replace(/\/+$/g, "")
    : "";
}

export async function onRequest(context) {
  const { request, env } = context;
  try {
    const method = request.method.toUpperCase();
    if (!["GET", "POST"].includes(method)) {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "请求方法不受支持。");
    }

    const requestUrl = new URL(request.url);
    if ([...requestUrl.searchParams].length > 0) {
      throw new HttpError(400, "INVALID_QUERY", "接口不接受查询参数。");
    }

    const path = relativePath(requestUrl);
    if (method === "POST" && path === "heartbeat") {
      const bodyText = await request.text();
      const result = await recordHeartbeat(env.NIKO_PRESENCE, bodyText);
      return jsonResponse({ data: result });
    }

    if (method === "GET" && path === "stats") {
      authorizeOps(request, env);
      const stats = await buildStats(env.NIKO_PRESENCE);
      return jsonResponse({ data: stats });
    }

    throw new HttpError(404, "ROUTE_NOT_FOUND", "接口不存在。");
  } catch (error) {
    return errorResponse(error);
  }
}
