import { useState } from "react";
import { Plus } from "lucide-react";
import { api } from "@/lib/api";
import { rawInput } from "@/lib/raw-input";

/**
 * Adding a secret from the GUI.
 *
 * The value field is a password input, so it is not on screen while being
 * typed or afterwards — the same reason the CLI turns off terminal echo.
 */
export function AddSecret({ onAdded }: { onAdded: () => void }) {
  const [open, setOpen] = useState(false);
  const [reference, setReference] = useState("");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const ref = reference.trim();
    if (!ref) {
      setError("a reference is needed, e.g. stripe/sk_live");
      return;
    }
    if (!value) {
      setError("a value is needed");
      return;
    }
    setBusy(true);
    try {
      await api.save(ref, value);
      setReference("");
      setValue("");
      setOpen(false);
      onAdded();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  }

  if (!open) {
    return (
      <div className="border-b border-line px-4 py-2.5">
        <button
          onClick={() => setOpen(true)}
          className="flex items-center gap-1.5 font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-primary"
        >
          <Plus size={13} />
          add a secret
        </button>
      </div>
    );
  }

  return (
    <form onSubmit={submit} className="border-b border-line px-4 py-3">
      <div className="flex items-center gap-2">
        <input
          {...rawInput}
          autoFocus
          value={reference}
          onChange={(e) => setReference(e.target.value)}
          placeholder="reference, e.g. stripe/sk_live"
          className="min-w-0 flex-1 border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
          aria-label="Reference"
        />
        <input
          {...rawInput}
          type="password"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="value"
          className="w-[38%] shrink-0 border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
          aria-label="Value"
        />
        <button
          type="submit"
          disabled={busy}
          className="shrink-0 border border-primary px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-primary hover:bg-primary hover:text-primary-foreground disabled:opacity-40"
        >
          {busy ? "saving" : "save"}
        </button>
        <button
          type="button"
          onClick={() => {
            setOpen(false);
            setError(null);
            setValue("");
          }}
          className="shrink-0 px-1 font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground"
        >
          cancel
        </button>
      </div>
      {error && (
        <div className="mt-2 font-mono text-[11px] text-destructive">{error}</div>
      )}
      <div className="mt-2 font-mono text-[10px] leading-relaxed text-muted-foreground">
        A reference is a name, not a value — <span className="text-foreground">project/key</span> by
        convention. Your manifest and endpoint definitions refer to it by that name, and the value
        stays in the OS keychain.
      </div>
    </form>
  );
}
