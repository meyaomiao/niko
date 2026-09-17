import { constantTimeEqual, HttpError, sha256Hex } from "./security.js";

export const HEARTBEAT_TTL_SECONDS = 180;
export const HEARTBEAT_RATE_LIMIT_SECONDS = 60;
export const DOWNLOADS_CACHE_TTL_SECONDS = 600;
export const LIVE_PREFIX = "live:";
export const RATE_LIMIT_PREFIX = "rl:";
export const DOWNLOADS_CACHE_KEY = "cache:github-downloads";
export const GITHUB_RELEASES_URL =
  "https://api.github.com/repos/meyaomiao/niko/releases?per_page=100";
export const PRESENCE_SALT = "niko-presence-v1:";
const MAX_HEARTBEAT_BYTES = 2048;
const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[1-8][0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/i;
const VERSION_RE = /^\d{1,3}\.\d{1,3}\.\d{1,3}$/;
const PLATFORM_RE = /^(macos|windows|linux)$/;

function plainObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

export function isInstallerAsset(name) {
  if (typeof name !== "string" || name.includes("/") || name.includes("\\")) {
    return false;
  }
  const lower = name.toLowerCase();
  if (lower.endsWith(".sig") || lower.includes("latest.json") || lower.includes("sha256")) {
    return false;
  }
  if (lower.endsWith(".dmg")) {
    return true;
  }
  if (lower.endsWith(".msi")) {
    return true;
  }
  return lower.endsWith(".exe") && (lower.includes("setup") || lower.includes("installer"));
}

export function installerPlatform(name) {
  if (!isInstallerAsset(name)) {
    return null;
  }
  const lower = name.toLowerCase();
  if (lower.endsWith(".dmg")) {
    return "macos";
  }
  return "windows";
}

export function parseHeartbeatBody(text) {
  if (typeof text !== "string" || text.length === 0 || text.length > MAX_HEARTBEAT_BYTES) {
    throw new HttpError(400, "INVALID_REQUEST", "心跳数据无效。");
  }
  let payload;
  try {
    payload = JSON.parse(text);
  } catch {
    throw new HttpError(400, "INVALID_REQUEST", "心跳数据无效。");
  }
  if (!plainObject(payload)) {
    throw new HttpError(400, "INVALID_REQUEST", "心跳数据无效。");
  }
  const keys = Object.keys(payload);
  if (
    keys.length !== 3 ||
    keys.some((key) => !["install_id", "platform", "app_version"].includes(key))
  ) {
    throw new HttpError(400, "INVALID_REQUEST", "心跳数据无效。");
  }
  const installId = payload.install_id;
  const platform = payload.platform;
  const appVersion = payload.app_version;
  if (
    typeof installId !== "string" ||
    typeof platform !== "string" ||
    typeof appVersion !== "string" ||
    !UUID_RE.test(installId) ||
    !PLATFORM_RE.test(platform) ||
    !VERSION_RE.test(appVersion)
  ) {
    throw new HttpError(400, "INVALID_REQUEST", "心跳数据无效。");
  }
  return {
    install_id: installId.toLowerCase(),
    platform,
    app_version: appVersion,
  };
}

export async function presenceKey(installId) {
  const digest = await sha256Hex(`${PRESENCE_SALT}${installId.toLowerCase()}`);
  return `${LIVE_PREFIX}${digest.slice(0, 32)}`;
}

export function rateLimitKey(installDigest) {
  return `${RATE_LIMIT_PREFIX}${installDigest}`;
}

export function emptyOnline() {
  return {
    total: 0,
    macos: 0,
    windows: 0,
    linux: 0,
    by_version: {},
  };
}

export function emptyDownloads() {
  return {
    installers: 0,
    macos: 0,
    windows: 0,
    by_release: [],
  };
}

export function summarizeOnline(records) {
  const online = emptyOnline();
  for (const record of records) {
    if (!plainObject(record)) {
      continue;
    }
    const platform = PLATFORM_RE.test(record.platform) ? record.platform : null;
    const version = VERSION_RE.test(record.app_version) ? record.app_version : null;
    if (!platform || !version) {
      continue;
    }
    online.total += 1;
    online[platform] += 1;
    online.by_version[version] = (online.by_version[version] || 0) + 1;
  }
  return online;
}

export function summarizeDownloads(releases) {
  const downloads = emptyDownloads();
  if (!Array.isArray(releases)) {
    return downloads;
  }
  for (const release of releases) {
    if (!plainObject(release) || !Array.isArray(release.assets)) {
      continue;
    }
    const tag =
      typeof release.tag_name === "string" && release.tag_name.length > 0
        ? release.tag_name
        : typeof release.name === "string" && release.name.length > 0
          ? release.name
          : "";
    if (!tag) {
      continue;
    }
    const row = { tag, installers: 0, macos: 0, windows: 0 };
    for (const asset of release.assets) {
      if (!plainObject(asset) || typeof asset.name !== "string") {
        continue;
      }
      const platform = installerPlatform(asset.name);
      if (!platform) {
        continue;
      }
      const count = Number(asset.download_count);
      const value = Number.isFinite(count) && count > 0 ? Math.floor(count) : 0;
      row.installers += value;
      row[platform] += value;
      downloads.installers += value;
      downloads[platform] += value;
    }
    if (row.installers > 0) {
      downloads.by_release.push(row);
    }
  }
  return downloads;
}

export function opsSecretFromRequest(request) {
  const header = request.headers.get("Authorization") || "";
  const bearer = /^Bearer\s+(\S+)$/i.exec(header);
  if (bearer) {
    return bearer[1];
  }
  return request.headers.get("X-Niko-Ops-Token") || "";
}

export function authorizeOps(request, env) {
  const configured = typeof env?.NIKO_OPS_SECRET === "string" ? env.NIKO_OPS_SECRET : "";
  if (configured.length < 16) {
    throw new HttpError(503, "OPS_NOT_CONFIGURED", "使用情况尚未完成配置。");
  }
  const provided = opsSecretFromRequest(request);
  if (!constantTimeEqual(configured, provided)) {
    throw new HttpError(401, "OPS_UNAUTHORIZED", "访问口令不正确。");
  }
}

async function listAllKeys(kv, prefix) {
  const keys = [];
  let cursor = "";
  do {
    const page = await kv.list({ prefix, cursor: cursor || undefined, limit: 1000 });
    for (const item of page.keys || []) {
      if (item?.name) {
        keys.push(item.name);
      }
    }
    cursor = page.list_complete ? "" : page.cursor || "";
  } while (cursor);
  return keys;
}

export async function readOnlineRecords(kv) {
  if (!kv) {
    throw new HttpError(503, "PRESENCE_NOT_CONFIGURED", "使用情况尚未完成配置。");
  }
  const names = await listAllKeys(kv, LIVE_PREFIX);
  const records = [];
  for (const name of names) {
    const raw = await kv.get(name);
    if (!raw) {
      continue;
    }
    try {
      records.push(JSON.parse(raw));
    } catch {
      continue;
    }
  }
  return records;
}

export async function recordHeartbeat(kv, bodyText) {
  if (!kv) {
    throw new HttpError(503, "PRESENCE_NOT_CONFIGURED", "使用情况尚未完成配置。");
  }
  const heartbeat = parseHeartbeatBody(bodyText);
  const key = await presenceKey(heartbeat.install_id);
  const limitKey = rateLimitKey(key.slice(LIVE_PREFIX.length));
  const limited = await kv.get(limitKey);
  if (limited) {
    throw new HttpError(429, "RATE_LIMITED", "操作过于频繁，请稍后重试。");
  }
  await kv.put(limitKey, "1", { expirationTtl: HEARTBEAT_RATE_LIMIT_SECONDS });
  await kv.put(
    key,
    JSON.stringify({
      platform: heartbeat.platform,
      app_version: heartbeat.app_version,
      seen_at: Date.now(),
    }),
    { expirationTtl: HEARTBEAT_TTL_SECONDS },
  );
  return { ok: true };
}

async function fetchGithubDownloads() {
  const response = await fetch(GITHUB_RELEASES_URL, {
    headers: {
      Accept: "application/vnd.github+json",
      "User-Agent": "Niko-Website-Presence/1.0",
    },
    signal: AbortSignal.timeout(8000),
  });
  if (!response.ok) {
    throw new Error(`github ${response.status}`);
  }
  return summarizeDownloads(await response.json());
}

export async function readDownloads(kv, now = Date.now()) {
  const fallback = emptyDownloads();
  if (!kv) {
    return fallback;
  }
  const cachedRaw = await kv.get(DOWNLOADS_CACHE_KEY);
  if (cachedRaw) {
    try {
      const cached = JSON.parse(cachedRaw);
      if (plainObject(cached) && Number(cached.cached_at) > 0) {
        const age = now - Number(cached.cached_at);
        if (age >= 0 && age < DOWNLOADS_CACHE_TTL_SECONDS * 1000 && plainObject(cached.downloads)) {
          return {
            ...emptyDownloads(),
            ...cached.downloads,
            by_release: Array.isArray(cached.downloads.by_release)
              ? cached.downloads.by_release
              : [],
          };
        }
      }
    } catch {
      /* fall through to refresh */
    }
  }
  try {
    const downloads = await fetchGithubDownloads();
    await kv.put(
      DOWNLOADS_CACHE_KEY,
      JSON.stringify({ cached_at: now, downloads }),
      { expirationTtl: DOWNLOADS_CACHE_TTL_SECONDS },
    );
    return downloads;
  } catch {
    if (cachedRaw) {
      try {
        const cached = JSON.parse(cachedRaw);
        if (plainObject(cached?.downloads)) {
          return {
            ...emptyDownloads(),
            ...cached.downloads,
            by_release: Array.isArray(cached.downloads.by_release)
              ? cached.downloads.by_release
              : [],
          };
        }
      } catch {
        /* ignore */
      }
    }
    return fallback;
  }
}

export async function buildStats(kv, now = Date.now()) {
  const [records, downloads] = await Promise.all([readOnlineRecords(kv), readDownloads(kv, now)]);
  return {
    online: summarizeOnline(records),
    downloads,
    generated_at: new Date(now).toISOString(),
    window_seconds: HEARTBEAT_TTL_SECONDS,
  };
}
