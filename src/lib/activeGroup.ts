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

export const ACTIVE_GROUP_STATUS_VERSION = 1;

export type ActiveGroupView =
  | { kind: "loading"; text: string }
  | { kind: "active"; text: string; group: string }
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
      || (candidate.group !== undefined && typeof candidate.group !== "string")
      || (candidate.group !== undefined && Array.from(candidate.group as string).length > 128)
      || (candidate.status === "active" && typeof candidate.group !== "string")
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
