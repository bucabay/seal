import { describe, expect, it } from "vitest";
import { actionFor, nextPrefix, shouldPreventDefault } from "./entry";

describe("actionFor", () => {
  it("Enter and Tab do the same thing in a name: move on", () => {
    expect(actionFor({ key: "Enter" }, "name")).toBe("next");
    expect(actionFor({ key: "Tab" }, "name")).toBe("next");
  });

  it("Enter and Tab do the same thing in a value: save and continue", () => {
    expect(actionFor({ key: "Enter" }, "value")).toBe("commit");
    expect(actionFor({ key: "Tab" }, "value")).toBe("commit");
  });

  it("Escape backs out of either field", () => {
    expect(actionFor({ key: "Escape" }, "name")).toBe("cancel");
    expect(actionFor({ key: "Escape" }, "value")).toBe("cancel");
  });

  it("leaves shift-tab alone, since going backwards already works", () => {
    expect(actionFor({ key: "Tab", shiftKey: true }, "name")).toBeNull();
    expect(actionFor({ key: "Tab", shiftKey: true }, "value")).toBeNull();
  });

  it("never steals a modified keystroke", () => {
    // Cmd-Enter, Ctrl-Tab and the like belong to the user or the system.
    for (const mod of [{ metaKey: true }, { ctrlKey: true }, { altKey: true }]) {
      expect(actionFor({ key: "Enter", ...mod }, "value")).toBeNull();
      expect(actionFor({ key: "Tab", ...mod }, "name")).toBeNull();
    }
    // Escape is still an escape, whatever is held down.
    expect(actionFor({ key: "Escape", metaKey: true }, "value")).toBe("cancel");
  });

  it("ordinary typing is not an action", () => {
    for (const key of ["a", "/", "Backspace", "ArrowDown", " "]) {
      expect(actionFor({ key }, "name")).toBeNull();
      expect(actionFor({ key }, "value")).toBeNull();
    }
  });
});

describe("shouldPreventDefault", () => {
  it("takes over Tab in a name so explicit focus only advances once", () => {
    expect(shouldPreventDefault({ key: "Tab" }, "name")).toBe(true);
  });

  it("takes over Enter in a name, since the browser would do nothing useful", () => {
    expect(shouldPreventDefault({ key: "Enter" }, "name")).toBe(true);
  });

  it("takes over both keys in a value, so focus does not jump to a button", () => {
    expect(shouldPreventDefault({ key: "Enter" }, "value")).toBe(true);
    expect(shouldPreventDefault({ key: "Tab" }, "value")).toBe(true);
  });

  it("leaves everything else to the browser", () => {
    expect(shouldPreventDefault({ key: "a" }, "value")).toBe(false);
    expect(shouldPreventDefault({ key: "Tab", shiftKey: true }, "value")).toBe(false);
  });
});

describe("nextPrefix", () => {
  it("follows where the key was actually saved, not where the form opened", () => {
    // Typed `mailkite/test` into a form opened under hardroad: the next key is
    // probably mailkite's too.
    expect(nextPrefix("mailkite/test", "hardroad/")).toBe("mailkite/");
  });

  it("keeps the issuer for a run of keys in one place", () => {
    expect(nextPrefix("stripe/sk_live", "stripe/")).toBe("stripe/");
  });

  it("falls back when what was saved has no issuer", () => {
    expect(nextPrefix("scratch", "stripe/")).toBe("stripe/");
    expect(nextPrefix("scratch")).toBe("");
  });

  it("treats a half-written reference as having no issuer", () => {
    expect(nextPrefix("stripe/", "github/")).toBe("github/");
    expect(nextPrefix("/leading", "github/")).toBe("github/");
  });
});
