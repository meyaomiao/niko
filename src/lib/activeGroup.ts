import { vendorOfGroup } from "./vendor.ts";

export type ActiveGroupState =
  | "active"
  | "not_niko"
  | "changed"
  | "unknown"
  | "unreadable";

export interface ActiveGroupStatus {
  version: number;
  target_id: string;
  status: ActiveGroupState;
  group?: string;
}

export interface EffectiveSelectionStatus extends ActiveGroupStatus {
  model?: string;
}

export const ACTIVE_GROUP_STATUS_VERSION = 1;

export type ActiveGroupView =
  | { kind: "loading"; text: string }
  | { kind: "active"; text: string; group: string }
  | { kind: "different"; text: string }
  | { kind: "not_niko"; text: string }
  | { kind: "changed"; text: string }
  | { kind: "unknown"; text: string };

export type EffectiveSelectionView =
  | { kind: "loading"; text: string }
  | { kind: "active"; text: string; provider: string; group: string; model: string }
  | { kind: "different"; text: string }
  | { kind: "not_niko"; text: string }
  | { kind: "changed"; text: string }
  | { kind: "unknown"; text: string };

const ACTIVE_TEXT = "当前正在使用的模型服务";
const CHANGED_TEXT = "这个应用的设置后来被改过，请重新接入到应用后再试。";
const UNKNOWN_TEXT = "暂时无法确认当前使用的模型服务，请重新检查。";
const ACTIVE_STATES = new Set<ActiveGroupState>([
  "active",
  "not_niko",
  "changed",
  "unknown",
  "unreadable",
]);

function isSafeSelectionText(value: unknown): value is string {
  return typeof value === "string"
    && value.trim().length > 0
    && Array.from(value).length <= 128
    && !/[\u0000-\u001f\u007f]/.test(value)
    && !/^(?:\/|\\\\|[A-Za-z]:[\\/])/.test(value)
    && !value.includes("://")
    && !/(?:sk-|api[_ -]?key|access[_ -]?token|bearer\s|auth\.json|config\.toml|sqlite|journal|wal)/i.test(value);
}

export function normalizeActiveGroupStatuses(value: unknown): Record<string, ActiveGroupStatus> {
  if (!Array.isArray(value)) return {};
  const result: Record<string, ActiveGroupStatus> = {};
  for (const item of value) {
    if (typeof item !== "object" || item === null || Array.isArray(item)) continue;
    const candidate = item as Record<string, unknown>;
    if (typeof candidate.target_id !== "string") continue;
    const fallback: ActiveGroupStatus = {
      version: ACTIVE_GROUP_STATUS_VERSION,
      target_id: candidate.target_id,
      status: "unknown",
    };
    if (
      candidate.version !== ACTIVE_GROUP_STATUS_VERSION
      || typeof candidate.status !== "string"
      || !ACTIVE_STATES.has(candidate.status as ActiveGroupState)
      || (candidate.group !== undefined && !isSafeSelectionText(candidate.group))
      || (candidate.status === "active" && !isSafeSelectionText(candidate.group))
    ) {
      result[candidate.target_id] = fallback;
      continue;
    }
    result[candidate.target_id] = {
      version: ACTIVE_GROUP_STATUS_VERSION,
      target_id: candidate.target_id,
      status: candidate.status as ActiveGroupState,
      ...(candidate.group ? { group: candidate.group } : {}),
    };
  }
  return result;
}

export function normalizeEffectiveSelectionStatuses(value: unknown): Record<string, EffectiveSelectionStatus> {
  if (!Array.isArray(value)) return {};
  const result: Record<string, EffectiveSelectionStatus> = {};
  for (const item of value) {
    if (typeof item !== "object" || item === null || Array.isArray(item)) continue;
    const candidate = item as Record<string, unknown>;
    const fallback: EffectiveSelectionStatus = {
      version: ACTIVE_GROUP_STATUS_VERSION,
      target_id: typeof candidate.target_id === "string" ? candidate.target_id : "unknown",
      status: "unknown",
    };
    if (
      candidate.version !== ACTIVE_GROUP_STATUS_VERSION
      || typeof candidate.target_id !== "string"
      || typeof candidate.status !== "string"
      || !ACTIVE_STATES.has(candidate.status as ActiveGroupState)
      || (candidate.group !== undefined && !isSafeSelectionText(candidate.group))
      || (candidate.model !== undefined && !isSafeSelectionText(candidate.model))
      || (candidate.status === "active" && (!isSafeSelectionText(candidate.group) || !isSafeSelectionText(candidate.model)))
    ) {
      result[fallback.target_id] = fallback;
      continue;
    }
    result[candidate.target_id] = {
      version: ACTIVE_GROUP_STATUS_VERSION,
      target_id: candidate.target_id,
      status: candidate.status as ActiveGroupState,
      ...(candidate.group ? { group: candidate.group } : {}),
      ...(candidate.model ? { model: candidate.model } : {}),
    };
  }
  return result;
}

export function commonActiveGroup(
  statuses: Record<string, ActiveGroupStatus>,
  targetIds: string[],
): string | null {
  if (targetIds.length === 0) return null;
  const groups = targetIds.map((id) => {
    const status = statuses[id];
    return status?.status === "active" && status.group ? status.group : null;
  });
  if (groups.some((group) => group === null)) return null;
  const first = groups[0] as string;
  return groups.every((group) => group === first) ? first : null;
}

export interface ActiveSelection {
  provider: string;
  group: string;
  model: string;
}

export function commonActiveSelection(
  statuses: Record<string, EffectiveSelectionStatus>,
  targetIds: string[],
): ActiveSelection | null {
  if (targetIds.length === 0) return null;
  const selections = targetIds.map((id) => {
    const status = statuses[id];
    return status?.status === "active" && status.group && status.model
      ? { group: status.group, model: status.model }
      : null;
  });
  if (selections.some((selection) => selection === null)) return null;
  const firstSelection = selections[0] as { group: string; model: string };
  const first: ActiveSelection = {
    provider: vendorOfGroup(firstSelection.group),
    group: firstSelection.group,
    model: firstSelection.model,
  };
  return selections.every(
    (selection) => selection?.group === first.group && selection.model === first.model,
  )
    ? first
    : null;
}

export function summarizeEffectiveSelections(
  statuses: Record<string, EffectiveSelectionStatus>,
  targetIds: string[],
  loading = false,
): EffectiveSelectionView {
  if (loading) return { kind: "loading", text: "正在确认当前设置…" };
  if (targetIds.length === 0) return { kind: "unknown", text: UNKNOWN_TEXT };

  const selected = targetIds.map((id) => statuses[id]);
  const common = commonActiveSelection(statuses, targetIds);
  if (common) {
    return {
      kind: "active",
      text: `${ACTIVE_TEXT}：${common.provider} · ${common.model} · ${common.group}`,
      provider: common.provider,
      group: common.group,
      model: common.model,
    };
  }
  if (selected.every((status) => status?.status === "active")) {
    return { kind: "different", text: "不同应用正在使用不同的模型服务，请分别检查。" };
  }
  if (selected.some((status) => status?.status === "changed")) {
    return { kind: "changed", text: CHANGED_TEXT };
  }
  if (selected.every((status) => status?.status === "not_niko")) {
    return { kind: "not_niko", text: "当前应用还没有接入 Niko，可选择模型服务后接入。" };
  }
  return { kind: "unknown", text: UNKNOWN_TEXT };
}

export function summarizeActiveGroups(
  statuses: Record<string, ActiveGroupStatus>,
  targetIds: string[],
  loading = false,
): ActiveGroupView {
  if (loading) return { kind: "loading", text: "正在确认当前设置…" };
  if (targetIds.length === 0) return { kind: "unknown", text: UNKNOWN_TEXT };

  const selected = targetIds.map((id) => statuses[id]);
  const common = commonActiveGroup(statuses, targetIds);
  if (common) return { kind: "active", text: `${ACTIVE_TEXT}：${common}`, group: common };

  if (
    selected.every((status) => status?.status === "active")
    && selected.some((status) => status?.status === "active")
  ) {
    return { kind: "different", text: "不同应用正在使用不同的模型服务，请分别检查。" };
  }
  if (selected.some((status) => status?.status === "changed")) {
    return { kind: "changed", text: CHANGED_TEXT };
  }
  if (selected.every((status) => status?.status === "not_niko")) {
    return { kind: "not_niko", text: "当前应用还没有接入 Niko，可选择模型服务后接入。" };
  }
  return { kind: "unknown", text: UNKNOWN_TEXT };
}
