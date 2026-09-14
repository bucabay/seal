import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KeyRound, Plus } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Header } from "@/components/header";
import { LoginDialog } from "@/components/login-dialog";
import { AddVaultDialog } from "@/components/add-vault-dialog";
import { AddEnvDialog } from "@/components/add-env-dialog";
import { SecretRow, type RowStatus } from "@/components/secret-row";
import type { EnvEntry } from "@/components/env-selector";
import { useAutosave } from "@/hooks/use-autosave";
import { useUser } from "@/hooks/use-user";

interface Secret {
  key: string;
  vault: string;
  /** Environment the value actually lives in. */
  env: string;
  /** True when the value comes from an environment this one extends. */
  inherited: boolean;
}

/**
 * Everything a queued write needs, captured when the edit is typed.
 *
 * The destination travels with the edit rather than being read at write time,
 * so switching vault or environment mid-debounce cannot redirect a save.
 */
interface Write {
  value: string;
  vault: string;
  env: string;
  /** Whether this write creates an override rather than updating in place. */
  inherited: boolean;
}

function omit<T>(record: Record<string, T>, key: string): Record<string, T> {
  const { [key]: _, ...rest } = record;
  return rest;
}

function App() {
  const [vaults, setVaults] = useState<string[]>([]);
  const [vault, setVault] = useState("seal");
  const [envs, setEnvs] = useState<EnvEntry[]>([]);
  const [env, setEnv] = useState("default");
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  /** Revealed values as they are stored. Membership is what "revealed" means. */
  const [stored, setStored] = useState<Record<string, string>>({});
  /** Local edits not yet written. Absent means the row matches `stored`. */
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [toast, setToast] = useState<string | null>(null);
  const [loginOpen, setLoginOpen] = useState(false);
  const [addVaultOpen, setAddVaultOpen] = useState(false);
  const [addEnvOpen, setAddEnvOpen] = useState(false);

  const { user, signIn, signOut } = useUser();

  const showToast = useCallback((msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2000);
  }, []);

  const loadVaults = useCallback(async () => {
    try {
      const list: string[] = await invoke("list_vaults");
      setVaults(list);
    } catch (e) {
      console.error(e);
    }
  }, []);

  const loadEnvs = useCallback(async () => {
    try {
      const list: EnvEntry[] = await invoke("list_envs", { vault });
      setEnvs(list);
      // Switching vaults can land on an environment the new one does not have.
      setEnv((current) =>
        list.some((e) => e.name === current) ? current : "default",
      );
    } catch (e) {
      console.error(e);
    }
  }, [vault]);

  const loadSecrets = useCallback(async () => {
    try {
      const list: Secret[] = await invoke("list_secrets", { vault, env });
      setSecrets(list);
    } catch (e) {
      console.error(e);
    }
  }, [vault, env]);

  // Write one edited row. Rejecting is how the row learns the value on screen
  // is not the value stored, so the error is reported and then re-thrown.
  const commit = useCallback(
    async (key: string, write: Write) => {
      try {
        await invoke("save_secret", {
          key,
          value: write.value,
          vault: write.vault,
          env: write.env,
        });
      } catch (e) {
        showToast(`Error: ${e}`);
        throw e;
      }
      // Only refresh a row the user is still looking at; re-adding a key that
      // has since been hidden would silently re-reveal it.
      setStored((s) => (key in s ? { ...s, [key]: write.value } : s));
      setDrafts((d) => omit(d, key));
      if (write.inherited) {
        // The value is now this environment's own: the badge and the counts
        // both have to follow.
        showToast(`Overrode ${key} in ${write.env}`);
        loadSecrets();
        loadEnvs();
      }
    },
    [loadEnvs, loadSecrets, showToast],
  );

  const {
    state: saveState,
    schedule,
    flush,
    flushAll,
    cancel,
  } = useAutosave<Write>(commit);

  useEffect(() => {
    loadVaults();
  }, [loadVaults]);

  useEffect(() => {
    loadEnvs();
  }, [loadEnvs]);

  useEffect(() => {
    loadSecrets();
  }, [loadSecrets]);

  // Edits belong to the vault and environment they were typed in, so they are
  // written out before the list they came from is replaced.
  useEffect(() => {
    return () => flushAll();
  }, [vault, env, flushAll]);

  useEffect(() => {
    setStored({});
    setDrafts({});
  }, [vault, env]);

  // Clicking away from the window is as good a signal as clicking away from
  // the field: do not sit on an unwritten edit while the app is in the
  // background.
  useEffect(() => {
    window.addEventListener("blur", flushAll);
    return () => window.removeEventListener("blur", flushAll);
  }, [flushAll]);

  const handleSave = async () => {
    if (!newKey.trim() || !newValue.trim()) return;
    try {
      await invoke("save_secret", {
        key: newKey.trim(),
        value: newValue,
        vault,
        env,
      });
      setNewKey("");
      setNewValue("");
      showToast("Saved");
      loadSecrets();
      loadEnvs();
      loadVaults();
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleReveal = async (key: string) => {
    if (key in stored) {
      // Hiding is not a reason to drop an edit.
      flush(key);
      setStored((s) => omit(s, key));
      setDrafts((d) => omit(d, key));
      return;
    }
    try {
      const value: string = await invoke("get_secret", { key, vault, env });
      setStored((s) => ({ ...s, [key]: value }));
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleEdit = (secret: Secret, value: string) => {
    setDrafts((d) => ({ ...d, [secret.key]: value }));
    // An empty value is a half-finished edit, not an instruction to store an
    // empty secret. It is held, marked, and never written.
    if (value === "") {
      cancel(secret.key);
      return;
    }
    schedule(secret.key, {
      value,
      vault,
      env,
      inherited: secret.inherited,
    });
  };

  const handleRevert = (key: string) => {
    cancel(key);
    setDrafts((d) => omit(d, key));
  };

  const handleCopy = async (key: string) => {
    try {
      // What is on screen wins, so copying an edit does not hand back the
      // value it replaced.
      const value =
        drafts[key] ??
        stored[key] ??
        (await invoke<string>("get_secret", { key, vault, env }));
      await navigator.clipboard.writeText(value);
      showToast("Copied");
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleDelete = async (key: string) => {
    cancel(key);
    try {
      await invoke("delete_secret", { key, vault, env });
      setStored((s) => omit(s, key));
      setDrafts((d) => omit(d, key));
      loadSecrets();
      loadEnvs();
      showToast("Deleted");
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleAddVault = async (name: string) => {
    try {
      await invoke("add_vault", { vault: name });
      await loadVaults();
      setVault(name);
      showToast(`Added vault "${name}"`);
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleAddEnv = async (name: string, extends_: string) => {
    try {
      await invoke("add_env", { vault, name, extends: extends_ });
      await loadEnvs();
      setEnv(name);
      showToast(`Added environment "${name}"`);
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleDeleteEnv = async (name: string) => {
    try {
      await invoke("delete_env", { vault, name });
      if (env === name) setEnv("default");
      await loadEnvs();
      showToast(`Removed environment "${name}"`);
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleSignIn = (name: string, email: string) => {
    signIn({ name, email });
    showToast(`Signed in as ${name}`);
  };

  const statusOf = (key: string): RowStatus | undefined =>
    drafts[key] === "" ? "empty" : saveState[key];

  const inheritedCount = secrets.filter((s) => s.inherited).length;

  return (
    <div className="flex min-h-screen flex-col bg-background text-ink">
      <Header
        vaults={vaults}
        current={vault}
        envs={envs}
        currentEnv={env}
        user={user}
        onSelectVault={setVault}
        onAddVault={() => setAddVaultOpen(true)}
        onSelectEnv={setEnv}
        onAddEnv={() => setAddEnvOpen(true)}
        onDeleteEnv={handleDeleteEnv}
        onSignIn={() => setLoginOpen(true)}
        onSignOut={() => {
          signOut();
          showToast("Signed out");
        }}
      />

      <main className="frame flex-1">
        {/* Add secret */}
        <form
          onSubmit={(e) => {
            e.preventDefault();
            handleSave();
          }}
          className="flex items-center gap-2 border-b border-line px-6 py-4"
        >
          <KeyRound className="h-4 w-4 shrink-0 text-muted-foreground" />
          <Input
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            placeholder="key"
            className="font-mono"
          />
          <Input
            value={newValue}
            onChange={(e) => setNewValue(e.target.value)}
            placeholder="value"
            className="font-mono"
          />
          <Button type="submit" className="shrink-0 gap-1.5">
            <Plus className="h-4 w-4" />
            Save
          </Button>
        </form>

        {/* Eyebrow */}
        <div className="flex items-center justify-between px-6 py-2.5">
          <span className="eyebrow">Secrets</span>
          <span className="font-mono text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
            {secrets.length} keys
            {inheritedCount > 0 && ` · ${inheritedCount} inherited`}
          </span>
        </div>

        {secrets.length === 0 ? (
          <div className="px-6 py-16 text-center">
            <div className="font-display text-lg text-ink">No secrets yet</div>
            <p className="mt-1 font-mono text-xs text-muted-foreground">
              Add one above, or switch vaults.
            </p>
          </div>
        ) : (
          <div className="border-t border-line">
            {secrets.map((s) => (
              <SecretRow
                key={s.key}
                name={s.key}
                env={s.env}
                inherited={s.inherited}
                value={
                  s.key in stored ? (drafts[s.key] ?? stored[s.key]) : null
                }
                status={statusOf(s.key)}
                onReveal={() => handleReveal(s.key)}
                onEdit={(value) => handleEdit(s, value)}
                onFlush={() => flush(s.key)}
                onRevert={() => handleRevert(s.key)}
                onCopy={() => handleCopy(s.key)}
                onDelete={() =>
                  s.inherited
                    ? showToast(
                        `Inherited from "${s.env}" — switch to it to delete`,
                      )
                    : handleDelete(s.key)
                }
              />
            ))}
          </div>
        )}
      </main>

      {toast && (
        <div className="fixed bottom-4 left-1/2 -translate-x-1/2 border border-line-strong bg-popover px-4 py-2 font-mono text-xs text-ink shadow-lg">
          {toast}
        </div>
      )}

      <LoginDialog
        open={loginOpen}
        onOpenChange={setLoginOpen}
        onSignIn={handleSignIn}
      />
      <AddVaultDialog
        open={addVaultOpen}
        onOpenChange={setAddVaultOpen}
        onAdd={handleAddVault}
      />
      <AddEnvDialog
        open={addEnvOpen}
        onOpenChange={setAddEnvOpen}
        envs={envs}
        onAdd={handleAddEnv}
      />
    </div>
  );
}

export default App;
