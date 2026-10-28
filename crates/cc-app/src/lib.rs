//! cc-app command layer (ticket 12): the thin Tauri shell over cc-core. Every
//! command is `*_impl(data_dir, ...)` pure function returning the same structured
//! JSON shape as the matching `cc-cli --json` subcommand — UI 走的路和 CLI/AI 走的
//! 是同一条 cc-core 路. The `#[tauri::command]` wrappers in main.rs only unpack
//! arguments and inject the app data dir; tests drive the impl functions directly.
//! Ticket 13 adds pipeline.rs: wizard steps 4–6 (voice / person matte / generate)
//! with progress events.

pub mod commands;
pub mod pipeline;
