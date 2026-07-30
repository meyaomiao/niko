import assert from "node:assert/strict";
import test from "node:test";

import {
  filteredSearch,
  idempotencyKey,
  matchRoute,
  parseBody,
} from "../functions/_lib/routes.js";
import {
  CSRF_COOKIE,
  HttpError,
  contentSecurityPolicy,
  createUpstreamRequest,
  csrfCookie,
  removeAuthTokens,
  sessionCookie,
  validateMutationRequest,
  validatePaymentResponse,
} from "../functions/_lib/security.js";

function assertHttpError(error, status, code) {
  assert.ok(error instanceof HttpError);
  assert.equal(error.status, status);
  assert.equal(error.code, code);
  return true;
}

function validPaymentParams() {
  return {
    pid: "10001",
    type: "alipay",
    out_trade_no: "NKO0123456789abcdef0123456789abcdef",
    notify_url: "https://momotoken.win/api/user/epay/notify",
    name: "Niko 余额充值",
    money: "10.00",
    device: "pc",
    sign_type: "MD5",
    return_url: "https://niko-ai.cc/payment/return/?order_id=NKO123",
    sign: "0123456789abcdef0123456789abcdef",
  };
}

test("route matching only permits the explicit BFF surface", () => {
  assert.deepEqual(matchRoute("GET", "wallet/summary"), {
    id: "walletSummary",
    protected: true,
    path: "wallet/summary",
  });
  assert.equal(
    matchRoute("GET", "wallet/topup-orders/order_123").orderId,
    "order_123",
  );

  assert.throws(
    () => matchRoute("DELETE", "wallet/summary"),
    (error) => assertHttpError(error, 404, "ROUTE_NOT_FOUND"),
  );
  assert.throws(
    () => matchRoute("GET", "wallet%2Fsummary"),
    (error) => assertHttpError(error, 404, "ROUTE_NOT_FOUND"),
  );
  assert.throws(
    () => matchRoute("GET", "../wallet/summary"),
    (error) => assertHttpError(error, 404, "ROUTE_NOT_FOUND"),
  );
});

test("query filtering rejects fields outside the route contract", () => {
  const route = matchRoute("GET", "wallet/topups");
  assert.equal(
    filteredSearch(new URL("https://niko-ai.cc/api/niko/v1/wallet/topups?limit=20&cursor=next"), route),
    "?limit=20&cursor=next",
  );
  assert.throws(
    () => filteredSearch(new URL("https://niko-ai.cc/api/niko/v1/wallet/topups?admin=true"), route),
    (error) => assertHttpError(error, 400, "INVALID_QUERY"),
  );
});

test("mutation validation requires same-origin and a matching CSRF token", () => {
  const token = "a".repeat(64);
  const cookies = new Map([[CSRF_COOKIE, token]]);
  const valid = new Request("https://niko-ai.cc/api/niko/v1/auth/login", {
    method: "POST",
    headers: {
      Origin: "https://niko-ai.cc",
      "X-CSRF-Token": token,
    },
  });
  assert.doesNotThrow(() => validateMutationRequest(valid, cookies));

  const crossOrigin = new Request(valid.url, {
    method: "POST",
    headers: {
      Origin: "https://example.com",
      "X-CSRF-Token": token,
    },
  });
  assert.throws(
    () => validateMutationRequest(crossOrigin, cookies),
    (error) => assertHttpError(error, 403, "ORIGIN_REJECTED"),
  );

  const wrongToken = new Request(valid.url, {
    method: "POST",
    headers: {
      Origin: "https://niko-ai.cc",
      "X-CSRF-Token": "b".repeat(64),
    },
  });
  assert.throws(
    () => validateMutationRequest(wrongToken, cookies),
    (error) => assertHttpError(error, 403, "CSRF_REJECTED"),
  );
});

test("session cookies keep authentication server-only", () => {
  const session = sessionCookie("session-token", 600);
  assert.match(session, /^__Host-niko_session=/);
  assert.match(session, /Path=\//);
  assert.match(session, /Max-Age=600/);
  assert.match(session, /Secure/);
  assert.match(session, /HttpOnly/);
  assert.match(session, /SameSite=Lax/);
  assert.doesNotMatch(session, /Domain=/);

  const csrf = csrfCookie("csrf-token", 600);
  assert.match(csrf, /^__Host-niko_csrf=/);
  assert.match(csrf, /Secure/);
  assert.doesNotMatch(csrf, /HttpOnly/);
});

test("top-up requests require one amount mode and a valid idempotency key", async () => {
  const route = matchRoute("POST", "wallet/topup-orders");
  const request = new Request("https://niko-ai.cc/api/niko/v1/wallet/topup-orders", {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Idempotency-Key": "order_request_1234",
    },
    body: JSON.stringify({ option_id: "starter", payment_channel: "alipay" }),
  });

  assert.deepEqual(
    await parseBody(request, route),
    {
      option_id: "starter",
      payment_channel: "alipay",
    },
  );
  assert.equal(idempotencyKey(request, route), "order_request_1234");

  const mixedMode = new Request(request.url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      option_id: "starter",
      amount: "10.00",
      currency: "USD",
      payment_channel: "alipay",
    }),
  });
  await assert.rejects(
    () => parseBody(mixedMode, route),
    (error) => assertHttpError(error, 400, "INVALID_REQUEST"),
  );

  const forbiddenField = new Request(request.url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      option_id: "starter",
      payment_channel: "alipay",
      quota_to_add: "999999",
    }),
  });
  await assert.rejects(
    () => parseBody(forbiddenField, route),
    (error) => assertHttpError(error, 400, "INVALID_REQUEST"),
  );

  assert.throws(
    () => idempotencyKey(new Request(request.url), route),
    (error) => assertHttpError(error, 400, "IDEMPOTENCY_KEY_REQUIRED"),
  );
});

test("payment responses require the exact Epay POST contract on an allowed HTTPS origin", () => {
  const env = {
    NIKO_PAYMENT_ALLOWED_ORIGINS: "https://pay.example.com",
    NIKO_PAYMENT_REDIRECT_ORIGINS: "https://checkout.example.com",
  };
  assert.doesNotThrow(() =>
    validatePaymentResponse(
      {
        data: {
          payment_url: "https://pay.example.com/checkout/123",
          payment_params: validPaymentParams(),
        },
      },
      env,
    ),
  );
  assert.throws(
    () =>
      validatePaymentResponse(
        {
          payment_url: "http://pay.example.com/checkout",
          payment_params: validPaymentParams(),
        },
        env,
      ),
    (error) => assertHttpError(error, 502, "PAYMENT_URL_REJECTED"),
  );
  assert.throws(
    () =>
      validatePaymentResponse(
        {
          payment_url: "https://checkout.example.com/redirect-only",
          payment_params: validPaymentParams(),
        },
        env,
      ),
    (error) => assertHttpError(error, 502, "PAYMENT_URL_REJECTED"),
  );
  assert.throws(
    () =>
      validatePaymentResponse(
        {
          payment_url: "https://pay.example.com.evil.test/",
          payment_params: validPaymentParams(),
        },
        env,
      ),
    (error) => assertHttpError(error, 502, "PAYMENT_URL_REJECTED"),
  );
});

test("payment parameter validation rejects pollution, nesting, wrong types, and abnormal size", () => {
  const env = { NIKO_PAYMENT_ALLOWED_ORIGINS: "https://pay.example.com" };
  const response = (paymentParams) => ({
    payment_url: "https://pay.example.com/submit.php",
    payment_params: paymentParams,
  });
  const invalidParams = [
    null,
    [],
    { ...validPaymentParams(), name: { nested: true } },
    { ...validPaymentParams(), money: 10 },
    { ...validPaymentParams(), pid: "p".repeat(129) },
    { ...validPaymentParams(), extra: "unexpected" },
    (() => {
      const params = validPaymentParams();
      delete params.sign;
      return params;
    })(),
    Object.assign(Object.create(null), validPaymentParams()),
    JSON.parse(`{"__proto__":"polluted",${JSON.stringify(validPaymentParams()).slice(1)}`),
    JSON.parse(`{"constructor":"polluted",${JSON.stringify(validPaymentParams()).slice(1)}`),
    JSON.parse(`{"prototype":"polluted",${JSON.stringify(validPaymentParams()).slice(1)}`),
  ];

  for (const paymentParams of invalidParams) {
    assert.throws(
      () => validatePaymentResponse(response(paymentParams), env),
      (error) => assertHttpError(error, 502, "INVALID_PAYMENT_RESPONSE"),
    );
  }
});

test("payment CSP uses only configured origins for form submissions", () => {
  const policy = contentSecurityPolicy({
    NIKO_PAYMENT_ALLOWED_ORIGINS:
      "https://pay.example.com, https://gateway.example.net:8443, https://ignored.test/path, http://insecure.test",
    NIKO_PAYMENT_REDIRECT_ORIGINS:
      "https://checkout.example.com, https://ignored-redirect.test/path, http://insecure-redirect.test",
  });
  const formAction = policy.split("; ").find((directive) => directive.startsWith("form-action"));
  assert.equal(
    formAction,
    "form-action 'self' https://pay.example.com https://gateway.example.net:8443 https://checkout.example.com",
  );
  assert.doesNotMatch(formAction, /(?:^|\s)https:(?:\s|$)/);
});

test("upstream requests carry a verifiable HMAC and never expose auth tokens", async () => {
  const source = new Request("https://niko-ai.cc/api/niko/v1/wallet/summary", {
    headers: { "CF-Connecting-IP": "203.0.113.9" },
  });
  const upstream = await createUpstreamRequest({
    request: source,
    env: {
      MOMOTOKEN_NIKO_API_ORIGIN: "https://momotoken.win",
      NIKO_BFF_SECRET: "test-secret-that-is-at-least-32-bytes",
    },
    pathname: "/api/niko/v1/wallet/summary",
    search: "?limit=1",
    method: "GET",
    body: undefined,
    sessionToken: "s".repeat(32),
    csrfToken: "c".repeat(64),
    idempotencyKey: "",
  });

  assert.equal(upstream.url, "https://momotoken.win/api/niko/v1/wallet/summary?limit=1");
  assert.equal(upstream.headers.get("X-Niko-Session"), "s".repeat(32));
  assert.equal(upstream.headers.get("X-Niko-CSRF"), "c".repeat(64));
  const timestamp = upstream.headers.get("X-Niko-Timestamp") || "";
  const nonce = upstream.headers.get("X-Niko-Nonce") || "";
  const bodyHash = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
  const canonical = [
    "GET",
    "/api/niko/v1/wallet/summary?limit=1",
    bodyHash,
    timestamp,
    nonce,
    "203.0.113.9",
    "s".repeat(32),
    "c".repeat(64),
    "",
  ].join("\n");
  const encoder = new TextEncoder();
  const key = await crypto.subtle.importKey(
    "raw",
    encoder.encode("test-secret-that-is-at-least-32-bytes"),
    { name: "HMAC", hash: "SHA-256" },
    false,
    ["sign"],
  );
  const expectedBytes = new Uint8Array(
    await crypto.subtle.sign("HMAC", key, encoder.encode(canonical)),
  );
  const expectedSignature = [...expectedBytes]
    .map((value) => value.toString(16).padStart(2, "0"))
    .join("");
  assert.equal(upstream.headers.get("X-Niko-Signature"), expectedSignature);
  assert.match(nonce, /^[a-f0-9]{32}$/);

  const payload = {
    data: {
      username: "niko-user",
      session_token: "s".repeat(32),
      csrf_token: "c".repeat(64),
    },
  };
  const extracted = removeAuthTokens(payload);
  assert.equal(extracted.sessionToken, "s".repeat(32));
  assert.equal(extracted.csrfToken, "c".repeat(64));
  assert.equal("session_token" in extracted.payload.data, false);
  assert.equal("csrf_token" in extracted.payload.data, false);
});

test("registration, login, and custom top-up bodies match the upstream contract", async () => {
  const registerRoute = matchRoute("POST", "auth/register");
  const registerRequest = new Request("https://niko-ai.cc/api/niko/v1/auth/register", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      username: "niko-user",
      password: "Password123",
      turnstile_token: "t".repeat(20),
    }),
  });
  assert.deepEqual(await parseBody(registerRequest, registerRoute), {
    username: "niko-user",
    password: "Password123",
    turnstile_token: "t".repeat(20),
  });

  const loginRoute = matchRoute("POST", "auth/login");
  const loginRequest = new Request("https://niko-ai.cc/api/niko/v1/auth/login", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({
      account: "user@example.com",
      password: "Password123",
      turnstile_token: "t".repeat(20),
    }),
  });
  assert.deepEqual(await parseBody(loginRequest, loginRoute), {
    username: "user@example.com",
    password: "Password123",
    turnstile_token: "t".repeat(20),
  });

  const topupRoute = matchRoute("POST", "wallet/topup-orders");
  const topupRequest = new Request(
    "https://niko-ai.cc/api/niko/v1/wallet/topup-orders",
    {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        amount: "12.30",
        currency: "CNY",
        payment_channel: "alipay",
      }),
    },
  );
  assert.deepEqual(await parseBody(topupRequest, topupRoute), {
    amount_minor: 1230,
    currency: "CNY",
    payment_channel: "alipay",
  });
});
