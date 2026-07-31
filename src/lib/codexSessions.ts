export interface CodexSessionDiagnostic {
  level: string;
  code: string;
  message: string;
  path?: string | null;
  thread_id?: string | null;
}

export interface CodexSessionThread {
  thread_id: string;
  providers: string[];
  workspaces: string[];
  archived?: boolean | null;
  rollout_count: number;
}

export interface CodexSessionInventory {
  codex_home: string;
  active_provider?: string | null;
  defined_providers: string[];
  provider_layout: string;
  layout_hint: string;
  normalization_status: string;
  normalization_target_provider: string;
  session_index_entries?: number | null;
  thread_count: number;
  archived_thread_count: number;
  diagnostics: CodexSessionDiagnostic[];
  threads: CodexSessionThread[];
}

export interface CodexSessionMutationOutcome {
  ok: boolean;
  target_provider: string;
  changed_artifacts: number;
  restart_allowed: boolean;
  retryable: boolean;
  message: string;
}

export function codexSessionSearchText(thread: CodexSessionThread): string {
  return [
    thread.thread_id,
    thread.providers.join(" "),
    thread.workspaces.join(" "),
    thread.archived ? "archived" : "active",
  ]
    .join(" ")
    .toLowerCase();
}

export function codexProviderLabel(provider: string): string {
  switch (provider.toLowerCase()) {
    case "openai":
      return "官方";
    case "custom":
      return "Niko";
    case "momotoken":
      return "旧版 Niko";
    default:
      return "兼容来源";
  }
}

export function filterCodexSessionThreads(
  threads: CodexSessionThread[],
  query: string,
): CodexSessionThread[] {
  const normalized = query.trim().toLowerCase();
  if (!normalized) return threads;
  return threads.filter((thread) => codexSessionSearchText(thread).includes(normalized));
}

export function codexNormalizationLabel(status: string): string {
  switch (status) {
    case "no_changes":
      return "当前状态正常";
    case "needs_check":
      return "发现需要整理的会话";
    case "blocked":
      return "有部分会话暂时无法处理";
    default:
      return "状态待检查";
  }
}
