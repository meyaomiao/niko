import { HttpError } from "./security.js";

export const GITHUB_LATEST_JSON =
  "https://github.com/meyaomiao/niko/releases/latest/download/latest.json";

export const UNIVERSAL_MAC_PLATFORM = "darwin-universal";

function plainObject(value) {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function platformEntry(value) {
  if (!plainObject(value)) return null;
  const url = typeof value.url === "string" ? value.url.trim() : "";
  const signature = typeof value.signature === "string" ? value.signature.trim() : "";
  if (!url || !signature) return null;
  return { url, signature };
}

export function normalizeUpdaterPlatforms(platforms) {
  const next = plainObject(platforms) ? { ...platforms } : {};
  const universal = platformEntry(next[UNIVERSAL_MAC_PLATFORM]);
  if (universal) {
    if (!platformEntry(next["darwin-aarch64"])) next["darwin-aarch64"] = universal;
    if (!platformEntry(next["darwin-x86_64"])) next["darwin-x86_64"] = universal;
  }
  delete next[UNIVERSAL_MAC_PLATFORM];
  return next;
}

export function rewriteUpdaterManifest(manifest) {
  if (!plainObject(manifest) || typeof manifest.version !== "string" || !/^\d+\.\d+\.\d+/.test(manifest.version.trim())) {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }
  if (!plainObject(manifest.platforms)) {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }

  const platforms = {};
  for (const [key, value] of Object.entries(manifest.platforms)) {
    const entry = platformEntry(value);
    if (entry) platforms[key] = entry;
  }

  const rewritten = normalizeUpdaterPlatforms(platforms);
  if (Object.keys(rewritten).length === 0) {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }

  return {
    version: manifest.version.trim(),
    notes: typeof manifest.notes === "string" ? manifest.notes : undefined,
    pub_date: typeof manifest.pub_date === "string" ? manifest.pub_date : undefined,
    platforms: rewritten,
  };
}

export async function fetchLatestUpdaterManifest(fetcher = fetch) {
  let response;
  try {
    response = await fetcher(GITHUB_LATEST_JSON, {
      headers: {
        Accept: "application/json",
        "User-Agent": "niko-updater-proxy",
      },
      redirect: "follow",
    });
  } catch {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }
  if (!response.ok) {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }
  let payload;
  try {
    payload = await response.json();
  } catch {
    throw new HttpError(502, "UPDATER_UNAVAILABLE", "暂时无法读取更新信息。");
  }
  return rewriteUpdaterManifest(payload);
}

export function updaterJsonResponse(manifest) {
  return new Response(JSON.stringify(manifest), {
    status: 200,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "public, max-age=60, must-revalidate",
      "X-Content-Type-Options": "nosniff",
    },
  });
}
