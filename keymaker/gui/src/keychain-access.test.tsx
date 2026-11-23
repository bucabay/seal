import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import App from "./App";
import { api } from "./lib/api";

vi.mock("./lib/api", () => ({
  api: Object.fromEntries([
    "refs", "tasks", "endpoints", "audit", "environments", "health", "approvals",
    "save", "reveal", "remove", "enableBiometrics",
  ].map((name) => [name, vi.fn()])),
}));

beforeEach(() => {
  vi.resetAllMocks();
  vi.stubGlobal("localStorage", { getItem: () => null, setItem: vi.fn() });
  vi.stubGlobal("navigator", { clipboard: { writeText: vi.fn().mockResolvedValue(undefined) } });
  vi.mocked(api.refs).mockResolvedValue([
    { reference: "test/key", issuer: "test", name: "key", present: true, used_by: [], biometric: false },
  ]);
  for (const method of [api.tasks, api.endpoints, api.audit, api.environments, api.approvals]) {
    vi.mocked(method).mockResolvedValue([]);
  }
  vi.mocked(api.health).mockResolvedValue({
    enforcement: "test", enforced: true, audit_intact: true, audit_entries: 0, missing_refs: [],
  });
});

afterEach(() => { cleanup(); vi.unstubAllGlobals(); });

it("coalesces repeated reveal and copy clicks into one pending keychain read", async () => {
  let resolve!: (value: string) => void;
  vi.mocked(api.reveal).mockImplementation(() => new Promise((done) => { resolve = done; }));
  render(<App />);
  await screen.findByText("key");
  fireEvent.click(screen.getByLabelText("Reveal"));
  fireEvent.click(screen.getByLabelText("Reveal"));
  fireEvent.click(screen.getByLabelText("Copy"));
  expect(api.reveal).toHaveBeenCalledExactlyOnceWith("test/key");
  await act(async () => resolve("test-only-value"));
  expect(navigator.clipboard.writeText).toHaveBeenCalledExactlyOnceWith("test-only-value");
  expect(screen.getByLabelText("Value of test/key")).toBeTruthy();
});

it("does not retry a cancelled read and permits a later deliberate retry", async () => {
  vi.mocked(api.reveal).mockRejectedValueOnce(new Error("Keychain authorization was cancelled"));
  render(<App />);
  await screen.findByText("key");
  fireEvent.click(screen.getByLabelText("Reveal"));
  await screen.findByText("Error: Keychain authorization was cancelled");
  expect(api.reveal).toHaveBeenCalledTimes(1);
  vi.mocked(api.reveal).mockResolvedValue("test-only-value");
  fireEvent.click(screen.getByLabelText("Reveal"));
  await screen.findByLabelText("Value of test/key");
  expect(api.reveal).toHaveBeenCalledTimes(2);
});

it("enables per-item protection without revealing a value to the frontend", async () => {
  let resolve!: () => void;
  vi.mocked(api.enableBiometrics).mockImplementation(() => new Promise((done) => { resolve = done; }));
  render(<App />);
  await screen.findByText("key");
  fireEvent.click(screen.getByLabelText("Enable Touch ID"));
  fireEvent.click(screen.getByLabelText("Enable Touch ID"));
  expect(api.enableBiometrics).toHaveBeenCalledExactlyOnceWith("test/key");
  expect(api.reveal).not.toHaveBeenCalled();
  vi.mocked(api.refs).mockResolvedValue([
    { reference: "test/key", issuer: "test", name: "key", present: true, used_by: [], biometric: true },
  ]);
  await act(async () => resolve());
  await waitFor(() => expect(screen.getByLabelText("Touch ID protected")).toBeTruthy());
});
