import { errorResponse, HttpError } from "./_lib/security.js";
import { fetchLatestUpdaterManifest, updaterJsonResponse } from "./_lib/updater.js";

export async function onRequest({ request }) {
  try {
    if (request.method.toUpperCase() !== "GET") {
      throw new HttpError(405, "METHOD_NOT_ALLOWED", "请求方法不受支持。");
    }
    const manifest = await fetchLatestUpdaterManifest();
    return updaterJsonResponse(manifest);
  } catch (error) {
    return errorResponse(error);
  }
}
