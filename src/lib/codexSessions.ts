export const SAFE_ERROR_VERSION = 1 as const;
export const SESSION_PAGE_SIZE = 20;
export const SESSION_QUERY_LIMIT = 80;

const FALLBACK_MESSAGE = "操作失败，请稍后再试。";
const SAFE_ERRORS: Record<string, ReadonlyArray<{
  message: string;
  retryable: boolean;
  action?: string;
}>> = {
  invalid_request: [{ message: "请求无效，请重新检查。", retryable: false }],
  read_failed: [{ message: "本地内容暂时无法读取。", retryable: true, action: "retry" }],
  busy: [{ message: "另一个操作正在进行，请稍后再试。", retryable: true, action: "retry" }],
  change_failed: [
    { message: "操作未完成，原有内容保持可用。", retryable: false },
    { message: "操作未完成，原有内容保持可用。", retryable: true, action: "retry" },
  ],
  open_failed: [{ message: "未能打开应用，请手动打开。", retryable: true, action: "retry" }],
};

export interface SafeCommandError {
  version: typeof SAFE_ERROR_VERSION;
  code: string;
  message: string;
  retryable: boolean;
  action?: string;
}

export interface CodexSessionThread {
  thread_id: string;
  title?: string | null;
  summary?: string | null;
  updated_at?: string | null;
  archived: boolean;
  can_continue: boolean;
}

export interface CodexSessionPage {
  status: "healthy" | "needs_check" | "blocked";
  items: CodexSessionThread[];
  next_cursor?: string | null;
}

export interface CodexSessionMutationOutcome {
  status: "applied" | "unchanged" | "applied_needs_manual_open";
  message: string;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function parseSafeCommandError(value: unknown): SafeCommandError | null {
  if (!isRecord(value) || value.version !== SAFE_ERROR_VERSION) return null;
  if (Object.keys(value).some((key) => !["version", "code", "message", "retryable", "action"].includes(key))) return null;
  if (typeof value.code !== "string" || !/^[a-z0-9_]{1,48}$/.test(value.code)) return null;
  if (typeof value.message !== "string" || value.message.length < 1 || value.message.length > 160) return null;
  if (typeof value.retryable !== "boolean") return null;
  if (value.action !== undefined && (typeof value.action !== "string" || !/^[a-z_]{1,32}$/.test(value.action))) return null;
  const allowed = SAFE_ERRORS[value.code]?.some((candidate) =>
    candidate.message === value.message
      && candidate.retryable === value.retryable
      && candidate.action === value.action
  );
  if (!allowed) return null;
  return value as unknown as SafeCommandError;
}

export function safeFailure(value: unknown): SafeCommandError {
  return parseSafeCommandError(value) ?? {
    version: SAFE_ERROR_VERSION,
    code: "unknown_failure",
    message: FALLBACK_MESSAGE,
    retryable: false,
  };
}

export function boundSessionQuery(value: string): string {
  return Array.from(value.trim()).slice(0, SESSION_QUERY_LIMIT).join("");
}

export type RequestKind = "scan" | "action" | "detect";
export interface RequestGuard {
  mounted: boolean;
  scan: number;
  action: number;
  detect: number;
}

export function initialRequestGuard(): RequestGuard {
  return { mounted: true, scan: 0, action: 0, detect: 0 };
}

export function beginRequest(state: RequestGuard, kind: RequestKind) {
  const generation = state[kind] + 1;
  return { state: { ...state, [kind]: generation }, generation };
}

export function unmountRequests(state: RequestGuard): RequestGuard {
  return {
    mounted: false,
    scan: state.scan + 1,
    action: state.action + 1,
    detect: state.detect + 1,
  };
}

export function mountRequests(state: RequestGuard): RequestGuard {
  return { ...state, mounted: true };
}

export function acceptsResponse(
  state: RequestGuard,
  kind: RequestKind,
  generation: number,
): boolean {
  return state.mounted && state[kind] === generation;
}

export function codexNormalizationLabel(status: string): string {
  if (status === "healthy") return "当前状态正常";
  if (status === "needs_check") return "发现需要整理的会话";
  if (status === "blocked") return "有部分会话暂时无法继续";
  return "状态待检查";
}
