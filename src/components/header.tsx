import { Moon, ShieldCheck, Sun } from "lucide-react";

import type { User } from "@/hooks/use-user";
import { useTheme } from "@/hooks/use-theme";
import { VaultSelector } from "@/components/vault-selector";
import { UserMenu } from "@/components/user-menu";

interface HeaderProps {
  vaults: string[];
  current: string;
  user: User | null;
  onSelectVault: (vault: string) => void;
  onAddVault: () => void;
  onSignIn: () => void;
  onSignOut: () => void;
}

export function Header({
  vaults,
  current,
  user,
  onSelectVault,
  onAddVault,
  onSignIn,
  onSignOut,
}: HeaderProps) {
  const { theme, toggleTheme } = useTheme();

  return (
    <header className="flex h-14 items-center gap-3 border-b border-line px-4">
      <div className="flex items-center gap-2">
        <ShieldCheck className="h-5 w-5 text-brand" />
        <span className="font-display text-lg font-semibold tracking-tight text-ink">
          Seal
        </span>
      </div>

      <div className="mx-2 h-6 w-px bg-line" />

      <VaultSelector
        vaults={vaults}
        current={current}
        onSelect={onSelectVault}
        onAddVault={onAddVault}
      />

      <div className="ml-auto flex items-center gap-2">
        <button
          onClick={toggleTheme}
          className="inline-flex h-9 w-9 items-center justify-center border border-line-strong bg-background text-muted-foreground transition-colors hover:bg-accent/50 hover:text-ink"
          aria-label="Toggle theme"
        >
          {theme === "dark" ? (
            <Sun className="h-4 w-4" />
          ) : (
            <Moon className="h-4 w-4" />
          )}
        </button>
        <UserMenu user={user} onSignIn={onSignIn} onSignOut={onSignOut} />
      </div>
    </header>
  );
}

export default Header;
