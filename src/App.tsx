import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Secret {
  key: string;
  account: string;
}

function App() {
  const [account, setAccount] = useState("seal");
  const [secrets, setSecrets] = useState<Secret[]>([]);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [visibleKeys, setVisibleKeys] = useState<Set<string>>(new Set());
  const [revealedValues, setRevealedValues] = useState<Map<string, string>>(new Map());
  const [toast, setToast] = useState<string | null>(null);

  const showToast = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2000);
  };

  const loadSecrets = useCallback(async () => {
    try {
      const list: Secret[] = await invoke("list_secrets", { account });
      setSecrets(list);
    } catch (e) {
      console.error(e);
    }
  }, [account]);

  useEffect(() => {
    loadSecrets();
  }, [loadSecrets]);

  const handleSave = async () => {
    if (!newKey.trim() || !newValue.trim()) return;
    try {
      const key = newKey.includes("/") ? newKey : newKey.trim();
      await invoke("save_secret", { key, value: newValue, account });
      setNewKey("");
      setNewValue("");
      showToast("Saved");
      loadSecrets();
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleReveal = async (key: string) => {
    if (revealedValues.has(key)) {
      revealedValues.delete(key);
      setRevealedValues(new Map(revealedValues));
      visibleKeys.delete(key);
      setVisibleKeys(new Set(visibleKeys));
      return;
    }
    try {
      const value: string = await invoke("get_secret", { key, account });
      revealedValues.set(key, value);
      setRevealedValues(new Map(revealedValues));
      visibleKeys.add(key);
      setVisibleKeys(new Set(visibleKeys));
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleCopy = async (key: string) => {
    try {
      const value: string = await invoke("get_secret", { key, account });
      await navigator.clipboard.writeText(value);
      showToast("Copied!");
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleDelete = async (key: string) => {
    try {
      await invoke("delete_secret", { key, account });
      loadSecrets();
      showToast("Deleted");
    } catch (e: any) {
      showToast(`Error: ${e}`);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter") handleSave();
  };

  return (
    <div>
      <header>
        <h1>Seal</h1>
        <div className="account-bar">
          <span style={{ fontSize: 13, color: "var(--text-muted)" }}>account</span>
          <input
            value={account}
            onChange={(e) => setAccount(e.target.value)}
            placeholder="seal"
          />
        </div>
      </header>

      <div className="add-form">
        <input
          placeholder="key (or ns/key)"
          value={newKey}
          onChange={(e) => setNewKey(e.target.value)}
          onKeyDown={handleKeyDown}
        />
        <input
          placeholder="value"
          value={newValue}
          onChange={(e) => setNewValue(e.target.value)}
          onKeyDown={handleKeyDown}
        />
        <button onClick={handleSave}>Save</button>
      </div>

      {secrets.length === 0 && (
        <div className="empty">No secrets saved under &quot;{account}&quot;</div>
      )}

      {secrets.map((s) => {
        const isVisible = visibleKeys.has(s.key);
        const value = revealedValues.get(s.key);
        return (
          <div key={s.key} className="secret-row">
            <span className="secret-key" onClick={() => handleReveal(s.key)}>
              {s.key}
            </span>
            {isVisible ? (
              <span className="secret-value visible">{value}</span>
            ) : (
              <span className="secret-value">●●●●●●●●</span>
            )}
            <button className="btn" onClick={() => handleCopy(s.key)}>
              Copy
            </button>
            <button className="btn btn-danger" onClick={() => handleDelete(s.key)}>
              Del
            </button>
          </div>
        );
      })}

      {toast && <div className="toast">{toast}</div>}
    </div>
  );
}

export default App;
