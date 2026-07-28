import { apiRequest, unwrap } from "./api.js";

const navigation = document.querySelector("[data-session-nav]");

if (navigation) {
  const loggedOut = navigation.querySelector("[data-logged-out]");
  const loggedIn = navigation.querySelector("[data-logged-in]");
  const accountName = navigation.querySelector("[data-account-name]");

  apiRequest("/auth/session")
    .then((payload) => {
      const data = unwrap(payload) || {};
      const authenticated = data.authenticated === true || Boolean(data.user);
      loggedOut.hidden = authenticated;
      loggedIn.hidden = !authenticated;
      if (authenticated && accountName) {
        const username = data.user?.username || data.username;
        accountName.textContent = username || "个人中心";
      }
    })
    .catch(() => {
      loggedOut.hidden = false;
      loggedIn.hidden = true;
    })
    .finally(() => {
      navigation.removeAttribute("aria-busy");
    });
}
