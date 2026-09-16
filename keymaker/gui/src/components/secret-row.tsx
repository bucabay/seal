import { useState } from "react";
import { Eye, EyeOff, Copy, Check, Trash2, Save } from "lucide-react";
import { api, type RefRow } from "@/lib/api";
import { cn } from "@/lib/utils";

const MASK = "•".repeat(24);

/**
 * One reference. Revealing is deliberate and one row at a time: there is no
 * "show all", because the point of the product is that reading a value is an
 * act, not a default.
 */
export function SecretRow({
  row,
  onChanged,
}: {
  row: RefRow;
  onChanged: () => void;
}) {
  const [value, setValue] = useState<string | null>(null);
  const [draft, setDraft] = useState<string>("");
  const [copied, setCopied] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const revealed = value !== null;

  async function reveal() {
    setError(null);
    try {
      const v = await api.reveal(row.reference);
      setValue(v);
      setDraft(v);
    } catch (e) {
      setError(String(e));
    }
  }

  function hide() {
    setValue(null);
    setDraft("");
  }

  async function copy() {
    const v = value ?? (await api.reveal(row.reference).catch(() => null));
    if (v === null) return;
    await navigator.clipboard.writeText(v);
    setCopied(true);
    // The clipboard is readable by any process, so say so rather than let it
    // pass unnoticed.
    setTimeout(() => setCopied(false), 1600);
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      await api.save(row.reference, draft);
      setValue(draft);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    setBusy(true);
    setError(null);
    try {
      await api.remove(row.reference);
      setValue(null);
      onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const dirty = revealed && draft !== value;

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

        <input
          className={cn(
            "w-[38%] shrink-0 border border-line bg-background px-2 py-1 font-mono text-xs",
            "focus:border-primary focus:outline-none",
            !revealed && "cursor-pointer select-none text-muted-foreground",
          )}
          value={revealed ? draft : row.present ? MASK : ""}
          placeholder={row.present ? "" : "no value stored — type one"}
          readOnly={!revealed && row.present}
          onClick={() => !revealed && row.present && reveal()}
          onChange={(e) => {
            if (!revealed) setValue("");
            setDraft(e.target.value);
          }}
          aria-label={revealed ? `Value of ${row.reference}` : `${row.reference}, hidden`}
        />

        <div className="flex shrink-0 items-center gap-1">
          {row.present && (
            <button
              onClick={revealed ? hide : reveal}
              className="p-1.5 text-muted-foreground hover:text-foreground"
              title={revealed ? "Hide" : "Reveal (recorded in the audit log)"}
              aria-label={revealed ? "Hide" : "Reveal"}
            >
              {revealed ? <EyeOff size={15} /> : <Eye size={15} />}
            </button>
          )}
          {row.present && (
            <button
              onClick={copy}
              className="p-1.5 text-muted-foreground hover:text-foreground"
              title="Copy to clipboard (any process can read the clipboard)"
              aria-label="Copy"
            >
              {copied ? <Check size={15} className="text-primary" /> : <Copy size={15} />}
            </button>
          )}
          {(dirty || (!row.present && draft)) && (
            <button
              onClick={save}
              disabled={busy}
              className="p-1.5 text-primary hover:opacity-80 disabled:opacity-40"
              title="Save"
              aria-label="Save"
            >
              <Save size={15} />
            </button>
          )}
          {row.present && (
            <button
              onClick={remove}
              disabled={busy}
              className="p-1.5 text-muted-foreground hover:text-destructive disabled:opacity-40"
              title="Delete"
              aria-label="Delete"
            >
              <Trash2 size={15} />
            </button>
          )}
        </div>
      </div>
      {error && (
        <div className="px-4 pb-2 font-mono text-[11px] text-destructive">{error}</div>
      )}
      {copied && (
        <div className="px-4 pb-2 font-mono text-[11px] text-muted-foreground">
          on the clipboard — any process on this machine can read it
        </div>
      )}
    </div>
  );
}
