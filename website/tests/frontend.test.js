import assert from "node:assert/strict";
import test from "node:test";

import {
  createTopupIdempotencyState,
  submitPaymentForm,
} from "../src/js/payment.js";
import {
  logoutFromNavigation,
  renderSessionNavigation,
} from "../src/js/site-session.js";

function fakeDocument() {
  const appended = [];
  return {
    appended,
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

test("payment handoff builds and submits a hidden HTTPS POST form", () => {
  const documentRef = fakeDocument();
  const form = submitPaymentForm(
    "https://pay.example.com/submit.php",
    { pid: "10001", sign: "abc123" },
    documentRef,
  );

  assert.equal(documentRef.appended[0], form);
  assert.equal(form.method, "POST");
  assert.equal(form.action, "https://pay.example.com/submit.php");
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
