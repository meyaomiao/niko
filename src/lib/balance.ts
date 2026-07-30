export interface BalanceSnapshot {
  quota: number;
  quotaPerUnit: number;
  updatedAt: number;
}

export interface BalanceState {
  snapshot: BalanceSnapshot | null;
  refreshing: boolean;
  error: string;
}

export type BalanceAction =
  | { type: "refresh-started" }
  | { type: "refresh-succeeded"; snapshot: BalanceSnapshot }
  | { type: "refresh-failed"; error: string };

function numeric(value: unknown): number | null {
  if (typeof value === "number") {
    return Number.isFinite(value) ? value : null;
  }
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  if (!/^-?\d+(?:\.\d+)?$/.test(trimmed)) return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? parsed : null;
}

export function parseBalanceSnapshot(
  quotaValue: unknown,
  quotaPerUnitValue: unknown,
  updatedAt = Date.now(),
): BalanceSnapshot | null {
  const quota = numeric(quotaValue);
  const quotaPerUnit = numeric(quotaPerUnitValue);
  if (
    quota === null ||
    !Number.isSafeInteger(quota) ||
    quotaPerUnit === null ||
    quotaPerUnit <= 0 ||
    !Number.isFinite(updatedAt) ||
    updatedAt <= 0
  ) {
    return null;
  }
  return { quota, quotaPerUnit, updatedAt };
}

function roundHalfAwayFromZero(value: number): number {
  return value < 0 ? Math.ceil(value - 0.5) : Math.floor(value + 0.5);
}

export function formatBalanceUSD(snapshot: BalanceSnapshot | null): string {
  if (!snapshot) return "—";
  const centsValue = (snapshot.quota * 100) / snapshot.quotaPerUnit;
  if (!Number.isFinite(centsValue) || Math.abs(centsValue) > Number.MAX_SAFE_INTEGER) {
    return "—";
  }
  const cents = roundHalfAwayFromZero(centsValue);
  const absolute = Math.abs(cents);
  const amount = `${Math.floor(absolute / 100)}.${String(absolute % 100).padStart(2, "0")}`;
  return cents < 0 ? `-$${amount}` : `$${amount}`;
}

export function formatBalanceUpdatedAt(
  snapshot: BalanceSnapshot | null,
): string {
  if (!snapshot) return "";
  return `${new Date(snapshot.updatedAt).toLocaleTimeString("zh-CN", {
    hour: "2-digit",
    minute: "2-digit",
  })} 更新`;
}

export function balanceReducer(state: BalanceState, action: BalanceAction): BalanceState {
  switch (action.type) {
    case "refresh-started":
      return { ...state, refreshing: true, error: "" };
    case "refresh-succeeded":
      return { snapshot: action.snapshot, refreshing: false, error: "" };
    case "refresh-failed":
      return { ...state, refreshing: false, error: action.error };
  }
}
