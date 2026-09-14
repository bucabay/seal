import { Check, ChevronsUpDown, Layers, Plus, Trash2 } from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";

export interface EnvEntry {
  name: string;
  extends: string | null;
  own_count: number;
}

interface EnvSelectorProps {
  envs: EnvEntry[];
  current: string;
  onSelect: (env: string) => void;
  onAddEnv: () => void;
  onDeleteEnv: (env: string) => void;
}

/** `dev -> prod -> default`, so the fallback order is visible at a glance. */
function lineage(envs: EnvEntry[], name: string): string {
  const byName = new Map(envs.map((e) => [e.name, e]));
  const chain: string[] = [];
  const seen = new Set<string>();
  let cursor: string | null = name;
  while (cursor && !seen.has(cursor)) {
    seen.add(cursor);
    chain.push(cursor);
    cursor = byName.get(cursor)?.extends ?? null;
  }
  return chain.join(" → ");
}

export function EnvSelector({
  envs,
  current,
  onSelect,
  onAddEnv,
  onDeleteEnv,
}: EnvSelectorProps) {
  const chain = lineage(envs, current);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        className="inline-flex h-9 items-center gap-2 border border-line-strong bg-background px-3 text-sm transition-colors hover:bg-accent/50 focus-visible:outline-none"
        title={chain}
      >
        <Layers className="h-4 w-4 text-muted-foreground" />
        <span className="font-mono text-[13px] text-ink">{current}</span>
        <ChevronsUpDown className="h-3.5 w-3.5 text-muted-foreground" />
      </DropdownMenuTrigger>
      <DropdownMenuContent align="start" className="w-72">
        <DropdownMenuLabel className="font-mono text-[11px] uppercase tracking-[0.18em] text-muted-foreground">
          Environment
        </DropdownMenuLabel>
        {envs.map((env) => (
          <DropdownMenuItem
            key={env.name}
            onSelect={() => onSelect(env.name)}
            className="group/env font-mono text-[13px]"
          >
            <div className="flex min-w-0 flex-1 flex-col">
              <span className="truncate">{env.name}</span>
              <span className="truncate text-[11px] text-muted-foreground">
                {env.extends ? `extends ${env.extends}` : "root"} ·{" "}
                {env.own_count} own
              </span>
            </div>
            {env.name === current && <Check className="h-4 w-4 text-brand" />}
            {env.name !== "default" && (
              <button
                onClick={(e) => {
                  e.preventDefault();
                  e.stopPropagation();
                  onDeleteEnv(env.name);
                }}
                className="ml-1 inline-flex h-6 w-6 shrink-0 items-center justify-center text-muted-foreground opacity-0 transition-opacity hover:text-destructive group-hover/env:opacity-100"
                aria-label={`Delete environment ${env.name}`}
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            )}
          </DropdownMenuItem>
        ))}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          onSelect={onAddEnv}
          className="font-mono text-[13px] text-brand"
        >
          <Plus className="h-4 w-4" />
          New environment
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

export default EnvSelector;
