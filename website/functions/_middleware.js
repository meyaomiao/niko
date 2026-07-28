import { contentSecurityPolicy } from "./_lib/security.js";

export async function onRequest({ env, next }) {
  const response = await next();
  const headers = new Headers(response.headers);
  headers.set("Content-Security-Policy", contentSecurityPolicy(env));
  return new Response(response.body, {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}
