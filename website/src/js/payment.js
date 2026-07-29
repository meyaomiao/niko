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

export function createTopupPaymentState(openPayment = submitPaymentForm) {
  let handoff = null;
  let creating = null;

  return {
    status() {
      return handoff ? "ready" : creating ? "creating" : "idle";
    },
    async create(createPayment) {
      if (handoff) {
        return { created: false, handoff };
      }
      if (!creating) {
        creating = Promise.resolve()
          .then(createPayment)
          .then((nextHandoff) => {
            handoff = nextHandoff;
            return { created: true, handoff };
          })
          .finally(() => {
            creating = null;
          });
      }
      return creating;
    },
    open(documentRef = document, formTarget = "_blank") {
      if (!handoff) {
        throw new Error("Missing payment handoff");
      }
      return openPayment(handoff.paymentUrl, handoff.paymentParams, documentRef, formTarget);
    },
    clear() {
      handoff = null;
    },
  };
}

export function preparePaymentWindow(
  windowRef = window,
) {
  let paymentWindow = null;
  try {
    paymentWindow = windowRef.open("", "_blank");
  } catch {
    // The retry button can submit to a fresh tab from a direct user gesture.
  }
  if (!paymentWindow) {
    return { documentRef: () => null, close() {} };
  }

  try {
    paymentWindow.document.title = "正在打开支付页面";
    paymentWindow.document.body.textContent = "订单创建中，正在准备支付页面…";
  } catch {
    // The window may already have navigated or been isolated by the browser.
  }

  return {
    documentRef() {
      try {
        return paymentWindow.closed ? null : paymentWindow.document;
      } catch {
        return null;
      }
    },
    close() {
      try {
        if (!paymentWindow.closed) {
          paymentWindow.close();
        }
      } catch {
        // Ignore browser cleanup failures after a blocked or isolated navigation.
      }
    },
  };
}

export function submitPaymentForm(
  paymentUrl,
  paymentParams,
  documentRef = document,
  formTarget = "_blank",
) {
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
  form.target = formTarget;
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
  try {
    form.submit();
  } catch (error) {
    form.remove();
    throw error;
  }
  setTimeout(() => form.remove(), 0);
  return form;
}
