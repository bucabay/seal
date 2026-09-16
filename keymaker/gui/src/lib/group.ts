import type { RefRow } from "./api";

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
    .sort((a, b) => a.issuer.localeCompare(b.issuer));
}
