# Running the GUI

```sh
pnpm --dir ../../gui build     # produce gui/dist
cargo build                    # or cargo build --release
./target/debug/keymaker-gui
```

## Building a real app bundle

```sh
pnpm --dir ../../gui add -D @tauri-apps/cli@^2     # once
../../gui/node_modules/.bin/tauri build
```

Produces `target/release/bundle/macos/Keymaker.app` and a `.dmg` beside it.

Note that `beforeBuildCommand` runs from the directory **above** this one
(`crates/`), not from here — which is why it reads `../gui` while
`frontendDist` alongside it reads `../../gui/dist`. They look inconsistent and
are both correct.

## Where it looks for a project

The GUI reads `.keymaker` and `.keymaker.endpoints.toml` from its working
directory. Launched from Finder that is `/`, so it shows stored secrets but no
tasks or endpoints. To point it at a project:

```sh
cd ~/code/my-project && open -na Keymaker --args .     # or:
KEYMAKER_MANIFEST=~/code/my-project/.keymaker open -na Keymaker
```

## Why `devUrl` is not in `tauri.conf.json`

Tauri uses `devUrl` for **any** debug build, not only for `tauri dev`. With it in
the main config, `cargo build && ./keymaker-gui` opens a window pointed at a
fixed `localhost` port and renders whatever is listening there.

That is not merely a nuisance. This window can call `reveal`, which is the one
command in the project that returns a secret value. Any process on the machine
that binds that port first — another project's dev server, or something
deliberate — would be serving the UI that reads secrets.

It happened during development: a different project had vite on 5173, 5174 and
5175, and the Keymaker window rendered that project's app.

So the main config has no `devUrl`, and every debug build loads the bundled
`frontendDist`. Live reload is opt-in:

```sh
pnpm --dir ../../gui dev &     # 127.0.0.1:5187, strict port
cargo tauri dev --config tauri.dev.conf.json
```

The dev config binds `127.0.0.1` rather than `localhost` (which can resolve to
an IPv6 address a different process holds) and vite runs with `strictPort`, so a
collision fails loudly instead of silently moving to the next port.
