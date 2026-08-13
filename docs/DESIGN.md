# Design System

The GUI is built on **ShadCN/ui** (Radix primitives + Tailwind) styled after
**kitedeploy.com** — a technical, "engineering tool" aesthetic: sharp corners,
hairline borders, mono labels, and a split orange/blue brand.

## Framework

| Layer | Choice |
|---|---|
| UI kit | ShadCN/ui (new-york style) |
| Primitives | Radix UI (dialog, dropdown-menu, avatar, label, separator, slot) |
| Styling | Tailwind CSS v3.4 |
| Icons | Lucide |
| Fonts | Space Grotesk, DM Sans, DM Mono (self-hosted via `@fontsource`) |
| Build | Vite 6 + React 19 + TypeScript |

Components are copied into `src/components/ui/` (not imported from a package),
so they can be freely restyled. `components.json` documents the setup.

## Color tokens

Defined as CSS variables in `src/index.css`, referenced by Tailwind in
`tailwind.config.cjs` (`ink`, `brand`, `line`, plus the standard shadcn tokens).

### Light

| Token | Value | Use |
|---|---|---|
| `--background` | `#ffffff` | page background |
| `--ink` | `#171717` | primary text |
| `--ink-soft` | `#525252` | muted text |
| `--brand` / `--primary` | `#f38020` (orange) | brand / primary actions |
| `--accent` | `#3b82f6` (blue) | secondary accent |
| `--line` / `--border` | `#e7e5e4` | hairlines |

### Dark

| Token | Value | Use |
|---|---|---|
| `--background` | `#0d0d0d` | page background |
| `--ink` | `#ededed` | primary text |
| `--ink-soft` | `#8a8a8a` | muted text |
| `--brand` / `--primary` | `#3b82f6` (blue) | brand / primary actions |
| `--accent` | `#f38020` (orange) | secondary accent |
| `--line` / `--border` | `#1e2430` | hairlines |

The brand color **flips between light and dark** (orange in light, blue in
dark) — same as kitedeploy.com. Dark is the default theme.

## Typography

- **Display** (headings, wordmark): Space Grotesk (`font-display`)
- **Body**: DM Sans (`font-sans`)
- **Mono** (keys, values, labels, eyebrows): DM Mono (`font-mono`)

`eyebrow` — a reusable mono uppercase label with wide tracking:

```
font-mono text-[11px] uppercase tracking-[0.18em] text-ink-soft
```

## Shape & surface

- **Radius: 0** — fully sharp corners (`--radius: 0rem`), a deliberate signature
  look.
- **Hairlines** everywhere: `border-line` (1px) separates header, rows, and
  dialogs.
- **Halftone dot field** (`dotfield`) is available as a background texture
  (used sparingly).

## Theming

`src/hooks/use-theme.tsx` provides a `ThemeProvider` that:

1. Persists the theme to `localStorage` (`seal-theme`).
2. Toggles the `.dark` class on `<html>`.
3. Defaults to dark.

The header renders a sun/moon toggle calling `useTheme().toggleTheme`.

## Components

| Component | Location | Notes |
|---|---|---|
| `Header` | `components/header.tsx` | logo + vault selector + theme toggle + user menu |
| `VaultSelector` | `components/vault-selector.tsx` | dropdown of vaults + "Add vault" |
| `UserMenu` | `components/user-menu.tsx` | avatar → sign in / sign out |
| `LoginDialog` | `components/login-dialog.tsx` | name/email/password (UI only) |
| `AddVaultDialog` | `components/add-vault-dialog.tsx` | create a vault |
| ShadCN primitives | `components/ui/*` | button, dialog, dropdown-menu, avatar, input, label, separator |

## App icon

A white **shield with a keyhole cutout** on a blue gradient rounded-square
(`#4f8ef7 → #2563eb`), tying into the in-app `ShieldCheck` wordmark.

- Source: `src-tauri/icons/icon.svg` (edit and regenerate).
- Generated: `icon.png` (1024), `32x32.png`, `128x128.png`, `128x128@2x.png`,
  `icon.icns` (macOS), `icon.ico` (Windows).

The shield evokes security; the keyhole evokes secrets/vault; the combination
is more distinctive than a bare lock. Rounded-square over circle because it is
the macOS convention and reads well on Windows/Linux.
