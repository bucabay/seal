import { Eye, EyeOff, Copy, Check, Trash2 } from "lucide-react";
import type { RefRow } from "@/lib/api";
import type { SaveState } from "@/hooks/use-autosave";
import { cn } from "@/lib/utils";
import { rawInput } from "@/lib/raw-input";

const MASK = "•".repeat(24);

/** A row's state in the write cycle, plus the one case that is not a write. */
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
  saved: "text-primary",
  error: "text-destructive",
  empty: "text-destructive",
};

/**
 * One reference.
 *
 * Revealing is deliberate and one row at a time: the point of the product is
 * that reading a value is an act, not a default. Editing, by contrast, saves
 * itself — a Save button on a field you have already decided to change is just
 * a way to lose work.
 *
 * The row is controlled: the draft and the revealed value live in App, so a
 * pending edit can be flushed when the window loses focus or the list reloads.
 */
export function SecretRow({
  row,
  revealed,
  draft,
  status,
  copied,
  onReveal,
  onHide,
  onChange,
  onFlush,
  onCopy,
  onDelete,
}: {
  row: RefRow;
  /** The revealed value, or null while hidden. */
  revealed: string | null;
  /** An unsaved edit, if there is one. */
  draft?: string;
  status?: RowStatus;
  copied: boolean;
  onReveal: () => void;
  onHide: () => void;
  onChange: (value: string) => void;
  onFlush: () => void;
  onCopy: () => void;
  onDelete: () => void;
}) {
  const isRevealed = revealed !== null;
  const shown = draft ?? revealed ?? "";
  const editable = isRevealed || !row.present;

  return (
    <div className="border-b border-line last:border-b-0">
      <div className="flex items-center gap-3 px-4 py-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <span className="truncate font-mono text-sm text-foreground">
              {row.reference}
            </span>
            {!row.present && (
              <span className="shrink-0 border border-line px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                not on this machine
              </span>
            )}
          </div>
          {row.used_by.length > 0 && (
            <div className="mt-0.5 truncate font-mono text-[11px] text-muted-foreground">
              {row.used_by.join(" · ")}
            </div>
          )}
        </div>

        {/* Fixed width so the row does not jump as the label changes. */}
        <span
          className={cn(
            "w-14 shrink-0 text-right font-mono text-[10px] uppercase tracking-wider",
            status ? STATUS_TONE[status] : "text-transparent",
          )}
          aria-live="polite"
        >
          {status ? STATUS_LABEL[status] : ""}
        </span>

        <input
          {...rawInput}
          className={cn(
            "w-[34%] shrink-0 border border-line bg-background px-2 py-1 font-mono text-xs",
            "focus:border-primary focus:outline-none",
            !editable && "cursor-pointer select-none text-muted-foreground",
          )}
          value={editable ? shown : MASK}
          placeholder={row.present ? "" : "type a value — it saves itself"}
          readOnly={!editable}
          onClick={() => !isRevealed && row.present && onReveal()}
          onChange={(e) => onChange(e.target.value)}
          // Leaving the field is a clear sign the edit is finished; do not make
          // the user wait out the debounce.
          onBlur={onFlush}
          aria-label={isRevealed ? `Value of ${row.reference}` : `${row.reference}, hidden`}
        />

        <div className="flex shrink-0 items-center gap-1">
          {row.present && (
            <button
              onClick={isRevealed ? onHide : onReveal}
              className="p-1.5 text-muted-foreground hover:text-foreground"
              title={isRevealed ? "Hide" : "Reveal (recorded in the audit log)"}
              aria-label={isRevealed ? "Hide" : "Reveal"}
            >
              {isRevealed ? <EyeOff size={15} /> : <Eye size={15} />}
            </button>
          )}
          {row.present && (
            <button
              onClick={onCopy}
              className="p-1.5 text-muted-foreground hover:text-foreground"
              title="Copy to clipboard (any process can read the clipboard)"
              aria-label="Copy"
            >
              {copied ? <Check size={15} className="text-primary" /> : <Copy size={15} />}
            </button>
          )}
          {row.present && (
            <button
              onClick={onDelete}
              className="p-1.5 text-muted-foreground hover:text-destructive"
              title="Delete"
              aria-label="Delete"
            >
              <Trash2 size={15} />
            </button>
          )}
        </div>
      </div>
      {copied && (
        <div className="px-4 pb-2 font-mono text-[11px] text-muted-foreground">
          on the clipboard — any process on this machine can read it
        </div>
      )}
    </div>
  );
}
