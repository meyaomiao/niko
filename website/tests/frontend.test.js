import assert from "node:assert/strict";
import test from "node:test";

import {
  createTopupIdempotencyState,
  createTopupPaymentState,
  submitPaymentForm,
} from "../src/js/payment.js";
import {
  logoutFromNavigation,
  renderSessionNavigation,
} from "../src/js/site-session.js";
import {
  DESKTOP_TURNSTILE_ACTION,
  validDesktopNonce,
  validDesktopSiteKey,
  verificationCallbackUrl,
} from "../src/js/desktop-verification.js";

test("desktop verification keeps the production action and nonce/site-key-bound callback", () => {
  const nonce = "a".repeat(32);
  const siteKey = "0x4AAAAAAD_7tPGZn65hZ-Ov";
  const token = `token+/${"x".repeat(20)}`;
  assert.equal(DESKTOP_TURNSTILE_ACTION, "niko_register");
  assert.equal(validDesktopNonce(nonce), true);
  assert.equal(validDesktopNonce("wrong"), false);
  assert.equal(validDesktopSiteKey(siteKey), true);
  assert.equal(validDesktopSiteKey("wrong"), false);

  const callback = new URL(verificationCallbackUrl(nonce, siteKey, token));
  assert.equal(callback.protocol, "niko-register:");
  assert.equal(callback.host, "verified");
  assert.equal(callback.searchParams.get("nonce"), nonce);
  assert.equal(callback.searchParams.get("site_key"), siteKey);
  assert.equal(callback.searchParams.get("token"), token);
  assert.throws(() => verificationCallbackUrl(nonce, "wrong", token));
});

function fakeDocument() {
  const appended = [];
  const removed = [];
  return {
    appended,
    removed,
    body: {
      append(element) {
        appended.push(element);
      },
    },
    createElement(tagName) {
      const element = {
        tagName,
        children: [],
        append(child) {
          this.children.push(child);
        },
        remove() {
          removed.push(this);
          const index = appended.indexOf(this);
          if (index !== -1) {
            appended.splice(index, 1);
          }
        },
      };
      if (tagName === "form") {
        element.submitCount = 0;
        element.submit = function submit() {
          this.submitCount += 1;
        };
      }
      return element;
    },
  };
}

function fakeNavigation() {
  const elements = {
    "[data-logged-out]": { hidden: true },
    "[data-logged-in]": { hidden: false },
    "[data-account-name]": { textContent: "alice" },
    "[data-session-logout]": { disabled: false },
  };
  return {
    elements,
    attributes: new Map(),
    querySelector(selector) {
      return elements[selector];
    },
    setAttribute(name, value) {
      this.attributes.set(name, value);
    },
    removeAttribute(name) {
      this.attributes.delete(name);
    },
  };
}

test("payment handoff submits a temporary hidden HTTPS POST form in a new tab", async () => {
  const documentRef = fakeDocument();
  const form = submitPaymentForm(
    "https://pay.example.com/submit.php",
    { pid: "10001", sign: "abc123" },
    documentRef,
  );

  assert.equal(documentRef.appended[0], form);
  assert.equal(form.method, "POST");
  assert.equal(form.action, "https://pay.example.com/submit.php");
  assert.equal(form.target, "_blank");
  assert.equal(form.acceptCharset, "UTF-8");
  assert.equal(form.hidden, true);
  assert.equal(form.submitCount, 1);
  assert.deepEqual(
    form.children.map(({ type, name, value }) => ({ type, name, value })),
    [
      { type: "hidden", name: "pid", value: "10001" },
      { type: "hidden", name: "sign", value: "abc123" },
    ],
  );

  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(documentRef.appended, []);
  assert.equal(documentRef.removed[0], form);
});

test("payment handoff removes its form when submission throws", () => {
  const documentRef = fakeDocument();
  const originalCreateElement = documentRef.createElement;
  documentRef.createElement = (tagName) => {
    const element = originalCreateElement(tagName);
    if (tagName === "form") {
      element.submit = () => {
        throw new Error("blocked");
      };
    }
    return element;
  };

  assert.throws(
    () => submitPaymentForm("https://pay.example.com/submit.php", { pid: "10001" }, documentRef),
    /blocked/,
  );
  assert.deepEqual(documentRef.appended, []);
  assert.equal(documentRef.removed.length, 1);
});

test("payment handoff rejects non-plain parameter objects", () => {
  const documentRef = fakeDocument();
  const paymentParams = Object.assign(Object.create(null), { pid: "10001" });

  assert.throws(
    () => submitPaymentForm("https://pay.example.com/submit.php", paymentParams, documentRef),
    /Invalid payment parameters/,
  );
  assert.deepEqual(documentRef.appended, []);
});

test("top-up retries reuse a key until parameters change or creation succeeds", () => {
  let sequence = 0;
  const state = createTopupIdempotencyState(() => `key-${++sequence}`);
  const first = { amount: "10.00", currency: "CNY", payment_channel: "alipay" };

  assert.equal(state.keyFor(first), "key-1");
  assert.equal(state.keyFor({ ...first }), "key-1");
  assert.equal(state.keyFor({ ...first, amount: "20.00" }), "key-2");
  assert.equal(state.keyFor({ ...first, currency: "USD" }), "key-3");
  assert.equal(state.keyFor({ ...first, payment_channel: "wxpay" }), "key-4");
  state.clear();
  assert.equal(state.keyFor(first), "key-5");

  const refreshedPageState = createTopupIdempotencyState(() => "fresh-page-key");
  assert.equal(refreshedPageState.keyFor(first), "fresh-page-key");
});

test("created payment handoff can be reopened without creating another order", async () => {
  let createCount = 0;
  const opened = [];
  const documentRef = fakeDocument();
  const state = createTopupPaymentState((paymentUrl, paymentParams, openedDocument, formTarget) => {
    opened.push({ paymentUrl, paymentParams, documentRef: openedDocument, formTarget });
  });
  const createPayment = async () => {
    createCount += 1;
    return {
      paymentUrl: "https://pay.example.com/submit.php",
      paymentParams: { pid: "10001", sign: "abc123" },
      orderId: "NKO123",
    };
  };

  assert.equal(state.status(), "idle");
  const first = await state.create(createPayment);
  assert.equal(first.created, true);
  assert.equal(state.status(), "ready");
  state.open(documentRef, "_blank");

  const retry = await state.create(createPayment);
  assert.equal(retry.created, false);
  state.open(documentRef, "_blank");

  assert.equal(createCount, 1);
  assert.deepEqual(opened, [first.handoff, first.handoff].map(({ orderId, ...handoff }) => ({
    ...handoff,
    documentRef,
    formTarget: "_blank",
  })));

  state.clear();
  assert.equal(state.status(), "idle");
});

test("payment handoff exposes the creating to ready state transition", async () => {
  let resolvePayment;
  const state = createTopupPaymentState();
  const creation = state.create(
    () =>
      new Promise((resolve) => {
        resolvePayment = resolve;
      }),
  );

  await Promise.resolve();
  assert.equal(state.status(), "creating");
  resolvePayment({
    paymentUrl: "https://pay.example.com/submit.php",
    paymentParams: { pid: "10001" },
  });
  await creation;
  assert.equal(state.status(), "ready");
});

test("homepage logout uses the authenticated mutation and refreshes session state", async () => {
  const navigation = fakeNavigation();
  const requests = [];
  const request = async (path, options) => {
    requests.push({ path, options });
    return path === "/auth/session" ? { data: { authenticated: false } } : {};
  };

  renderSessionNavigation(navigation, {
    data: { authenticated: true, user: { username: "alice" } },
  });
  assert.equal(navigation.elements["[data-logged-in]"].hidden, false);
  assert.equal(navigation.elements["[data-account-name]"].textContent, "alice");

  await logoutFromNavigation(navigation, request);

  assert.deepEqual(requests, [
    { path: "/auth/logout", options: { method: "POST", body: {} } },
    { path: "/auth/session", options: undefined },
  ]);
  assert.equal(navigation.elements["[data-logged-out]"].hidden, false);
  assert.equal(navigation.elements["[data-logged-in]"].hidden, true);
  assert.equal(navigation.elements["[data-session-logout]"].disabled, false);
});
