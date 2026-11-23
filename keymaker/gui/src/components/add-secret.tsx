import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Plus } from "lucide-react";
import { rawInput } from "@/lib/raw-input";
import { splitPasted } from "@/lib/group";
import { actionFor, nextPrefix, referenceProblem, shouldPreventDefault } from "@/lib/entry";

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
  onCommit,
  focusRequest = 0,
  onCancel,
}: {
  /** e.g. `stripe/`, when adding from inside a group. */
  prefill?: string;
  onCommit: (reference: string, value: string) => void;
  focusRequest?: number;
  onCancel?: () => void;
}) {
  const [open, setOpen] = useState(prefill !== undefined);
  const [reference, setReference] = useState(prefill ?? "");
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [entrySequence, setEntrySequence] = useState(0);
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
  useLayoutEffect(() => {
    if (open) focusReference();
  }, [open, focusRequest, entrySequence]);

  /** Enter and Tab mean the same thing; which one depends only on the field. */
  function onKey(e: React.KeyboardEvent, field: "name" | "value") {
    const key = { ...e, isComposing: e.nativeEvent.isComposing };
    if (shouldPreventDefault(key, field)) e.preventDefault();
    switch (actionFor(key, field)) {
      case "next":
        valueField.current?.focus();
        break;
      case "commit":
        commit();
        break;
      case "cancel":
        close();
        break;
    }
  }

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
    onCancel?.();
  }

  /**
   * Save what is typed and be ready for the next key straight away.
   *
   * The fields clear before the write finishes, on purpose: entering secrets is
   * a run, and waiting on the keychain between each one would make it feel
   * like a form rather than a list. The parent retains the submitted entry and
   * its save status independently of this next-entry form.
   */
  function commit() {
    setError(null);
    const ref = reference.trim();
    const problem = referenceProblem(ref);
    if (problem) {
      setError(problem);
      focusReference();
      return;
    }
    if (!value) {
      setError("a value is needed");
      valueField.current?.focus();
      return;
    }

    // A prefix is a convenience, not a constraint: whatever issuer was actually
    // typed is where this lands, and where the next one starts.
    const captured = value;
    setReference(nextPrefix(ref, prefill));
    setValue("");
    setEntrySequence((sequence) => sequence + 1);
    onCommit(ref, captured);
  }

  function submit(e: React.FormEvent) {
    e.preventDefault();
    commit();
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
          onKeyDown={(e) => onKey(e, "name")}
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
          onKeyDown={(e) => onKey(e, "value")}
          placeholder="value — enter or tab saves"
          className="w-[34%] shrink-0 border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
          aria-label="Value"
        />
        <button
          type="submit"
          className="shrink-0 border border-primary px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-primary hover:bg-primary hover:text-primary-foreground disabled:opacity-40"
        >
          save
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
      ) : null}
    </form>
  );
}
