export const API_REQUEST_TIMEOUT_MS = 10_000;

export class ApiRequestError extends Error {
  readonly status?: number;
  readonly code?: string;
  readonly payload?: unknown;

  constructor(message: string, options: { status?: number; code?: string; payload?: unknown } = {}) {
    super(message);
    this.name = "ApiRequestError";
    this.status = options.status;
    this.code = options.code;
    this.payload = options.payload;
  }
}

function responseError(status: number, payload?: unknown): ApiRequestError {
  const body = payload && typeof payload === "object"
    ? payload as { message?: unknown; code?: unknown }
    : {};
  // Never show a proxy's HTML response or an unbounded response body in the UI.
  const message = typeof body.message === "string"
    && body.message.length <= 160
    && !/[<>\u0000-\u001f]/.test(body.message)
    ? body.message.trim()
    : "";
  return new ApiRequestError(`${message || "服务请求失败"}（HTTP ${status}）`, {
    status,
    code: typeof body.code === "string" ? body.code : "http",
    payload,
  });
}

/** One deadline covers both response headers and the complete JSON body. */
export async function requestJson<T>(
  url: string,
  init: RequestInit = {},
  timeoutMs = API_REQUEST_TIMEOUT_MS,
): Promise<T> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let onAbort: (() => void) | undefined;
  const stopped = new Promise<never>((_resolve, reject) => {
    onAbort = () => {
      controller.abort();
      reject(init.signal?.reason ?? new DOMException("请求已取消", "AbortError"));
    };
    if (init.signal?.aborted) {
      onAbort();
      return;
    }
    init.signal?.addEventListener("abort", onAbort, { once: true });
    timer = setTimeout(() => {
      // Reject independently as well: callers must recover even if a transport
      // or a response-body reader does not settle after abort().
      reject(new ApiRequestError("服务请求超时，请检查网络或服务状态后重试", { code: "timeout" }));
      controller.abort();
    }, timeoutMs);
  });
  try {
    if (init.signal?.aborted) return await stopped;
    return await Promise.race([
      stopped,
      (async () => {
        const response = await fetch(url, { ...init, signal: controller.signal });
        let payload: unknown;
        try {
          payload = await response.json();
        } catch (error) {
          if (controller.signal.aborted) throw error;
          if (!response.ok) throw responseError(response.status);
          throw new ApiRequestError("服务返回了无效数据，请稍后重试", { code: "invalid_response" });
        }
        if (!response.ok) throw responseError(response.status, payload);
        return payload as T;
      })(),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
    if (onAbort) init.signal?.removeEventListener("abort", onAbort);
  }
}
