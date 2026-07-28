import { createIdempotencyKey } from "./api.js";

const FORBIDDEN_PARAM_KEYS = new Set(["__proto__", "constructor", "prototype"]);

function plainObject(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return false;
  }
  return Object.getPrototypeOf(value) === Object.prototype;
}

function topupFingerprint(body) {
  return JSON.stringify([
    body.option_id || "",
    body.amount || "",
    body.currency || "",
    body.payment_channel || "",
  ]);
}

export function createTopupIdempotencyState(createKey = createIdempotencyKey) {
  let fingerprint = "";
  let key = "";

  return {
    keyFor(body) {
      const nextFingerprint = topupFingerprint(body);
      if (!key || nextFingerprint !== fingerprint) {
        fingerprint = nextFingerprint;
        key = createKey();
      }
      return key;
    },
    clear() {
      fingerprint = "";
      key = "";
    },
  };
}

export function submitPaymentForm(paymentUrl, paymentParams, documentRef = document) {
  const target = new URL(paymentUrl);
  if (target.protocol !== "https:" || target.username || target.password) {
    throw new Error("Unsafe payment URL");
  }
  if (!plainObject(paymentParams)) {
    throw new Error("Invalid payment parameters");
  }

  const form = documentRef.createElement("form");
  form.method = "POST";
  form.action = target.href;
  form.acceptCharset = "UTF-8";
  form.hidden = true;

  for (const [name, value] of Object.entries(paymentParams)) {
    if (FORBIDDEN_PARAM_KEYS.has(name) || typeof value !== "string") {
      throw new Error("Invalid payment parameters");
    }
    const input = documentRef.createElement("input");
    input.type = "hidden";
    input.name = name;
    input.value = value;
    form.append(input);
  }

  documentRef.body.append(form);
  form.submit();
  return form;
}
