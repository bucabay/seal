/**
 * What a keystroke means while entering a secret.
 *
 * One definition, used by the add form and by every existing row, so the
 * rhythm is the same wherever you are typing:
 *
 * - in a **name**, Enter and Tab both move on to the value
 * - in a **value**, Enter and Tab both save and open the next row
 * - Escape backs out
 *
 * Tab and Enter are deliberately the same key here. Entering secrets is a run
 * of name/value pairs, and having to remember which one advances and which one
 * commits is a rule with no payoff.
 */
export type EntryField = "name" | "value";

export type EntryAction =
  /** Move to the value field. */
  | "next"
  /** Save what is typed and open the next row. */
  | "commit"
  /** Abandon this entry. */
  | "cancel"
  /** Nothing special; let the browser handle it. */
  | null;

export interface Keystroke {
  key: string;
  shiftKey?: boolean;
  metaKey?: boolean;
  ctrlKey?: boolean;
  altKey?: boolean;
  isComposing?: boolean;
}

export function actionFor(e: Keystroke, field: EntryField): EntryAction {
  if (e.isComposing) return null;
  if (e.key === "Escape") return "cancel";

  // A modifier means the user is doing something else — selecting, or using a
  // system shortcut. Never steal those.
  if (e.metaKey || e.ctrlKey || e.altKey) return null;

  // Shift-Tab is going backwards, which the browser already does correctly.
  if (e.key === "Tab" && e.shiftKey) return null;

  if (e.key === "Enter" || e.key === "Tab") {
    return field === "name" ? "next" : "commit";
  }
  return null;
}

/**
 * Should the browser's own handling be suppressed?
 *
 * All recognised actions are handled explicitly, including name-field Tab,
 * so native navigation cannot advance a second time after we move focus.
 */
export function shouldPreventDefault(e: Keystroke, field: EntryField): boolean {
  const action = actionFor(e, field);
  if (action === null) return false;
  return true;
}

/** Match the backend reference rules before advancing to the next entry. */
export function referenceProblem(reference: string): string | null {
  if (!reference) return "a reference is needed, for example stripe/api-key";
  if (/\s/.test(reference)) return "a reference cannot contain spaces";
  if (reference.startsWith("/") || reference.endsWith("/")) return "a reference cannot begin or end with `/`";
  if (reference.includes("//")) return "a reference cannot contain an empty segment";
  if (new TextEncoder().encode(reference).length > 512) return "that reference is too long";
  if (/[\x00-\x1f\x7f-\x9f]/.test(reference)) return "a reference cannot contain control characters";
  return null;
}

/**
 * The issuer to offer next, after saving `reference`.
 *
 * A prefix is a convenience, not a constraint: typing `mailkite/test` into a
 * form opened under `hardroad` saves to mailkite. Having done that, mailkite is
 * what the next key most likely belongs to, so that is what comes back.
 */
export function nextPrefix(reference: string, fallback?: string): string {
  const at = reference.indexOf("/");
  if (at > 0 && at < reference.length - 1) return `${reference.slice(0, at)}/`;
  return fallback ?? "";
}
