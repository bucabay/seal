import { Check, ChevronsUpDown, Folder, Plus } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

interface AccountSelectorProps {
  accounts: string[];
  current: string;
  onSelect: (account: string) => void;
  onAddAccount: () => void;
}

export function AccountSelector({
  accounts,
  current,
  onSelect,
  onAddAccount,
}: AccountSelectorProps) {
  return (
    <DropdownMenu>
      <DropdownMenuTrigger className="inline-flex h-9 items-center gap-2 border border-line-strong bg-background px-3 text-sm transition-colors hover:bg-accent/50 focus-visible:outline-none">
        <Folder className="h-4 w-4 text-muted-foreground" />
        <span className="font-mono text-[13px] text-ink">{current}</span>
        <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-56">
        <DropdownMenuLabel className="font-mono text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
          Account
        </DropdownMenuLabel>
        {accounts.map((account) => (
          <DropdownMenuItem
            key={account}
            onSelect={() => onSelect(account)}
            className="font-mono text-[13px]"
          >
            <span className="flex-1 truncate">{account}</span>
            {account === current && <Check className="h-4 w-4 text-brand" />}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={onAddAccount}
          className="font-mono text-[13px] text-brand"
        >
          <Plus className="h-4 w-4" />
          Add account
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default AccountSelector;
