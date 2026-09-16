//! The main Tauri config must not carry a `devUrl`.
//!
//! Tauri uses `devUrl` for *any* debug build, not only `tauri dev`. With one in
//! the main config, `cargo build && ./keymaker-gui` opens a window pointed at a
//! fixed localhost port and renders whatever is listening there.
//!
//! This window can call `reveal`, the one command in the project that returns a
//! secret value — so whoever holds that port controls the UI that reads
//! secrets. It is not hypothetical: during development another project had vite
//! on 5173, 5174 and 5175, and the Keymaker window rendered that project's app.
//!
//! Dev-server settings live in `tauri.dev.conf.json`, which is opt-in.

#[test]
fn the_main_config_has_no_dev_url() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
        .expect("tauri.conf.json");
    let config: serde_json::Value = serde_json::from_str(&src).expect("valid JSON");

    let build = &config["build"];
    assert!(
        build.get("devUrl").is_none(),
        "tauri.conf.json must not set devUrl: a debug build would then render \
         whatever happens to be on that port, in a window that can reveal secrets. \
         Put it in tauri.dev.conf.json instead."
    );
    assert!(
        build.get("beforeDevCommand").is_none(),
        "beforeDevCommand belongs in tauri.dev.conf.json too"
    );
    assert!(
        build["frontendDist"].is_string(),
        "every build must load the bundled frontend"
    );
}

#[test]
fn the_dev_config_binds_a_loopback_address_explicitly() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.dev.conf.json"))
        .expect("tauri.dev.conf.json");
    let config: serde_json::Value = serde_json::from_str(&src).expect("valid JSON");
    let dev_url = config["build"]["devUrl"].as_str().expect("devUrl");

    assert!(
        dev_url.starts_with("http://127.0.0.1:"),
        "the dev server must be pinned to 127.0.0.1, not `localhost`, which can \
         resolve to an address a different process holds: {}",
        dev_url
    );
}
