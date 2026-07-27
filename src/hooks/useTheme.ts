import { useEffect, useState } from "react";

export type Theme = "light" | "dark";

const KEY = "niko_theme";

function applyTheme(theme: Theme) {
  document.documentElement.classList.toggle("dark", theme === "dark");
}

/** 读取已保存的主题，默认浅色 */
export function initTheme(): Theme {
  const saved = localStorage.getItem(KEY);
  const theme: Theme = saved === "dark" ? "dark" : "light";
  applyTheme(theme);
  return theme;
}

export function useTheme() {
  const [theme, setTheme] = useState<Theme>(() => initTheme());

  useEffect(() => {
    applyTheme(theme);
    localStorage.setItem(KEY, theme);
  }, [theme]);

  return { theme, toggle: () => setTheme((t) => (t === "dark" ? "light" : "dark")) };
}
