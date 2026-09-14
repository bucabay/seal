import { useEffect, useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { EnvEntry } from "@/components/env-selector";

interface AddEnvDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  envs: EnvEntry[];
  onAdd: (name: string, extends_: string) => void;
}

export function AddEnvDialog({
  open,
  onOpenChange,
  envs,
  onAdd,
}: AddEnvDialogProps) {
  const [name, setName] = useState("");
  const [parent, setParent] = useState("default");

  // Reopening with a different vault's environments must not keep a parent
  // that no longer exists.
  useEffect(() => {
    if (open) {
      setName("");
      setParent(envs.some((e) => e.name === "default") ? "default" : envs[0]?.name ?? "default");
    }
  }, [open, envs]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    onAdd(name.trim(), parent);
    onOpenChange(false);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader>
          <DialogTitle className="font-display">New environment</DialogTitle>
          <DialogDescription>
            An environment only stores what it overrides — anything it does not
            define is read from the one it extends.
          </DialogDescription>
        </DialogHeader>
        <form onSubmit={handleSubmit} className="space-y-4">
          <div className="space-y-2">
            <Label htmlFor="env">Name</Label>
            <Input
              id="env"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="production"
              autoFocus
            />
          </div>
          <div className="space-y-2">
            <Label htmlFor="extends">Extends</Label>
            <select
              id="extends"
              value={parent}
              onChange={(e) => setParent(e.target.value)}
              className="flex h-9 w-full border border-line-strong bg-background px-3 font-mono text-[13px] text-ink focus-visible:outline-none"
            >
              {envs.map((env) => (
                <option key={env.name} value={env.name}>
                  {env.name}
                </option>
              ))}
            </select>
          </div>
          <DialogFooter>
            <Button type="submit" className="w-full">
              Create environment
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

export default AddEnvDialog;
