import { invoke } from "@tauri-apps/api/core";

export type RefRow = {
  reference: string;
  /** Derived from the reference, never stored. */
  issuer: string;
  name: string;
  present: boolean;
  used_by: string[];
};
export type TaskRow = { name: string; command: string };
export type EndpointRow = {
  name: string;
  method: string;
  host: string;
  path: string;
  secret: string;
  gated: boolean;
};
export type AuditRow = { seq: number; at: number; summary: string };
export type Health = {
  enforcement: string;
  enforced: boolean;
  audit_intact: boolean;
  audit_entries: number;
  missing_refs: string[];
};
export type ApprovalRow = {
  id: string;
  capability: string;
  rule: string;
  detail: string;
  seconds_left: number;
};
export type RunResult = {
  exit_code: number | null;
  stdout: string;
  stderr: string;
  redacted: boolean;
  leaked_files: string[];
};

export const api = {
  refs: () => invoke<RefRow[]>("list_refs"),
  /**
   * The one call in the whole product that returns a secret value.
   * It exists because a person sometimes has to check which key they stored,
   * and every use is written to the audit chain.
   */
  reveal: (reference: string) => invoke<string>("reveal", { reference }),
  save: (reference: string, value: string) =>
    invoke<void>("save_secret", { reference, value }),
  remove: (reference: string) => invoke<void>("delete_secret", { reference }),
  tasks: () => invoke<TaskRow[]>("list_tasks"),
  endpoints: () => invoke<EndpointRow[]>("list_endpoints"),
  environments: () => invoke<string[]>("list_environments"),
  audit: () => invoke<AuditRow[]>("audit_rows"),
  health: () => invoke<Health>("health"),
  runTask: (task: string, env: string) =>
    invoke<RunResult>("run_task", { task, env }),
  approvals: () => invoke<ApprovalRow[]>("pending_approvals"),
  decide: (id: string, granted: boolean) =>
    invoke<void>("decide_approval", { id, granted }),
};
