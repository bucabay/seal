import { describe, expect, it } from "vitest";
import { DEFAULT_ISSUER, grouped, matches, splitPasted } from "./group";
import type { RefRow } from "./api";

const row = (reference: string, present = true, used_by: string[] = []): RefRow => {
  const at = reference.indexOf("/");
  const [issuer, name] =
    at > 0 && at < reference.length - 1
      ? [reference.slice(0, at), reference.slice(at + 1)]
      : [DEFAULT_ISSUER, reference];
  return { reference, issuer, name, present, used_by };
};

describe("splitPasted", () => {
  it("splits a pasted reference and value", () => {
    expect(splitPasted("stripe/api-key sk_xxx")).toEqual({
      reference: "stripe/api-key",
      value: "sk_xxx",
    });
  });

  it("keeps spaces inside a value", () => {
    expect(splitPasted("vault/pass correct horse battery staple")).toEqual({
      reference: "vault/pass",
      value: "correct horse battery staple",
    });
  });

  it("returns no value for a bare reference", () => {
    expect(splitPasted("stripe/api-key")).toEqual({ reference: "stripe/api-key" });
    expect(splitPasted("  stripe/api-key   ")).toEqual({ reference: "stripe/api-key" });
  });

  it("treats a tab as a separator, for pastes out of a spreadsheet", () => {
    expect(splitPasted("stripe/api-key\tsk_xxx")).toEqual({
      reference: "stripe/api-key",
      value: "sk_xxx",
    });
  });
});

describe("matches", () => {
  const r = row("stripe/sk_live", true, ["mailkite", "hardroad"]);

  it("matches on the reference", () => {
    expect(matches(r, "stri")).toBe(true);
    expect(matches(r, "sk_live")).toBe(true);
    expect(matches(r, "GITHUB")).toBe(false);
  });

  it("matches on the projects using it, so a project name finds its keys", () => {
    expect(matches(r, "hardroad")).toBe(true);
  });

  it("an empty query matches everything", () => {
    expect(matches(r, "")).toBe(true);
    expect(matches(r, "   ")).toBe(true);
  });
});

describe("grouped", () => {
  const rows = [
    row("stripe/sk_live"),
    row("stripe/whsec", false),
    row("github/token"),
    row("looseend"),
  ];

  it("derives groups from the names, with nothing stored", () => {
    expect(grouped(rows, "").map((g) => g.issuer)).toEqual([
      "github",
      "stripe",
      DEFAULT_ISSUER,
    ]);
  });

  it("puts references with no value first, since those need attention", () => {
    const stripe = grouped(rows, "").find((g) => g.issuer === "stripe")!;
    expect(stripe.rows.map((r) => r.reference)).toEqual([
      "stripe/whsec",
      "stripe/sk_live",
    ]);
  });

  it("counts how many of a group are stored", () => {
    const stripe = grouped(rows, "").find((g) => g.issuer === "stripe")!;
    expect(stripe.stored).toBe(1);
    expect(stripe.rows.length).toBe(2);
  });

  it("a filter drops groups with nothing in them", () => {
    expect(grouped(rows, "stripe").map((g) => g.issuer)).toEqual(["stripe"]);
  });

  it("keeps the catch-all group at the bottom", () => {
    // Alphabetically `general` would land first; it belongs last.
    const withGeneral = [row("scratch"), row("anthropic/api_key")];
    expect(grouped(withGeneral, "").map((g) => g.issuer)).toEqual([
      "anthropic",
      DEFAULT_ISSUER,
    ]);
  });

  it("a group appears as soon as a key is named into it", () => {
    // The property that makes groups free: naming is the only act.
    const before = grouped(rows, "").map((g) => g.issuer);
    expect(before).not.toContain("anthropic");
    const after = grouped([...rows, row("anthropic/api_key")], "").map((g) => g.issuer);
    expect(after).toContain("anthropic");
  });

  it("filtering to nothing yields no groups rather than empty ones", () => {
    expect(grouped(rows, "no-such-thing")).toEqual([]);
  });
});
