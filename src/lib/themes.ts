// Colour themes. Each theme is a set of CSS custom properties applied to the
// document root; "system" follows the desktop's light/dark preference.

export type ThemeId = "system" | "light" | "dark" | "ubuntu-dark";

export interface Theme {
  id: Exclude<ThemeId, "system">;
  name: string;
  description: string;
  dark: boolean;
  tokens: Record<string, string>;
}

const light: Theme = {
  id: "light",
  name: "Arbor Light",
  description: "Cool greys with a teal accent.",
  dark: false,
  tokens: {
    bg: "#e9edef", surface: "#fbfcfc", "surface-2": "#f2f5f6", "surface-3": "#e6ebed",
    line: "#d6dde1", "line-soft": "#e4e9ec",
    ink: "#1a2227", "ink-2": "#4f5c64", "ink-3": "#85919a",
    accent: "#1b6b70", "accent-ink": "#ffffff", "accent-soft": "rgba(27, 107, 112, 0.12)",
    sel: "rgba(27, 107, 112, 0.14)", "sel-strong": "rgba(27, 107, 112, 0.24)", focus: "#2b8f96",
    danger: "#b23b2a", "danger-soft": "rgba(178, 59, 42, 0.1)",
    warn: "#9a6a0a", "warn-soft": "rgba(224, 164, 46, 0.18)",
    "add-bg": "#e3f3e6", "add-bg-strong": "#c5e8cd", "add-ink": "#1e6a37",
    "del-bg": "#fbe8e4", "del-bg-strong": "#f4cbc2", "del-ink": "#9b3423",
    "lane-0": "#1f9e8f", "lane-1": "#d9952a", "lane-2": "#d15c78", "lane-3": "#4677cf",
    "lane-4": "#8a68cc", "lane-5": "#6f9431", "lane-6": "#d4703a", "lane-7": "#5f7f90",
    "av-bg-s": "45%", "av-bg-l": "88%", "av-ink-s": "45%", "av-ink-l": "28%",
    "btn-bg": "#fbfcfc", "btn-hover": "#f2f5f6", "input-bg": "#fbfcfc",
    "shadow-pop": "0 12px 32px rgba(18, 32, 40, 0.18), 0 2px 6px rgba(18, 32, 40, 0.1)",
    "backdrop": "rgba(10, 18, 22, 0.32)",
    "syn-keyword": "#8a3fa0", "syn-string": "#2c7a35", "syn-number": "#b0571f", "syn-comment": "#7f8b93", "syn-title": "#1d5fa8", "syn-type": "#0f7477",
    "syn-attr": "#9a5d12", "syn-meta": "#66727a", "syn-variable": "#a23f2f", "syn-regexp": "#2c7a35", "syn-builtin": "#0f7477",
  },
};

const dark: Theme = {
  id: "dark",
  name: "Arbor Dark",
  description: "Deep blue-greys with a teal accent.",
  dark: true,
  tokens: {
    bg: "#14181b", surface: "#1b2024", "surface-2": "#20262a", "surface-3": "#283035",
    line: "#2d353a", "line-soft": "#252c31",
    ink: "#e3e8eb", "ink-2": "#a5b0b7", "ink-3": "#6e7a82",
    accent: "#5cbfb3", "accent-ink": "#0d1b1d", "accent-soft": "rgba(92, 191, 179, 0.14)",
    sel: "rgba(92, 191, 179, 0.14)", "sel-strong": "rgba(92, 191, 179, 0.26)", focus: "#5cbfb3",
    danger: "#ec7b67", "danger-soft": "rgba(236, 123, 103, 0.12)",
    warn: "#e6b450", "warn-soft": "rgba(230, 180, 80, 0.14)",
    "add-bg": "rgba(77, 170, 103, 0.13)", "add-bg-strong": "rgba(77, 170, 103, 0.3)", "add-ink": "#8fd6a3",
    "del-bg": "rgba(220, 96, 74, 0.13)", "del-bg-strong": "rgba(220, 96, 74, 0.3)", "del-ink": "#f09a89",
    "lane-0": "#49c2b2", "lane-1": "#e9b04a", "lane-2": "#e57b93", "lane-3": "#719ff0",
    "lane-4": "#a98ae8", "lane-5": "#98c056", "lane-6": "#ec915c", "lane-7": "#8aa6b5",
    "av-bg-s": "30%", "av-bg-l": "26%", "av-ink-s": "55%", "av-ink-l": "82%",
    "btn-bg": "#1b2024", "btn-hover": "#20262a", "input-bg": "#1b2024",
    "shadow-pop": "0 14px 36px rgba(0, 0, 0, 0.5), 0 2px 6px rgba(0, 0, 0, 0.3)",
    "backdrop": "rgba(0, 0, 0, 0.45)",
    "syn-keyword": "#c792ea", "syn-string": "#a3d083", "syn-number": "#f2a46b", "syn-comment": "#6e7a82", "syn-title": "#82aaff", "syn-type": "#5cbfb3",
    "syn-attr": "#e6b450", "syn-meta": "#8a96a0", "syn-variable": "#f07178", "syn-regexp": "#a3d083", "syn-builtin": "#5cbfb3",
  },
};

// Ubuntu's Yaru dark style: GNOME's neutral warm greys (header bars and
// sidebars a step lighter than content), Ubuntu orange for selection and
// accents, and the GNOME palette for everything else.
const ubuntuDark: Theme = {
  id: "ubuntu-dark",
  name: "Ubuntu Dark",
  description: "Matches Ubuntu's Yaru dark style, with Ubuntu orange accents.",
  dark: true,
  tokens: {
    bg: "#303030", surface: "#242424", "surface-2": "#2a2a2a", "surface-3": "#3a3a3a",
    line: "#1c1c1c", "line-soft": "#2e2e2e",
    ink: "#ffffff", "ink-2": "#c0bfbc", "ink-3": "#9a9996",
    accent: "#e95420", "accent-ink": "#ffffff", "accent-soft": "rgba(233, 84, 32, 0.16)",
    sel: "rgba(233, 84, 32, 0.22)", "sel-strong": "rgba(233, 84, 32, 0.36)", focus: "#f0835b",
    danger: "#ff7b63", "danger-soft": "rgba(255, 123, 99, 0.14)",
    warn: "#f8e45c", "warn-soft": "rgba(246, 211, 45, 0.14)",
    "add-bg": "rgba(38, 162, 105, 0.17)", "add-bg-strong": "rgba(38, 162, 105, 0.34)", "add-ink": "#8ff0a4",
    "del-bg": "rgba(224, 27, 36, 0.17)", "del-bg-strong": "rgba(224, 27, 36, 0.34)", "del-ink": "#f66151",
    "lane-0": "#e95420", "lane-1": "#f6d32d", "lane-2": "#dc8add", "lane-3": "#62a0ea",
    "lane-4": "#c061cb", "lane-5": "#57e389", "lane-6": "#5bc8af", "lane-7": "#9a9996",
    "av-bg-s": "22%", "av-bg-l": "30%", "av-ink-s": "60%", "av-ink-l": "84%",
    "btn-bg": "rgba(255, 255, 255, 0.08)", "btn-hover": "rgba(255, 255, 255, 0.13)", "input-bg": "rgba(255, 255, 255, 0.06)",
    "shadow-pop": "0 14px 36px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(255, 255, 255, 0.06)",
    "backdrop": "rgba(0, 0, 0, 0.5)",
    "syn-keyword": "#f2894c", "syn-string": "#9fcf63", "syn-number": "#c79bea", "syn-comment": "#808080", "syn-title": "#79a9e3", "syn-type": "#4fc4b8",
    "syn-attr": "#e8c06b", "syn-meta": "#909090", "syn-variable": "#f07a6e", "syn-regexp": "#9fcf63", "syn-builtin": "#4fc4b8",
  },
};

export const THEMES: Theme[] = [light, dark, ubuntuDark];

const media = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

export function resolveTheme(id: ThemeId): Theme {
  if (id === "system") return media?.matches ? dark : light;
  return THEMES.find((t) => t.id === id) ?? light;
}

let current: ThemeId = "system";

export function applyTheme(id: ThemeId) {
  current = id;
  const t = resolveTheme(id);
  const root = document.documentElement;
  for (const [k, v] of Object.entries(t.tokens)) root.style.setProperty(`--${k}`, v);
  root.style.colorScheme = t.dark ? "dark" : "light";
  root.dataset.theme = t.id;
  root.dataset.dark = String(t.dark);
}

// Follow the desktop when the theme is "system".
media?.addEventListener("change", () => {
  if (current === "system") applyTheme("system");
});
