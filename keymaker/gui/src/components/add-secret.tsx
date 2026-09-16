import { useEffect, useRef, useState } from "react";
import { Plus, Check } from "lucide-react";
import { api } from "@/lib/api";
import { rawInput } from "@/lib/raw-input";
import { splitPasted } from "@/lib/group";

/**
 * Adding secrets, one after another.
 *
 * Saving does not close the form. Adding keys is something people do in runs —
 * pasting a handful out of a provider's dashboard — so Enter stores this one
 * and leaves the cursor ready for the next. The saved key drops into its group
 * behind the form as soon as it lands.
 *
 * One paste fills both fields: `stripe/api-key sk_xxx` splits, and the value
 * goes straight to the masked field rather than sitting in plain text.
 */
export function AddSecret({
  prefill,
  onAdded,
  onCancel,
}: {
  /** e.g. `stripe/`, when adding from inside a group. */
  prefill?: string;
  onAdded: () => void;
  onCancel?: () => void;
}) {
  const [open, setOpen] = useState(prefill !== undefined);
  const [reference, setReference] = useState(prefill ?? "");
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [justAdded, setJustAdded] = useState<string | null>(null);
  const referenceField = useRef<HTMLInputElement>(null);
  const valueField = useRef<HTMLInputElement>(null);

  /** Put the cursor after the prefix, ready for the key name. */
  function focusReference() {
    const field = referenceField.current;
    if (!field) return;
    field.focus();
    const end = field.value.length;
    field.setSelectionRange(end, end);
  }

  useEffect(() => {
    if (prefill !== undefined) {
      setOpen(true);
      setReference(prefill);
    }
  }, [prefill]);

  // Whenever the form opens, the cursor belongs in the reference field —
  // after `stripe/` when adding inside a group, not in the value.
  useEffect(() => {
    if (open) focusReference();
  }, [open]);

  function takeReference(text: string) {
    const { reference: ref, value: pasted } = splitPasted(text);
    setReference(ref);
    if (pasted !== undefined) {
      setValue(pasted);
      valueField.current?.focus();
    }
  }

  function close() {
    setOpen(false);
    setReference("");
    setValue("");
    setError(null);
    setJustAdded(null);
    onCancel?.();
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const ref = reference.trim();
    if (!ref) {
      setError("a reference is needed, for example stripe/api-key");
      focusReference();
      return;
    }
    if (!value) {
      setError("a value is needed");
      valueField.current?.focus();
      return;
    }
    setBusy(true);
    try {
      await api.save(ref, value);
      // Stay open and reset for the next one. Back to the prefix when adding
      // inside a group, since the next key is probably the same issuer.
      setReference(prefill ?? "");
      setValue("");
      setJustAdded(ref);
      focusReference();
      onAdded();
    } catch (err) {
      setError(String(err).replace(/^.*?constraint violation: /, ""));
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
    <form onSubmit={submit} className="border-b border-line bg-secondary/40 px-4 py-3">
      <div className="flex items-center gap-2">
        <input
          {...rawInput}
          ref={referenceField}
          value={reference}
          onChange={(e) => takeReference(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && close()}
          placeholder="stripe/api-key — or paste `stripe/api-key sk_xxx`"
          className="min-w-0 flex-1 border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
          aria-label="Reference"
        />
        <input
          {...rawInput}
          ref={valueField}
          type="password"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && close()}
          placeholder="value — enter to save"
          className="w-[34%] shrink-0 border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
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
          onClick={close}
          className="shrink-0 px-1 font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:text-foreground"
        >
          done
        </button>
      </div>
      {error ? (
        <div className="mt-2 font-mono text-[11px] text-destructive">{error}</div>
      ) : justAdded ? (
        <div className="mt-2 flex items-center gap-1.5 font-mono text-[11px] text-primary">
          <Check size={12} />
          added {justAdded} — next one, or `done` to finish
        </div>
      ) : null}
    </form>
  );
}
