import { useCallback, useEffect, useState } from "react";
import {
  KeyRound,
  Play,
  Globe,
  ScrollText,
  ShieldQuestion,
  Check,
  X,
  ShieldCheck,
  ShieldAlert,
  Sun,
  Moon,
  RefreshCw,
  Lock,
  Search,
  Plus,
} from "lucide-react";
import {
  api,
  type ApprovalRow,
  type AuditRow,
  type EndpointRow,
  type Health,
  type RefRow,
  type RunResult,
  type TaskRow,
} from "@/lib/api";
import { cn, describeEvent, when } from "@/lib/utils";
import { SecretRow, type RowStatus } from "@/components/secret-row";
import { useAutosave } from "@/hooks/use-autosave";
import { AddSecret } from "@/components/add-secret";
import { grouped } from "@/lib/group";

type Tab = "secrets" | "approvals" | "tasks" | "endpoints" | "audit";

const TABS: { id: Tab; label: string; icon: typeof KeyRound }[] = [
  { id: "secrets", label: "Secrets", icon: KeyRound },
  { id: "approvals", label: "Approvals", icon: ShieldQuestion },
  { id: "tasks", label: "Tasks", icon: Play },
  { id: "endpoints", label: "Endpoints", icon: Globe },
  { id: "audit", label: "Audit", icon: ScrollText },
];

function Eyebrow({ children }: { children: React.ReactNode }) {
  return (
    <div className="font-mono text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
      {children}
    </div>
  );
}

function Panel({
  title,
  note,
  children,
}: {
  title: string;
  note?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="border border-line bg-card">
      <header className="flex items-baseline justify-between border-b border-line px-4 py-2.5">
        <h2 className="font-display text-sm font-600 text-foreground">{title}</h2>
        {note && <Eyebrow>{note}</Eyebrow>}
      </header>
      {children}
    </section>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="px-4 py-8 text-center font-mono text-xs text-muted-foreground">
      {children}
    </div>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("secrets");
  const [dark, setDark] = useState(true);
  const [refs, setRefs] = useState<RefRow[]>([]);
  const [tasks, setTasks] = useState<TaskRow[]>([]);
  const [endpoints, setEndpoints] = useState<EndpointRow[]>([]);
  const [audit, setAudit] = useState<AuditRow[]>([]);
  const [approvals, setApprovals] = useState<ApprovalRow[]>([]);
  const [envs, setEnvs] = useState<string[]>([]);
  const [env, setEnv] = useState("default");
  const [health, setHealth] = useState<Health | null>(null);
  const [running, setRunning] = useState<string | null>(null);
  /** Revealed values, by reference. Membership is what "revealed" means. */
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  /** Edits not yet written. */
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [copied, setCopied] = useState<string | null>(null);
  /** What is typed in the search box. Empty means the whole inventory. */
  const [query, setQuery] = useState("");
  /** Which group's inline add form is open, if any. */
  const [addingTo, setAddingTo] = useState<string | null>(null);
  const [result, setResult] = useState<(RunResult & { task: string }) | null>(null);
  const [error, setError] = useState<string | null>(null);

  // An empty value is a half-finished edit, not an instruction to store
  // nothing — the backend refuses it anyway, so never send one.
  const commit = useCallback(async (reference: string, value: string) => {
    try {
      await api.save(reference, value);
    } catch (e) {
      setError(String(e));
      throw e;
    }
    // Keep the value on screen. It is already there — the user just typed it —
    // and clearing the field after a successful save reads as losing the edit.
    setRevealed((r) => ({ ...r, [reference]: value }));
    setDrafts((d) => {
      // Only retire the draft if it is still the text that was just written.
      // Typing carries on while a write is in the air, and discarding the
      // newer text would snap the field back to the older value — which then
      // becomes what the next save stores.
      if (d[reference] !== value) return d;
      const { [reference]: _, ...rest } = d;
      return rest;
    });
    // A reference that had nothing behind it now does, so drop the badge
    // without waiting for a reload.
    setRefs((rows) =>
      rows.map((row) =>
        row.reference === reference && !row.present ? { ...row, present: true } : row,
      ),
    );
  }, []);

  const { state: saveState, schedule, flush, flushAll } = useAutosave<string>(commit);

  const refresh = useCallback(async () => {
    setError(null);
    try {
      const [r, t, e, a, v, h, p] = await Promise.all([
        api.refs(),
        api.tasks(),
        api.endpoints(),
        api.audit(),
        api.environments(),
        api.health(),
        api.approvals(),
      ]);
      setRefs(r);
      setTasks(t);
      setEndpoints(e);
      setAudit(a);
      setEnvs(v);
      setHealth(h);
      setApprovals(p);
      if (v.length > 0 && !v.includes(env)) setEnv(v[0]);
    } catch (err) {
      setError(String(err));
    }
  }, [env]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", dark);
  }, [dark]);

  // A pending edit must not be lost to a window switch, a tab change, or the
  // app closing. Each of these is a moment the user considers the edit done.
  useEffect(() => {
    window.addEventListener("blur", flushAll);
    window.addEventListener("beforeunload", flushAll);
    return () => {
      window.removeEventListener("blur", flushAll);
      window.removeEventListener("beforeunload", flushAll);
    };
  }, [flushAll]);

  useEffect(() => () => flushAll(), [tab, flushAll]);

  // An agent blocked on a decision is waiting on a person, so poll rather than
  // make them hit refresh. Cheap: it reads one small file.
  useEffect(() => {
    const t = setInterval(() => {
      api.approvals().then(setApprovals).catch(() => {});
    }, 2000);
    return () => clearInterval(t);
  }, []);

  async function decide(id: string, granted: boolean) {
    try {
      await api.decide(id, granted);
      await refresh();
    } catch (err) {
      setError(String(err));
    }
  }

  async function reveal(reference: string) {
    try {
      const value = await api.reveal(reference);
      setRevealed((r) => ({ ...r, [reference]: value }));
    } catch (err) {
      setError(String(err));
    }
  }

  function hide(reference: string) {
    // Flush first: hiding a row with an unsaved edit would discard it.
    flush(reference);
    setRevealed((r) => {
      const { [reference]: _, ...rest } = r;
      return rest;
    });
  }

  function edit(reference: string, value: string) {
    setDrafts((d) => ({ ...d, [reference]: value }));
    // An empty field is a half-finished edit; the row says "empty" and nothing
    // is written until there is something to write.
    if (value !== "") schedule(reference, value);
  }

  async function copy(reference: string) {
    try {
      const value = revealed[reference] ?? (await api.reveal(reference));
      await navigator.clipboard.writeText(value);
      setCopied(reference);
      setTimeout(() => setCopied((c) => (c === reference ? null : c)), 1600);
    } catch (err) {
      setError(String(err));
    }
  }

  async function remove(reference: string) {
    try {
      await api.remove(reference);
      hide(reference);
      await refresh();
    } catch (err) {
      setError(String(err));
    }
  }

  // Groups are derived from the names, so there is nothing to keep in step.
  const groups = grouped(refs, query);

  /** An empty draft is reported as such rather than as an unsaved write. */
  const statusOf = (reference: string): RowStatus | undefined =>
    drafts[reference] === "" ? "empty" : saveState[reference];

  async function run(task: string) {
    setRunning(task);
    setResult(null);
    try {
      const out = await api.runTask(task, env);
      setResult({ ...out, task });
      void refresh();
    } catch (err) {
      setError(String(err));
    } finally {
      setRunning(null);
    }
  }

  return (
    <div className="min-h-screen bg-background font-sans text-foreground">
      <header className="flex items-center gap-4 border-b border-line px-5 py-3">
        <div className="flex items-center gap-2">
          <Lock size={16} className="text-primary" />
          <span className="font-display text-base font-700 tracking-tight">keymaker</span>
        </div>

        <div className="flex-1" />

        {health && (
          <div
            className="flex items-center gap-1.5 border border-line px-2 py-1"
            title={health.enforcement}
          >
            {health.enforced ? (
              <ShieldCheck size={13} className="text-primary" />
            ) : (
              <ShieldAlert size={13} className="text-destructive" />
            )}
            <span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
              {health.enforced ? "jail enforced" : "jail not enforced"}
            </span>
          </div>
        )}

        <button
          onClick={() => void refresh()}
          className="p-1.5 text-muted-foreground hover:text-foreground"
          aria-label="Refresh"
          title="Refresh"
        >
          <RefreshCw size={15} />
        </button>
        <button
          onClick={() => setDark((d) => !d)}
          className="p-1.5 text-muted-foreground hover:text-foreground"
          aria-label="Toggle theme"
        >
          {dark ? <Sun size={15} /> : <Moon size={15} />}
        </button>
      </header>

      <nav className="flex gap-0 border-b border-line px-5">
        {TABS.map(({ id, label, icon: Icon }) => (
          <button
            key={id}
            onClick={() => setTab(id)}
            className={cn(
              "flex items-center gap-1.5 border-b-2 px-3 py-2.5 font-mono text-[11px] uppercase tracking-wider",
              tab === id
                ? "border-primary text-foreground"
                : "border-transparent text-muted-foreground hover:text-foreground",
            )}
          >
            <Icon size={13} />
            {label}
            {id === "approvals" && approvals.length > 0 && (
              <span className="ml-0.5 bg-primary px-1 text-[10px] font-600 text-primary-foreground">
                {approvals.length}
              </span>
            )}
          </button>
        ))}
      </nav>

      <main className="mx-auto max-w-5xl space-y-5 p-5">
        {error && (
          <div className="border border-destructive/40 bg-destructive/5 px-4 py-2.5 font-mono text-xs text-destructive">
            {error}
          </div>
        )}

        {tab === "secrets" && (
          <>
            <Panel
              title="Secrets"
              note={`${refs.filter((r) => r.present).length} of ${refs.length} stored here`}
            >
              {/* Search is the interface: the inventory is small enough that
                  typing beats navigating, and an empty box is the full list. */}
              <div className="flex items-center gap-2 border-b border-line px-4 py-2.5">
                <Search size={14} className="shrink-0 text-muted-foreground" />
                <input
                  autoFocus
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  onKeyDown={(e) => e.key === "Escape" && setQuery("")}
                  placeholder="filter by reference or project…"
                  className="min-w-0 flex-1 bg-transparent font-mono text-xs focus:outline-none"
                  aria-label="Filter secrets"
                />
                {query && (
                  <button
                    onClick={() => setQuery("")}
                    className="shrink-0 font-mono text-[10px] uppercase tracking-wider text-muted-foreground hover:text-foreground"
                  >
                    clear
                  </button>
                )}
              </div>

              <AddSecret onAdded={() => void refresh()} />

              {groups.length === 0 ? (
                <Empty>
                  {refs.length === 0
                    ? "Nothing stored yet. Add a secret above — name it issuer/key, like stripe/api-key, and the group appears on its own."
                    : `Nothing matches “${query}”.`}
                </Empty>
              ) : (
                groups.map((group) => (
                  <div key={group.issuer}>
                    <div className="sticky top-0 z-10 flex items-center gap-2 border-b border-line bg-secondary/60 px-4 py-1.5 backdrop-blur">
                      <span className="font-mono text-[11px] uppercase tracking-wider text-foreground">
                        {group.issuer}
                      </span>
                      <span className="font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                        {group.stored} of {group.rows.length}
                      </span>
                      <div className="flex-1" />
                      <button
                        onClick={() =>
                          setAddingTo(addingTo === group.issuer ? null : group.issuer)
                        }
                        className="text-muted-foreground hover:text-primary"
                        title={`Add a key under ${group.issuer}`}
                        aria-label={`Add a key under ${group.issuer}`}
                      >
                        <Plus size={13} />
                      </button>
                    </div>

                    {addingTo === group.issuer && (
                      <AddSecret
                        prefill={`${group.issuer}/`}
                        onAdded={() => {
                          setAddingTo(null);
                          void refresh();
                        }}
                        onCancel={() => setAddingTo(null)}
                      />
                    )}

                    <div className="border-l border-line ml-4">
                    {group.rows.map((r) => (
                      <SecretRow
                        key={r.reference}
                        row={r}
                        revealed={r.reference in revealed ? revealed[r.reference] : null}
                        draft={drafts[r.reference]}
                        status={statusOf(r.reference)}
                        copied={copied === r.reference}
                        onReveal={() => void reveal(r.reference)}
                        onHide={() => hide(r.reference)}
                        onChange={(v) => edit(r.reference, v)}
                        onFlush={() => flush(r.reference)}
                        onCopy={() => void copy(r.reference)}
                        onDelete={() => void remove(r.reference)}
                      />
                    ))}
                    </div>
                  </div>
                ))
              )}
            </Panel>
            <p className="font-mono text-[11px] leading-relaxed text-muted-foreground">
              Name a key <span className="text-foreground">issuer/key</span> and it files
              itself — there are no groups to create. Edits save themselves a moment after
              you stop typing, and immediately when you leave the field. This window is the only place a value can be read —
              the CLI and the MCP surface have no such command, which is the point of
              them. Every reveal is written to the audit log.
            </p>
          </>
        )}

        {tab === "approvals" && (
          <>
            <Panel
              title="Waiting for a decision"
              note={approvals.length === 0 ? "nothing waiting" : `${approvals.length} pending`}
            >
              {approvals.length === 0 ? (
                <Empty>
                  When a policy stops an agent, the request appears here for you to
                  answer. Nothing is waiting.
                </Empty>
              ) : (
                approvals.map((a) => (
                  <div key={a.id} className="border-b border-line px-4 py-3 last:border-b-0">
                    <div className="flex items-start gap-3">
                      <div className="min-w-0 flex-1">
                        <div className="font-mono text-sm">{a.capability}</div>
                        <div className="mt-0.5 font-mono text-[11px] text-muted-foreground">
                          stopped by: {a.rule}
                        </div>
                        {a.detail && (
                          <div className="mt-1 border border-line bg-background px-2 py-1 font-mono text-[11px]">
                            {a.detail}
                          </div>
                        )}
                        <div className="mt-1 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                          lapses in {a.seconds_left}s
                        </div>
                      </div>
                      <div className="flex shrink-0 gap-2">
                        <button
                          onClick={() => void decide(a.id, true)}
                          className="flex items-center gap-1 border border-primary px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-primary hover:bg-primary hover:text-primary-foreground"
                        >
                          <Check size={12} />
                          approve
                        </button>
                        <button
                          onClick={() => void decide(a.id, false)}
                          className="flex items-center gap-1 border border-line px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider text-muted-foreground hover:border-destructive hover:text-destructive"
                        >
                          <X size={12} />
                          deny
                        </button>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </Panel>
            <p className="font-mono text-[11px] leading-relaxed text-muted-foreground">
              One approval authorises one call, and lapses if nobody answers. An agent
              cannot approve its own request — the MCP tool for it grants nothing.
            </p>
          </>
        )}

        {tab === "tasks" && (
          <>
            <div className="flex items-center gap-2">
              <Eyebrow>Environment</Eyebrow>
              <select
                value={env}
                onChange={(e) => setEnv(e.target.value)}
                className="border border-line bg-background px-2 py-1 font-mono text-xs focus:border-primary focus:outline-none"
              >
                {(envs.length ? envs : ["default"]).map((e) => (
                  <option key={e}>{e}</option>
                ))}
              </select>
            </div>

            <Panel title="Tasks" note="from .keymaker">
              {tasks.length === 0 ? (
                <Empty>No tasks declared. Add a [tasks] section to .keymaker.</Empty>
              ) : (
                tasks.map((t) => (
                  <div
                    key={t.name}
                    className="flex items-center gap-3 border-b border-line px-4 py-3 last:border-b-0"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="font-mono text-sm">{t.name}</div>
                      <div className="truncate font-mono text-[11px] text-muted-foreground">
                        {t.command}
                      </div>
                    </div>
                    <button
                      onClick={() => void run(t.name)}
                      disabled={running !== null}
                      className="flex items-center gap-1.5 border border-line px-2.5 py-1 font-mono text-[11px] uppercase tracking-wider hover:border-primary hover:text-primary disabled:opacity-40"
                    >
                      <Play size={12} />
                      {running === t.name ? "running" : "run"}
                    </button>
                  </div>
                ))
              )}
            </Panel>

            {result && (
              <Panel
                title={`Output — ${result.task}`}
                note={`exit ${result.exit_code ?? "?"}`}
              >
                <div className="space-y-2 px-4 py-3">
                  {result.redacted && (
                    <div className="border border-primary/40 bg-primary/5 px-3 py-1.5 font-mono text-[11px] text-primary">
                      This task printed a secret value. It has been masked.
                    </div>
                  )}
                  {result.leaked_files.map((f) => (
                    <div
                      key={f}
                      className="border border-destructive/40 bg-destructive/5 px-3 py-1.5 font-mono text-[11px] text-destructive"
                    >
                      This task wrote a credential to {f}
                    </div>
                  ))}
                  {result.stdout && (
                    <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs text-foreground">
                      {result.stdout}
                    </pre>
                  )}
                  {result.stderr && (
                    <pre className="overflow-x-auto whitespace-pre-wrap font-mono text-xs text-muted-foreground">
                      {result.stderr}
                    </pre>
                  )}
                </div>
              </Panel>
            )}
          </>
        )}

        {tab === "endpoints" && (
          <Panel title="Endpoints" note={`${endpoints.length} defined`}>
            {endpoints.length === 0 ? (
              <Empty>
                No endpoint definitions found. Drop one in .keymaker.endpoints.toml.
              </Empty>
            ) : (
              endpoints.map((e) => (
                <div
                  key={e.name}
                  className="flex items-center gap-3 border-b border-line px-4 py-3 last:border-b-0"
                >
                  <span className="w-14 shrink-0 font-mono text-[10px] uppercase tracking-wider text-muted-foreground">
                    {e.method}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="font-mono text-sm">{e.name}</div>
                    <div className="truncate font-mono text-[11px] text-muted-foreground">
                      {e.host}
                      {e.path}
                    </div>
                  </div>
                  <span className="shrink-0 font-mono text-[11px] text-muted-foreground">
                    {e.secret}
                  </span>
                  {e.gated && (
                    <span
                      className="shrink-0 border border-primary/50 px-1.5 py-0.5 font-mono text-[10px] uppercase tracking-wider text-primary"
                      title="Policy stops this for a person"
                    >
                      needs approval
                    </span>
                  )}
                </div>
              ))
            )}
          </Panel>
        )}

        {tab === "audit" && (
          <Panel
            title="Audit"
            note={
              health
                ? health.audit_intact
                  ? `${health.audit_entries} entries · chain intact`
                  : "CHAIN BROKEN"
                : undefined
            }
          >
            {health && !health.audit_intact && (
              <div className="border-b border-destructive/40 bg-destructive/5 px-4 py-2 font-mono text-[11px] text-destructive">
                The audit chain does not verify. An entry has been edited or removed.
              </div>
            )}
            {audit.length === 0 ? (
              <Empty>Nothing recorded yet.</Empty>
            ) : (
              [...audit].reverse().map((row) => {
                const { kind, detail } = describeEvent(row.summary);
                return (
                  <div
                    key={row.seq}
                    className="flex gap-3 border-b border-line px-4 py-2 last:border-b-0"
                  >
                    <span className="w-10 shrink-0 text-right font-mono text-[11px] text-muted-foreground">
                      {row.seq}
                    </span>
                    <span className="w-36 shrink-0 font-mono text-[11px] text-muted-foreground">
                      {when(row.at)}
                    </span>
                    <span className="w-40 shrink-0 font-mono text-[11px] text-foreground">
                      {kind}
                    </span>
                    <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-muted-foreground">
                      {detail}
                    </span>
                  </div>
                );
              })
            )}
          </Panel>
        )}
      </main>
    </div>
  );
}
