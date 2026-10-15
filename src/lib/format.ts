// Locale-aware formatting. Sizes use binary units with the familiar "GB" labels, the way
// Windows Explorer shows them, so numbers match what the user sees elsewhere.

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

export function formatBytes(bytes: number, locale = "en", digits?: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** exp;
  const d = digits ?? (exp === 0 ? 0 : value >= 100 ? 0 : 1);
  const n = new Intl.NumberFormat(locale, { minimumFractionDigits: d, maximumFractionDigits: d }).format(value);
  return `${n} ${UNITS[exp]}`;
}

/** Splits a size into number and unit, for large display figures. */
export function splitBytes(bytes: number, locale = "en"): { value: string; unit: string } {
  const [value = "0", unit = "B"] = formatBytes(bytes, locale).split(" ");
  return { value, unit };
}

export function formatCount(n: number, locale = "en"): string {
  return new Intl.NumberFormat(locale).format(n);
}

export function formatPercent(part: number, whole: number, locale = "en"): string {
  if (whole <= 0) return "0%";
  const pct = (part / whole) * 100;
  return new Intl.NumberFormat(locale, { style: "percent", maximumFractionDigits: pct < 1 && pct > 0 ? 1 : 0 }).format(part / whole);
}

export function formatDate(unixSeconds: number, locale = "en"): string {
  if (!unixSeconds) return "";
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "short", day: "numeric" }).format(new Date(unixSeconds * 1000));
}

export function formatDateTime(iso: string, locale = "en"): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return new Intl.DateTimeFormat(locale, { year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(d);
}

/** "3 days ago" / "3 gün önce". */
export function formatAgo(unixSeconds: number, locale = "en", now = Date.now()): string {
  if (!unixSeconds) return "";
  const diff = unixSeconds * 1000 - now;
  const rtf = new Intl.RelativeTimeFormat(locale, { numeric: "auto" });
  const abs = Math.abs(diff);
  const minute = 60_000;
  const hour = 60 * minute;
  const day = 24 * hour;
  if (abs < hour) return rtf.format(Math.round(diff / minute), "minute");
  if (abs < day) return rtf.format(Math.round(diff / hour), "hour");
  if (abs < 30 * day) return rtf.format(Math.round(diff / day), "day");
  if (abs < 365 * day) return rtf.format(Math.round(diff / (30 * day)), "month");
  return rtf.format(Math.round(diff / (365 * day)), "year");
}

export function daysSince(unixSeconds: number, now = Date.now()): number {
  if (!unixSeconds) return Infinity;
  return Math.floor((now - unixSeconds * 1000) / 86_400_000);
}

/** Last path segment, for compact labels. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  return i >= 0 ? trimmed.slice(i + 1) : trimmed;
}

export function parentPath(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  return i > 0 ? trimmed.slice(0, i) : trimmed;
}

/** Replaces the user profile prefix with `~` to keep long paths readable. */
export function shortPath(path: string, profile: string): string {
  const p = profile.replace(/[\\/]+$/, "");
  if (p && path.length > p.length && path.toLowerCase().startsWith(p.toLowerCase() + "\\")) return "~" + path.slice(p.length);
  return path;
}

export function extOf(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(i + 1).toLowerCase() : "";
}
