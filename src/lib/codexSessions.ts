export const SAFE_ERROR_VERSION = 1 as const;
export const SESSION_PAGE_SIZE = 50;
export const SESSION_QUERY_LIMIT = 80;
export const UNNAMED_SESSION_TITLE = "未命名会话";
export const CODEX_SESSION_SYNC_PROGRESS_EVENT = "codex-session-sync-progress";

const FALLBACK_MESSAGE = "操作没有完成，请重试。";
const SAFE_ERRORS: Record<string, ReadonlyArray<{
  message: string;
  retryable: boolean;
  action?: string;
}>> = {
  invalid_request: [{ message: "请求无效，请重新检查。", retryable: false }],
  read_failed: [{ message: "会话暂时无法读取。", retryable: true, action: "retry" }],
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
  title: string;
  summary: null;
  updated_at?: string | null;
  archived: boolean;
  provider?: string | null;
  can_continue: boolean;
  needs_migration: boolean;
  blockers: CodexSessionBlocker[];
}

export interface CodexSessionBlocker {
  title: string;
  thread_id: string;
  reason: string;
  next_step: string;
}

export interface CodexSessionPage {
  status: "healthy" | "needs_check" | "blocked";
  items: CodexSessionThread[];
  blockers: CodexSessionBlocker[];
  page: number;
  page_size: number;
  total: number;
  total_pages: number;
}

export interface CodexSessionMutationOutcome {
  status: "applied" | "unchanged" | "applied_needs_manual_open";
  message: string;
  requested: number;
  migrated: number;
  failed: number;
  changed_artifacts: number;
}

export type CodexSessionSyncPhase =
  | "preparing"
  | "backing_up"
  | "staging"
  | "committing"
  | "validating"
  | "completed";

export interface CodexSessionSyncProgress {
  phase: CodexSessionSyncPhase;
  percent: number;
  processed: number;
  total: number;
  target_provider: "custom" | "openai";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function normalizeCodexSessionSyncProgress(value: unknown): CodexSessionSyncProgress | null {
  if (!isRecord(value)) return null;
  const phase = value.phase;
  const targetProvider = value.target_provider;
  const numericValues = [value.percent, value.processed, value.total];
  if (
    !["preparing", "backing_up", "staging", "committing", "validating", "completed"].includes(String(phase))
    || (targetProvider !== "custom" && targetProvider !== "openai")
    || numericValues.some((item) => typeof item !== "number" || !Number.isInteger(item) || item < 0)
    || Number(value.percent) > 100
    || Number(value.processed) > Number(value.total)
  ) return null;
  return {
    phase: phase as CodexSessionSyncPhase,
    percent: Number(value.percent),
    processed: Number(value.processed),
    total: Number(value.total),
    target_provider: targetProvider,
  };
}

const SESSION_ID_PATTERN = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const SENSITIVE_TEXT_PATTERN = /(?:https?:\/\/|(?:^|[\s(])(?:~\/|\/(?:Users|home|private|tmp|var|workspace)|[A-Za-z]:[\\/])|sk-|api[_ -]?key|access[_ -]?token|auth\.json|config\.toml|sqlite|journal|wal)/i;

function cleanDisplayText(value: unknown, maxLength: number): string | null {
  if (typeof value !== "string") return null;
  const text = value.replace(/[\u0000-\u001f\u007f]+/g, " ").replace(/\s+/g, " ").trim();
  const lower = text.toLowerCase();
  if (
    !text
    || text.length > maxLength
    || text.includes("/")
    || text.includes("\\")
    || SENSITIVE_TEXT_PATTERN.test(text)
    || lower.includes("bearer ")
    || lower.includes("stack trace")
    || lower.includes("traceback")
    || lower.includes("panic")
  ) return null;
  return text;
}

function isSyntheticSessionTitle(value: string, threadId: string): boolean {
  return value === threadId || /^会话\s+[0-9a-f]{8}$/i.test(value);
}

export function displaySessionTitle(title: unknown, summary: unknown, threadId = ""): string {
  const cleanTitle = cleanDisplayText(title, 120);
  if (cleanTitle && !isSyntheticSessionTitle(cleanTitle, threadId)) return cleanTitle;
  return cleanDisplayText(summary, 96) ?? UNNAMED_SESSION_TITLE;
}

export function displaySessionId(value: string): string {
  return SESSION_ID_PATTERN.test(value) ? value : "无法确认";
}

function normalizeBlocker(value: unknown): CodexSessionBlocker | null {
  if (!isRecord(value)) return null;
  const title = displaySessionTitle(value.title, null, typeof value.thread_id === "string" ? value.thread_id : "");
  const reason = cleanDisplayText(value.reason, 160);
  const nextStep = cleanDisplayText(value.next_step, 160);
  if (!reason || !nextStep || typeof value.thread_id !== "string") return null;
  return {
    title,
    thread_id: displaySessionId(value.thread_id),
    reason,
    next_step: nextStep,
  };
}

export function normalizeCodexSessionPage(value: unknown): CodexSessionPage | null {
  if (!isRecord(value) || !["healthy", "needs_check", "blocked"].includes(String(value.status))) return null;
  if (!Array.isArray(value.items)) return null;
  const items: CodexSessionThread[] = [];
  for (const rawItem of value.items) {
    if (!isRecord(rawItem) || typeof rawItem.thread_id !== "string") continue;
    const blockers = Array.isArray(rawItem.blockers)
      ? rawItem.blockers.map(normalizeBlocker).filter((item): item is CodexSessionBlocker => item !== null)
      : [];
    const provider = cleanDisplayText(rawItem.provider, 80);
    const displayId = displaySessionId(rawItem.thread_id);
    items.push({
      thread_id: displayId,
      title: displaySessionTitle(rawItem.title, rawItem.summary, rawItem.thread_id),
      summary: null,
      updated_at: typeof rawItem.updated_at === "string" ? rawItem.updated_at : null,
      archived: rawItem.archived === true,
      provider: codexProviderLabel(provider),
      can_continue: rawItem.can_continue === true && displayId !== "无法确认",
      needs_migration: rawItem.needs_migration === true && displayId !== "无法确认",
      blockers,
    });
  }
  const blockers = (Array.isArray(value.blockers) ? value.blockers : [])
    .map(normalizeBlocker)
    .filter((item): item is CodexSessionBlocker => item !== null);
  const page = typeof value.page === "number" && Number.isInteger(value.page) ? value.page : 1;
  const pageSize = typeof value.page_size === "number" && Number.isInteger(value.page_size) ? value.page_size : SESSION_PAGE_SIZE;
  const total = typeof value.total === "number" && Number.isInteger(value.total) ? value.total : items.length;
  const totalPages = typeof value.total_pages === "number" && Number.isInteger(value.total_pages) ? value.total_pages : 0;
  if (page < 1 || pageSize < 1 || total < 0 || totalPages < 0) return null;
  return {
    status: value.status as CodexSessionPage["status"],
    items,
    blockers,
    page,
    page_size: pageSize,
    total,
    total_pages: totalPages,
  };
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
  if (status === "healthy") return "本地检查通过，会话可续接";
  if (status === "needs_check") return "有会话待处理";
  if (status === "blocked") return "有会话存在本地结构阻塞";
  return "状态待检查";
}

export function codexProviderLabel(provider?: string | null): string | null {
  if (!provider) return null;
  if (provider === "custom") return "Niko 模型服务";
  if (provider === "openai") return "ChatGPT 官方模型服务";
  if (provider === "Niko 模型服务" || provider === "ChatGPT 官方模型服务" || provider === "已记录的模型服务") {
    return provider;
  }
  return "已记录的模型服务";
}
