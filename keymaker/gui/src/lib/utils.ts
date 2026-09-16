import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/** Audit timestamps are unix seconds. */
export function when(at: number): string {
  return new Date(at * 1000).toLocaleString();
}

/**
 * Audit events arrive as a JSON object; show the event name separately from
 * its fields so a long line stays readable.
 */
export function describeEvent(summary: string): { kind: string; detail: string } {
  try {
    const parsed = JSON.parse(summary) as Record<string, unknown>;
    const kind = String(parsed.event ?? "event");
    const detail = Object.entries(parsed)
      .filter(([k]) => k !== "event")
      .map(([k, v]) => `${k}=${String(v)}`)
      .join("  ");
    return { kind, detail };
  } catch {
    return { kind: "event", detail: summary };
  }
}
