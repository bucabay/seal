import type { RefRow } from "./api";

/**
 * Where a reference with no issuer is filed. Mirrors
 * `keymaker_core::reference::DEFAULT_ISSUER`.
 */
export const DEFAULT_ISSUER = "general";

export type Group = { issuer: string; rows: RefRow[]; stored: number };

/**
 * Split a pasted `reference value` line.
 *
 * Mirrors `keymaker_core::reference::split_pasted`: the *first* run of
 * whitespace separates, so a value may contain spaces, as passphrases often do.
 */
export function splitPasted(line: string): { reference: string; value?: string } {
  const trimmed = line.replace(/^\s+/, "");
  const at = trimmed.search(/\s/);
  if (at === -1) return { reference: trimmed };
  const reference = trimmed.slice(0, at);
  const value = trimmed.slice(at).replace(/^\s+/, "");
  return value ? { reference, value } : { reference };
}

/**
 * Does this row match what was typed?
 *
 * Matches the reference and the projects using it, so typing a project name
 * finds everything it uses — which is the project view, without a project
 * navigation mode having to exist.
 */
export function matches(row: RefRow, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    row.reference.toLowerCase().includes(q) ||
    row.used_by.some((u) => u.toLowerCase().includes(q))
  );
}

/**
 * Group rows by issuer for display.
 *
 * Groups are derived, never stored, so they cannot fall out of step with the
 * keys they hold. Within a group, references with no value sort first: those
 * are what people come here to fix.
 */
export function grouped(rows: RefRow[], query: string): Group[] {
  const byIssuer = new Map<string, RefRow[]>();
  for (const row of rows) {
    if (!matches(row, query)) continue;
    const list = byIssuer.get(row.issuer) ?? [];
    list.push(row);
    byIssuer.set(row.issuer, list);
  }

  return [...byIssuer.entries()]
    .map(([issuer, list]) => ({
      issuer,
      rows: [...list].sort((a, b) => {
        if (a.present !== b.present) return a.present ? 1 : -1;
        return a.reference.localeCompare(b.reference);
      }),
      stored: list.filter((r) => r.present).length,
    }))
    .sort((a, b) => {
      // Named issuers first; the catch-all sits at the bottom where it belongs,
      // whatever it happens to be called alphabetically.
      if (a.issuer === DEFAULT_ISSUER) return 1;
      if (b.issuer === DEFAULT_ISSUER) return -1;
      return a.issuer.localeCompare(b.issuer);
    });
}

const COLLAPSED_KEY = "keymaker.collapsed-groups";

/**
 * Should this group's rows be on screen?
 *
 * A filter overrides a collapse. Hiding a search result behind a collapsed
 * header is worse than useless — the user has said what they are looking for,
 * and an empty-looking result would read as "not here".
 */
export function isExpanded(
  issuer: string,
  collapsed: ReadonlySet<string>,
  query: string,
): boolean {
  if (query.trim()) return true;
  return !collapsed.has(issuer);
}

/** Collapsed groups survive a restart; a desktop app should remember. */
export function loadCollapsed(): Set<string> {
  try {
    const raw = localStorage.getItem(COLLAPSED_KEY);
    return new Set(raw ? (JSON.parse(raw) as string[]) : []);
  } catch {
    // Storage can be unavailable or hold nonsense; an empty set means
    // everything shows, which is the safe way to be wrong.
    return new Set();
  }
}

export function saveCollapsed(collapsed: ReadonlySet<string>): void {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
  } catch {
    // Not remembering is a small loss; failing to collapse is not.
  }
}
