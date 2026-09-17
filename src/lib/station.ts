export const MOMOTOKEN_ORIGIN = "https://momotoken.win";

export type StationKind = "momotoken" | "newapi";

export interface StationAuth {
  kind?: StationKind | string;
  origin?: string;
}

export function isNewApiAuth(auth?: StationAuth | null): boolean {
  return auth?.kind === "newapi" && Boolean(auth.origin);
}

export function stationOrigin(auth?: StationAuth | null): string {
  if (isNewApiAuth(auth) && auth?.origin) return auth.origin;
  return MOMOTOKEN_ORIGIN;
}

export function relayBaseUrl(auth?: StationAuth | null): string {
  return `${stationOrigin(auth)}/v1`;
}

const BLOCKED_HOSTS = new Set(["0.0.0.0", "169.254.169.254", "metadata.google.internal"]);

/** 把用户填写的站点地址收成协议+主机+可选子路径，去掉末尾 /v1 或 /api。 */
export function normalizeStationOrigin(input: string): string {
  const trimmed = input.trim();
  if (!trimmed) throw new Error("请填写站点地址");
  let raw = trimmed;
  if (!/^[a-zA-Z][a-zA-Z0-9+.-]*:/.test(raw)) {
    raw = `https://${raw}`;
  }
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new Error("站点地址无效，请检查后再试");
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") {
    throw new Error("站点地址无效，请检查后再试");
  }
  if (url.username || url.password) {
    throw new Error("站点地址无效，请检查后再试");
  }
  const host = url.hostname.toLowerCase();
  if (!host || BLOCKED_HOSTS.has(host)) {
    throw new Error("站点地址无效，请检查后再试");
  }
  let path = url.pathname.replace(/\/+$/, "");
  if (path === "/v1" || path === "/api") path = "";
  return `${url.protocol}//${url.host}${path}`;
}

export function currentDeviceName(): string {
  if (typeof navigator === "undefined") return "Niko";
  const ua = navigator.userAgent;
  if (ua.includes("Mac")) return "macOS";
  if (ua.includes("Win")) return "Windows";
  if (ua.includes("Linux")) return "Linux";
  return "Niko";
}

export function parseOptionalUserId(input: string): number | undefined {
  const trimmed = input.trim();
  if (!trimmed) return undefined;
  if (!/^\d+$/.test(trimmed)) {
    throw new Error("用户 ID 需为正整数，可先留空");
  }
  const value = Number(trimmed);
  if (!Number.isSafeInteger(value) || value <= 0) {
    throw new Error("用户 ID 需为正整数，可先留空");
  }
  return value;
}
