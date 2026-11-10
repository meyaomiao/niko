export const RESTORE_CONFIRMATION_MS = 5_000;

export interface RestoreConfirmation {
  targetIds: string[];
  expiresAt: number;
}

export function restoreConfirmationMatches(
  confirmation: RestoreConfirmation | null,
  targetIds: string[],
  now = Date.now(),
): boolean {
  return Boolean(
    confirmation
    && now < confirmation.expiresAt
    && targetIds.length > 0
    && confirmation.targetIds.length === targetIds.length
    && targetIds.every((id) => confirmation.targetIds.includes(id)),
  );
}

/** 原生写入不能通过前端计时器取消；超时提示后仍等待真实结果，避免重复写入。 */
export async function waitForDesktopAction<T>(
  operation: Promise<T>,
  onSlow: () => void,
  delayMs = 30_000,
): Promise<T> {
  const timer = setTimeout(onSlow, delayMs);
  try {
    return await operation;
  } finally {
    clearTimeout(timer);
  }
}

export type TargetActionResult<T> =
  | { id: string; ok: true; value: T }
  | { id: string; ok: false; error: unknown };

/** 一个应用失败时仍处理其余应用，并保留每个应用的实际结果。 */
export async function runTargetActions<T>(
  targetIds: string[],
  action: (id: string) => Promise<T>,
): Promise<TargetActionResult<T>[]> {
  const results: TargetActionResult<T>[] = [];
  for (const id of targetIds) {
    try {
      results.push({ id, ok: true, value: await action(id) });
    } catch (error) {
      results.push({ id, ok: false, error });
    }
  }
  return results;
}
