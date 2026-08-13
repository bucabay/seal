import { Check, ChevronsUpDown, Folder, Plus } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface VaultSelectorProps {
  vaults: string[];
  current: string;
  onSelect: (vault: string) => void;
  onAddVault: () => void;
}

export function VaultSelector({
  vaults,
  current,
  onSelect,
  onAddVault,
}: VaultSelectorProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="inline-flex h-9 items-center gap-2 border border-line-strong bg-background px-3 text-sm transition-colors hover:bg-accent/50 focus-visible:outline-none">
        <Folder className="h-4 w-4 text-muted-foreground" />
        <span className="font-mono text-[13px] text-ink">{current}</span>
        <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-56">
        <DropdownMenuLabel className="font-mono text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
          Vault
        </DropdownMenuLabel>
        {vaults.map((vault) => (
          <DropdownMenuItem
            key={vault}
            onSelect={() => onSelect(vault)}
            className="font-mono text-[13px]"
          >
            <span className="flex-1 truncate">{vault}</span>
            {vault === current && <Check className="h-4 w-4 text-brand" />}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={onAddVault}
          className="font-mono text-[13px] text-brand"
        >
          <Plus className="h-4 w-4" />
          Add vault
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default VaultSelector;
