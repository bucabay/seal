import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Copy, Eye, EyeOff, KeyRound, Plus, Trash2 } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Header } from "@/components/header";
import { LoginDialog } from "@/components/login-dialog";
import { AddVaultDialog } from "@/components/add-vault-dialog";
import { useUser } from "@/hooks/use-user";

interface Secret {
  key: string;
  vault: string;
}

function App() {
  const [vaults, setVaults] = useState<string[]>([]);
  const [vault, setVault] = useState("seal");
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [revealed, setRevealed] = useState<Record<string, string>>({});
  const [toast, setToast] = useState<string | null>(null);
  const [loginOpen, setLoginOpen] = useState(false);
  const [addVaultOpen, setAddVaultOpen] = useState(false);

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

  const loadSecrets = useCallback(async () => {
    try {
      const list: Secret[] = await invoke("list_secrets", { vault });
      setSecrets(list);
      setRevealed({});
    } catch (e) {
      console.error(e);
    }
  }, [vault]);

  useEffect(() => {
    loadVaults();
  }, [loadVaults]);

  useEffect(() => {
    loadSecrets();
  }, [loadSecrets]);

  const handleSave = async () => {
    if (!newKey.trim() || !newValue.trim()) return;
    try {
      await invoke("save_secret", {
        key: newKey.trim(),
        value: newValue,
        vault,
      });
      setNewKey("");
      setNewValue("");
      showToast("Saved");
      loadSecrets();
      loadVaults();
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleReveal = async (key: string) => {
    if (key in revealed) {
      const next = { ...revealed };
      delete next[key];
      setRevealed(next);
      return;
    }
    try {
      const value: string = await invoke("get_secret", { key, vault });
      setRevealed((r) => ({ ...r, [key]: value }));
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleCopy = async (key: string) => {
    try {
      const value: string = await invoke("get_secret", { key, vault });
      await navigator.clipboard.writeText(value);
      showToast("Copied");
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleDelete = async (key: string) => {
    try {
      await invoke("delete_secret", { key, vault });
      loadSecrets();
      loadVaults();
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

  const handleSignIn = (name: string, email: string) => {
    signIn({ name, email });
    showToast(`Signed in as ${name}`);
  };

  return (
    <div className="flex min-h-screen flex-col bg-background text-ink">
      <Header
        vaults={vaults}
        current={vault}
        user={user}
        onSelectVault={setVault}
        onAddVault={() => setAddVaultOpen(true)}
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
          </span>
        </div>

        {secrets.length === 0 ? (
          <div className="px-6 py-16 text-center">
            <div className="font-display text-lg text-ink">
              No secrets yet
            </div>
            <p className="mt-1 font-mono text-xs text-muted-foreground">
              Add one above, or switch vaults.
            </p>
          </div>
        ) : (
          <div className="border-t border-line">
            {secrets.map((s) => {
              const isVisible = s.key in revealed;
              return (
                <div
                  key={s.key}
                  className="group flex items-center gap-3 border-b border-line px-6 py-3 transition-colors hover:bg-surface-tint"
                >
                  <span
                    className="flex-1 cursor-pointer font-mono text-[13px] text-ink"
                    onClick={() => handleReveal(s.key)}
                    title={s.key}
                  >
                    {s.key}
                  </span>
                  <span className="w-64 truncate font-mono text-[13px] text-muted-foreground">
                    {isVisible ? revealed[s.key] : "••••••••••••"}
                  </span>
                  <div className="flex shrink-0 items-center gap-1 opacity-0 transition-opacity group-hover:opacity-100">
                    <button
                      onClick={() => handleReveal(s.key)}
                      className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-accent/50 hover:text-ink"
                      aria-label={isVisible ? "Hide" : "Reveal"}
                    >
                      {isVisible ? (
                        <EyeOff className="h-4 w-4" />
                      ) : (
                        <Eye className="h-4 w-4" />
                      )}
                    </button>
                    <button
                      onClick={() => handleCopy(s.key)}
                      className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-accent/50 hover:text-ink"
                      aria-label="Copy"
                    >
                      <Copy className="h-4 w-4" />
                    </button>
                    <button
                      onClick={() => handleDelete(s.key)}
                      className="inline-flex h-8 w-8 items-center justify-center text-muted-foreground hover:bg-destructive/20 hover:text-destructive"
                      aria-label="Delete"
                    >
                      <Trash2 className="h-4 w-4" />
                    </button>
                  </div>
                </div>
              );
            })}
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
    </div>
  );
}

export default App;
