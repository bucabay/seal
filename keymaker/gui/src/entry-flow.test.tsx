import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import App from "./App";
import { AddSecret } from "./components/add-secret";
import { api, type RefRow } from "./lib/api";

vi.mock("./lib/api", () => ({
  api: Object.fromEntries([
    "refs", "tasks", "endpoints", "audit", "environments", "health",
    "approvals", "save", "reveal", "remove",
  ].map((name) => [name, vi.fn()])),
}));

const rows: RefRow[] = ["alpha", "middle", "zulu"].map((name) => ({
  reference: `stripe/${name}`, issuer: "stripe", name, present: true, used_by: [],
}));

beforeEach(() => {
  vi.resetAllMocks();
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  vi.mocked(api.refs).mockResolvedValue(rows);
  for (const method of [api.tasks, api.endpoints, api.audit, api.environments, api.approvals]) {
    vi.mocked(method).mockResolvedValue([]);
  }
  vi.mocked(api.health).mockResolvedValue({
    enforced: true, enforcement: "test", audit_intact: true, audit_entries: 0, missing_refs: [],
  });
  vi.mocked(api.reveal).mockResolvedValue("old-value");
  // Leave writes pending so tests can inspect the UI before persistence finishes.
  vi.mocked(api.save).mockImplementation(() => new Promise(() => {}));
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

function fill(reference: string, value = "test-value") {
  fireEvent.change(screen.getByLabelText("Reference"), { target: { value: reference } });
  fireEvent.change(screen.getByLabelText("Value"), { target: { value } });
}

async function start() {
  render(<App />);
  await screen.findByText("middle");
}

describe("entry keyboard and validation", () => {
  it.each(["Enter", "Tab"])("%s moves name to value exactly once, then submits and resets focus", (key) => {
    const onCommit = vi.fn();
    render(<AddSecret prefill="stripe/" onCommit={onCommit} />);
    fill("stripe/test");
    const name = screen.getByLabelText("Reference");
    const value = screen.getByLabelText("Value");
    name.focus();
    expect(fireEvent.keyDown(name, { key })).toBe(false);
    expect(document.activeElement).toBe(value);
    expect(fireEvent.keyDown(value, { key })).toBe(false);
    expect(onCommit).toHaveBeenCalledExactlyOnceWith("stripe/test", "test-value");
    expect(document.activeElement).toBe(name);
    expect((name as HTMLInputElement).value).toBe("stripe/");
    expect((value as HTMLInputElement).value).toBe("");
  });

  it("keeps an invalid reference and value available for correction", () => {
    const onCommit = vi.fn();
    render(<AddSecret prefill="stripe/" onCommit={onCommit} />);
    fill("stripe/");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    expect(onCommit).not.toHaveBeenCalled();
    expect((screen.getByLabelText("Value") as HTMLInputElement).value).toBe("test-value");
    expect(screen.getByText("a reference cannot begin or end with `/`")).toBeTruthy();
  });

  it("leaves arrow navigation, Shift-Tab and composition alone", () => {
    const onCommit = vi.fn();
    render(<AddSecret prefill="stripe/" onCommit={onCommit} />);
    fill("stripe/test");
    const value = screen.getByLabelText("Value");
    expect(fireEvent.keyDown(value, { key: "ArrowRight" })).toBe(true);
    expect(fireEvent.keyDown(value, { key: "Tab", shiftKey: true })).toBe(true);
    expect(fireEvent.keyDown(value, { key: "Enter", isComposing: true })).toBe(true);
    expect(onCommit).not.toHaveBeenCalled();
  });
});

describe("App entry flow", () => {
  it("global save immediately shows a destination row and keeps the top form ready", async () => {
    await start();
    fireEvent.click(screen.getByText("add a secret"));
    fill("mailkite/test");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    expect(api.save).toHaveBeenCalledExactlyOnceWith("mailkite/test", "test-value");
    expect(screen.getByLabelText("Collapse mailkite")).toBeTruthy();
    expect(screen.getByText("test")).toBeTruthy();
    expect(screen.getByText("saving…")).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByLabelText("Reference"));
    expect((screen.getByLabelText("Value") as HTMLInputElement).value).toBe("");
  });

  it("provider additions append and put the next form beneath the submitted row", async () => {
    await start();
    fireEvent.click(screen.getByLabelText("Add a key under stripe"));
    fill("stripe/aaa");
    fireEvent.click(screen.getByRole("button", { name: "save" }));
    const provider = screen.getByLabelText("Collapse stripe").parentElement!.parentElement!;
    const inputs = within(provider).getAllByRole("textbox");
    // Password inputs have no implicit textbox role; the one textbox is the next name.
    expect(inputs).toHaveLength(1);
    const labels = [...provider.querySelectorAll("input")].map((input) => input.getAttribute("aria-label"));
    expect(labels).toEqual([
      "stripe/alpha, hidden", "stripe/middle, hidden", "stripe/zulu, hidden",
      "stripe/aaa, hidden", "Reference", "Value",
    ]);
    expect(document.activeElement).toBe(screen.getByLabelText("Reference"));
  });

  it("saves a middle row immediately, inserts below it, and refocuses an already-open form", async () => {
    await start();
    fireEvent.click(screen.getByLabelText("stripe/middle, hidden"));
    const value = await screen.findByLabelText("Value of stripe/middle");
    fireEvent.change(value, { target: { value: "updated" } });
    fireEvent.keyDown(value, { key: "Tab" });
    expect(api.save).toHaveBeenCalledExactlyOnceWith("stripe/middle", "updated");
    const name = screen.getByLabelText("Reference");
    expect(document.activeElement).toBe(name);
    value.focus();
    fireEvent.keyDown(value, { key: "Enter" });
    expect(document.activeElement).toBe(name);
    expect(api.save).toHaveBeenCalledTimes(1);
    fill("stripe/new");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    const provider = screen.getByLabelText("Collapse stripe").parentElement!.parentElement!;
    expect([...provider.querySelectorAll("input")].map((input) => input.getAttribute("aria-label"))).toEqual([
      "stripe/alpha, hidden", "Value of stripe/middle", "stripe/new, hidden", "Reference", "Value", "stripe/zulu, hidden",
    ]);
  });

  it("changing the prefix moves both the submitted row and inline continuation to that provider", async () => {
    await start();
    fireEvent.click(screen.getByLabelText("Add a key under stripe"));
    fill("mailkite/test");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    const destination = screen.getByLabelText("Collapse mailkite").parentElement!.parentElement!;
    expect(within(destination).getByText("test")).toBeTruthy();
    expect(within(destination).getByLabelText("Reference")).toBe(document.activeElement);
    expect((screen.getByLabelText("Reference") as HTMLInputElement).value).toBe("mailkite/");
  });

  it("retains a failed addition after closing the form and retries on Enter", async () => {
    vi.mocked(api.save).mockRejectedValueOnce(new Error("write failed"));
    await start();
    fireEvent.click(screen.getByLabelText("Add a key under stripe"));
    fill("stripe/test");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    await screen.findByText("failed");
    fireEvent.click(screen.getByText("done"));
    const failed = screen.getByLabelText("stripe/test, hidden") as HTMLInputElement;
    expect(failed.value).toBe("test-value");
    expect(failed.type).toBe("password");
    fireEvent.keyDown(failed, { key: "Enter" });
    expect(api.save).toHaveBeenCalledTimes(2);
    expect(api.save).toHaveBeenLastCalledWith("stripe/test", "test-value");
    expect(document.activeElement).toBe(screen.getByLabelText("Reference"));
  });

  it("erasing an existing draft cancels the older queued value", async () => {
    await start();
    fireEvent.click(screen.getByLabelText("stripe/middle, hidden"));
    const value = await screen.findByLabelText("Value of stripe/middle");
    fireEvent.change(value, { target: { value: "do-not-save" } });
    fireEvent.change(value, { target: { value: "" } });
    fireEvent.keyDown(value, { key: "Enter" });
    expect(api.save).not.toHaveBeenCalled();
    expect(screen.getByText("empty")).toBeTruthy();
  });

  it("retries an existing row's failed draft without needing another edit", async () => {
    vi.mocked(api.save).mockRejectedValueOnce(new Error("write failed"));
    await start();
    fireEvent.click(screen.getByLabelText("stripe/middle, hidden"));
    const value = await screen.findByLabelText("Value of stripe/middle");
    fireEvent.change(value, { target: { value: "retry-this" } });
    fireEvent.keyDown(value, { key: "Enter" });
    await screen.findByText("failed");
    fireEvent.keyDown(value, { key: "Tab" });
    expect(api.save).toHaveBeenCalledTimes(2);
    expect(api.save).toHaveBeenLastCalledWith("stripe/middle", "retry-this");
  });

  it("marks success only after persistence, keeping position and the next draft through refresh", async () => {
    let finish!: () => void;
    vi.mocked(api.save).mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    await start();
    fireEvent.click(screen.getByLabelText("Add a key under stripe"));
    fill("stripe/aaa");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    expect(screen.queryByText("saved")).toBeNull();
    fill("stripe/next", "next-draft");
    await act(async () => finish());
    expect(screen.getByText("saved")).toBeTruthy();
    expect((screen.getByLabelText("Value") as HTMLInputElement).value).toBe("next-draft");
    // A lagging inventory response must not remove the newly saved row.
    fireEvent.click(screen.getByLabelText("Refresh"));
    await waitFor(() => expect(api.refs).toHaveBeenCalledTimes(2));
    expect(screen.getByText("aaa")).toBeTruthy();
    const provider = screen.getByLabelText("Collapse stripe").parentElement!.parentElement!;
    expect([...provider.querySelectorAll("input")].map((input) => input.getAttribute("aria-label"))).toEqual([
      "stripe/alpha, hidden", "stripe/middle, hidden", "stripe/zulu, hidden",
      "stripe/aaa, hidden", "Reference", "Value",
    ]);
    expect((screen.getByLabelText("Reference") as HTMLInputElement).value).toBe("stripe/next");
  });

  it("serializes duplicate-reference submissions and retains the newest draft", async () => {
    let finish!: () => void;
    vi.mocked(api.save).mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    await start();
    fireEvent.click(screen.getByText("add a secret"));
    fill("stripe/test", "first");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    fill("stripe/test", "second");
    fireEvent.keyDown(screen.getByLabelText("Value"), { key: "Enter" });
    expect(api.save).toHaveBeenCalledTimes(1);
    await act(async () => finish());
    await waitFor(() => expect(api.save).toHaveBeenCalledTimes(2));
    expect(api.save).toHaveBeenLastCalledWith("stripe/test", "second");
    expect((screen.getByLabelText("stripe/test, hidden") as HTMLInputElement).value).toBe("second");
  });
});
