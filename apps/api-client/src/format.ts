// What every front end needs to show API data safely: escaping, the copy pack's templates, and
// money and dates as the engine sends them (integer minor units, ISO dates). Formatting only:
// no amount is ever computed here.

const ENTITIES: Record<string, string> = {
  "&": "&amp;",
  "<": "&lt;",
  ">": "&gt;",
  '"': "&quot;",
  "'": "&#39;",
};

export function escapeHtml(text: string): string {
  return text.replace(/[&<>"']/g, (c) => ENTITIES[c] ?? c);
}

/** A copy pack's text by key. */
export type Copy = Record<string, string>;

/**
 * The text for `key` with each `{name}` replaced by `vars[name]`, escaped for HTML. A missing key
 * shows the key itself, so a gap is visible rather than blank.
 */
export function t(copy: Copy, key: string, vars: Record<string, string | number> = {}): string {
  const template = copy[key] ?? key;
  return template
    .split(/(\{[a-z_]+\})/)
    .map((part) => {
      const name = /^\{([a-z_]+)\}$/.exec(part)?.[1];
      const value = name === undefined ? part : String(vars[name] ?? part);
      return escapeHtml(value);
    })
    .join("");
}

/** The `{placeholders}` a template uses. */
export function placeholders(template: string): string[] {
  return [...template.matchAll(/\{([a-z_]+)\}/g)].map((m) => m[1] ?? "").sort();
}

/**
 * Money in minor units, in the market's currency (ISO 4217, from the registry), as people read it
 * in `language`: the platform's own currency formatting, so no market is written here (ADR-012).
 */
export function money(minor: number, currency: string, language = "en"): string {
  const amount = minor / 100;
  try {
    return new Intl.NumberFormat(language, {
      style: "currency",
      currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: 2,
    }).format(amount);
  } catch {
    return `${currency} ${amount.toFixed(2)}`;
  }
}

/** Basis points as a percentage with up to one decimal. */
export function percent(bps: number): string {
  const p = bps / 100;
  return `${Number.isInteger(p) ? p.toFixed(0) : p.toFixed(1)}%`;
}

/** An ISO date (`2026-11-19`) as people read it in `language`. */
export function day(iso: string, language = "en"): string {
  const [y, m, d] = iso.slice(0, 10).split("-").map(Number);
  if (!y || !m || !d) {
    return iso;
  }
  if (language.startsWith("zh")) {
    return `${y}年${m}月${d}日`;
  }
  const months = [
    "Jan",
    "Feb",
    "Mar",
    "Apr",
    "May",
    "Jun",
    "Jul",
    "Aug",
    "Sep",
    "Oct",
    "Nov",
    "Dec",
  ];
  return `${d} ${months[m - 1]} ${y}`;
}
