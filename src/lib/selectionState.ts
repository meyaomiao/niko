export interface DraftSelection {
  provider: string;
  model: string;
  group: string;
}

const STORAGE_KEY = "niko_draft_selections";
const MAX_TEXT_LENGTH = 128;

function isSafeText(value: unknown): value is string {
  return typeof value === "string"
    && value.trim().length > 0
    && Array.from(value).length <= MAX_TEXT_LENGTH
    && !/[\u0000-\u001f\u007f]/.test(value)
    && !value.includes("/")
    && !value.includes("\\")
    && !value.includes("://")
    && !/(?:sk-|api[_ -]?key|access[_ -]?token)/i.test(value);
}

function isDraftSelection(value: unknown): value is DraftSelection {
  if (typeof value !== "object" || value === null || Array.isArray(value)) return false;
  const candidate = value as Record<string, unknown>;
  return Object.keys(candidate).every((key) => ["provider", "model", "group"].includes(key))
    && isSafeText(candidate.provider)
    && isSafeText(candidate.model)
    && isSafeText(candidate.group);
}

export function loadDraftSelection(targetId: string): DraftSelection | null {
  if (!targetId) return null;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return null;
    const selection = (parsed as Record<string, unknown>)[targetId];
    return isDraftSelection(selection) ? selection : null;
  } catch {
    return null;
  }
}

export function saveDraftSelection(targetId: string, selection: DraftSelection): void {
  if (!targetId || !isDraftSelection(selection)) return;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    const current = typeof parsed === "object" && parsed !== null && !Array.isArray(parsed)
      ? parsed as Record<string, unknown>
      : {};
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ ...current, [targetId]: selection }));
  } catch {
    // Selection persistence is helpful but must never block configuration.
  }
}
