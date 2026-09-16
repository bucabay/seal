import { useEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { api } from "@/lib/api";
import { rawInput } from "@/lib/raw-input";
import { splitPasted } from "@/lib/group";

/**
 * Adding a secret.
 *
 * Two fields, but one paste fills both: putting `stripe/api-key sk_xxx` into
 * the reference box splits it, and the value lands in the masked field rather
 * than sitting in plain text where it was typed.
 *
 * There is no group to choose. The issuer is the part before the first slash,
 * so naming a key `stripe/anything` files it under Stripe and the group appears
 * on its own.
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
  const valueField = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (prefill !== undefined) {
      setOpen(true);
      setReference(prefill);
    }
  }, [prefill]);

  /** Split a pasted `reference value` so the value never sits in a plain field. */
  function takeReference(text: string) {
    const { reference: ref, value: pasted } = splitPasted(text);
    setReference(ref);
    if (pasted !== undefined) {
      setValue(pasted);
      // Move on, so the next keystroke does not land back in the reference.
      valueField.current?.focus();
    }
  }

  function close() {
    setOpen(false);
    setReference("");
    setValue("");
    setError(null);
    onCancel?.();
  }

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const ref = reference.trim();
    if (!ref) {
      setError("a reference is needed, for example stripe/api-key");
      return;
    }
    if (!value) {
      setError("a value is needed");
      return;
    }
    setBusy(true);
    try {
      await api.save(ref, value);
      close();
      onAdded();
    } catch (err) {
      // The backend does the real validation; show what it said.
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
          autoFocus={prefill === undefined || prefill === ""}
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
          autoFocus={!!prefill}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Escape" && close()}
          placeholder="value"
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
          cancel
        </button>
      </div>
      {error && <div className="mt-2 font-mono text-[11px] text-destructive">{error}</div>}
    </form>
  );
}
