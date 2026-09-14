import { Copy, Eye, EyeOff, Trash2 } from "lucide-react";

import type { SaveState } from "@/hooks/use-autosave";

/** `empty` is the caller's veto: a blank value is never written. */
export type RowStatus = SaveState | "empty";

const STATUS_LABEL: Record<RowStatus, string> = {
  pending: "edited",
  saving: "saving…",
  saved: "saved",
  error: "failed",
  empty: "empty",
};

const STATUS_TONE: Record<RowStatus, string> = {
  pending: "text-muted-foreground",
  saving: "text-muted-foreground",
  saved: "text-brand",
  error: "text-destructive",
  empty: "text-destructive",
};

const MASK = "••••••••••••";

export interface SecretRowProps {
  name: string;
  /** Environment the stored value lives in. */
  env: string;
  /** True when the value comes from an environment this one extends. */
  inherited: boolean;
  /** The value to show, or null while it is still hidden. */
  value: string | null;
  status?: RowStatus;
  onReveal: () => void;
  onEdit: (value: string) => void;
  /** Write now rather than waiting out the debounce. */
  onFlush: () => void;
  /** Throw away the local edit and go back to what is stored. */
  onRevert: () => void;
  onCopy: () => void;
  onDelete: () => void;
}

export function SecretRow({
  name,
  env,
  inherited,
  value,
  status,
  onReveal,
  onEdit,
  onFlush,
  onRevert,
  onCopy,
  onDelete,
}: SecretRowProps) {
  const revealed = value !== null;

  return (
    <div className="group flex items-center gap-3 border-b border-line px-6 py-3 transition-colors hover:bg-surface-tint">
      <span
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-2 font-mono text-[13px] text-ink"
        onClick={onReveal}
        title={inherited ? `${name} — inherited from ${env}` : name}
      >
        <span className="truncate">{name}</span>
        {inherited && (
          <span className="shrink-0 border border-line-strong px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-[0.14em] text-muted-foreground">
            {env}
          </span>
        )}
      </span>

      <input
        value={revealed ? value : MASK}
        readOnly={!revealed}
        onChange={(e) => onEdit(e.target.value)}
        // A hidden value cannot be edited: typing over dots you cannot read is
        // how the wrong secret gets saved silently.
        onClick={revealed ? undefined : onReveal}
        onBlur={onFlush}
        onKeyDown={(e) => {
          if (e.key === "Enter") {
            e.preventDefault();
            e.currentTarget.blur();
          } else if (e.key === "Escape") {
            e.preventDefault();
            onRevert();
          }
        }}
        spellCheck={false}
        autoComplete="off"
        autoCorrect="off"
        aria-label={revealed ? `Value of ${name}` : `${name} — hidden`}
        title={
          revealed
            ? inherited
              ? `Editing saves an override in this environment (currently from ${env})`
              : "Edits save automatically"
            : "Click to reveal and edit"
        }
        className={`w-64 shrink-0 truncate border bg-transparent px-2 py-1 font-mono text-[13px] transition-colors focus-visible:outline-none ${
          revealed
            ? "border-transparent text-ink hover:border-line focus:border-line-strong"
            : "cursor-pointer select-none border-transparent text-muted-foreground"
        }`}
      />

      <span
        className={`w-16 shrink-0 text-right font-mono text-[10px] uppercase tracking-[0.14em] ${
          status ? STATUS_TONE[status] : "text-transparent"
        }`}
        aria-live="polite"
      >
        {status ? STATUS_LABEL[status] : ""}
      </span>

      <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
        <button
          onClick={onReveal}
          className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-accent/50 hover:text-ink"
          aria-label={revealed ? "Hide" : "Reveal"}
        >
          {revealed ? (
            <EyeOff className="h-4 w-4" />
          ) : (
            <Eye className="h-4 w-4" />
          )}
        </button>
        <button
          onClick={onCopy}
          className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-accent/50 hover:text-ink"
          aria-label="Copy"
        >
          <Copy className="h-4 w-4" />
        </button>
        <button
          onClick={onDelete}
          disabled={inherited}
          className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-destructive/20 hover:text-destructive disabled:cursor-not-allowed disabled:opacity-40 disabled:hover:bg-transparent disabled:hover:text-muted-foreground"
          aria-label={inherited ? `Inherited from ${env}` : "Delete"}
        >
          <Trash2 className="h-4 w-4" />
        </button>
      </div>
    </div>
  );
}

export default SecretRow;
