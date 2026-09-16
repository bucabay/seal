// The GUI has no console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! The human surface.
//!
//! Everything here is a thin wrapper over `keymaker_core::gui`, which holds the
//! logic and the tests. This file exists to turn those into Tauri commands.
//!
//! Each command builds its own store, manifest and audit log rather than
//! holding them open. A GUI is not a hot path, and per-command construction
//! means a secret saved from the CLI shows up here without a restart.

use keymaker_core::audit::Log;
use keymaker_core::clock::SystemClock;
use keymaker_core::gui::{AuditRow, EndpointRow, Gui, Health, RefRow, TaskRow};
use keymaker_core::manifest::Manifest;
use keymaker_core::provider::Catalog;
use keymaker_core::runner::{ProcessSpawner, Runner};
use keymaker_core::store::{platform_store, SecretStore};
use serde::Serialize;
use std::path::PathBuf;

/// `SystemClock` holds nothing, so one can live for the whole program and be
/// borrowed by every audit log.
static CLOCK: SystemClock = SystemClock;

fn config_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    if cfg!(target_os = "macos") {
        PathBuf::from(home).join("Library/Application Support/keymaker")
    } else {
        PathBuf::from(home).join(".config/keymaker")
    }
}

fn manifest_path() -> PathBuf {
    std::env::var("KEYMAKER_MANIFEST")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(".keymaker"))
}

fn load_manifest() -> Manifest {
    std::fs::read_to_string(manifest_path())
        .ok()
        .and_then(|src| Manifest::from_toml(&src).ok())
        .unwrap_or_else(|| Manifest::from_toml("version = 1").expect("empty manifest is valid"))
}

fn load_catalog() -> Catalog {
    for p in [
        PathBuf::from(".keymaker.endpoints.toml"),
        config_dir().join("endpoints.toml"),
    ] {
        if let Ok(src) = std::fs::read_to_string(&p) {
            if let Ok(c) = Catalog::from_toml(&src) {
                return c;
            }
        }
    }
    Catalog::default()
}

/// Build everything a command needs, run `body`, and hand back the result.
fn with_gui<T>(body: impl FnOnce(&mut Gui) -> Result<T, String>) -> Result<T, String> {
    let mut store: Box<dyn SecretStore> = platform_store();
    let audit_path = config_dir().join("audit.jsonl");
    let audit = Log::open(&CLOCK, &audit_path).map_err(|e| e.to_string())?;
    let mut gui = Gui::new(store.as_mut(), load_manifest(), load_catalog(), audit);
    body(&mut gui)
}

#[tauri::command]
fn list_refs() -> Result<Vec<RefRow>, String> {
    with_gui(|g| Ok(g.refs()))
}

/// The one place in the whole project that returns a value.
///
/// It is reachable only from a window a person is looking at, and it is
/// recorded in the same audit chain as everything else.
#[tauri::command]
fn reveal(reference: String) -> Result<String, String> {
    with_gui(|g| g.reveal(&reference).map_err(|e| e.to_string()))
}

#[tauri::command]
fn save_secret(reference: String, value: String) -> Result<(), String> {
    with_gui(|g| g.save(&reference, &value).map_err(|e| e.to_string()))
}

#[tauri::command]
fn delete_secret(reference: String) -> Result<(), String> {
    with_gui(|g| g.delete(&reference).map_err(|e| e.to_string()))
}

#[tauri::command]
fn list_tasks() -> Result<Vec<TaskRow>, String> {
    with_gui(|g| Ok(g.tasks()))
}

#[tauri::command]
fn list_endpoints() -> Result<Vec<EndpointRow>, String> {
    with_gui(|g| Ok(g.endpoints()))
}

#[tauri::command]
fn list_environments() -> Result<Vec<String>, String> {
    with_gui(|g| Ok(g.environments()))
}

#[tauri::command]
fn audit_rows() -> Result<Vec<AuditRow>, String> {
    with_gui(|g| Ok(g.audit_rows()))
}

#[tauri::command]
fn health() -> Result<Health, String> {
    with_gui(|g| Ok(g.health()))
}

#[derive(Serialize)]
struct RunResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    redacted: bool,
    leaked_files: Vec<String>,
}

/// Run a task from the GUI. Output comes back redacted, exactly as it would
/// for an agent — a person watching a task run has no more need to see the
/// credential than the agent does.
#[tauri::command]
fn run_task(task: String, env: String) -> Result<RunResult, String> {
    let store = platform_store();
    let spawner = ProcessSpawner;
    let manifest = load_manifest();
    let runner = Runner::new(store.as_ref(), &spawner).watching_defaults();
    let out = runner
        .run_task(&manifest, &task, &env)
        .map_err(|e| e.to_string())?;
    Ok(RunResult {
        exit_code: out.exit_code,
        stdout: out.stdout_string(),
        stderr: out.stderr_string(),
        redacted: out.redacted,
        leaked_files: out
            .files_with_values
            .iter()
            .map(|p| p.display().to_string())
            .collect(),
    })
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            list_refs,
            reveal,
            save_secret,
            delete_secret,
            list_tasks,
            list_endpoints,
            list_environments,
            audit_rows,
            health,
            run_task,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start keymaker");
}
