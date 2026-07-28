import { apiRequest, unwrap } from "./api.js";

const navigation =
  typeof document === "undefined" ? null : document.querySelector("[data-session-nav]");

export function renderSessionNavigation(navigationElement, payload) {
  const loggedOut = navigationElement.querySelector("[data-logged-out]");
  const loggedIn = navigationElement.querySelector("[data-logged-in]");
  const accountName = navigationElement.querySelector("[data-account-name]");
  const data = unwrap(payload) || {};
  const authenticated = data.authenticated === true || Boolean(data.user);
  loggedOut.hidden = authenticated;
  loggedIn.hidden = !authenticated;
  if (accountName) {
    const username = data.user?.username || data.username;
    accountName.textContent = authenticated ? username || "个人中心" : "个人中心";
  }
}

export async function refreshSessionNavigation(
  navigationElement,
  request = apiRequest,
) {
  const loggedOut = navigationElement.querySelector("[data-logged-out]");
  const loggedIn = navigationElement.querySelector("[data-logged-in]");
  try {
    renderSessionNavigation(navigationElement, await request("/auth/session"));
  } catch {
    loggedOut.hidden = false;
    loggedIn.hidden = true;
  } finally {
    navigationElement.removeAttribute("aria-busy");
  }
}

export async function logoutFromNavigation(navigationElement, request = apiRequest) {
  const button = navigationElement.querySelector("[data-session-logout]");
  button.disabled = true;
  try {
    await request("/auth/logout", { method: "POST", body: {} });
    navigationElement.setAttribute("aria-busy", "true");
    await refreshSessionNavigation(navigationElement, request);
  } finally {
    button.disabled = false;
  }
}

if (navigation) {
  navigation.querySelector("[data-session-logout]").addEventListener("click", () => {
    logoutFromNavigation(navigation).catch(() => {
      navigation.removeAttribute("aria-busy");
    });
  });
  refreshSessionNavigation(navigation);
}
