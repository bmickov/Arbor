export function relTime(sec: number): string {
  const d = Date.now() / 1000 - sec;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  if (d < 86400 * 7) {
    const n = Math.floor(d / 86400);
    return n === 1 ? "yesterday" : `${n} days ago`;
  }
  return fullDate(sec, false);
}

export function fullDate(sec: number, withTime = true): string {
  const dt = new Date(sec * 1000);
  const sameYear = dt.getFullYear() === new Date().getFullYear();
  const date = dt.toLocaleDateString(undefined, { day: "numeric", month: "short", year: sameYear ? undefined : "numeric" });
  if (!withTime) return date;
  return `${date}, ${dt.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`;
}

/** Lane palette for the commit graph; index wraps. */
export const LANES = 8;
export const laneVar = (i: number) => `var(--lane-${i % LANES})`;

export function basename(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? p : p.slice(i + 1);
}
export function dirname(p: string): string {
  const i = p.lastIndexOf("/");
  return i < 0 ? "" : p.slice(0, i + 1);
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/).filter(Boolean);
  if (parts.length === 0) return "?";
  if (parts.length === 1) return parts[0].slice(0, 2).toUpperCase();
  return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
}

/** Stable hue per author so avatars are recognisable at a glance. */
export function authorHue(email: string): number {
  let h = 0;
  for (let i = 0; i < email.length; i++) h = (h * 31 + email.charCodeAt(i)) >>> 0;
  return h % 360;
}

export const STATUS_GLYPH: Record<string, string> = {
  added: "A", untracked: "U", modified: "M", deleted: "D", renamed: "R", typechange: "T", conflicted: "C", copied: "C",
};

export const IMAGE_EXT = /\.(png|jpe?g|gif|webp|bmp|ico|svg|avif)$/i;
