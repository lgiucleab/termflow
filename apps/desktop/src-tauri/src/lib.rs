//! Tauri glue: commands deserialize, call `termflow-core`, serialize. No business logic here.
#![allow(clippy::needless_pass_by_value)] // Tauri commands must take `tauri::State` by value.

use serde::{Deserialize, Serialize};
use tauri::Manager;
use tauri::ipc::{Channel, InvokeResponseBody, JavaScriptChannelId};
use tauri_specta::{Builder, Event, collect_commands, collect_events};
use termflow_config::Config;
use termflow_core::{App, Paths};
use termflow_terminal::OpenOptions;
use termflow_types::{AgentCli, AppEvent, AppInfo, Block, BlockId, ShellSpec, TerminalId};

/// Path of the generated bridge, relative to this crate.
pub const BINDINGS_PATH: &str = "../ui/src/bridge/generated/bindings.ts";

/// Every core bus event, forwarded to the webview.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct CoreEvent(pub AppEvent);

struct State {
    core: App,
    _log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

type Res<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[tauri::command]
#[specta::specta]
fn app_info(state: tauri::State<'_, State>) -> AppInfo {
    state.core.app_info()
}

#[tauri::command]
#[specta::specta]
fn app_config(state: tauri::State<'_, State>) -> Config {
    state.core.config().clone()
}

#[tauri::command]
#[specta::specta]
fn list_shells(state: tauri::State<'_, State>) -> Vec<ShellSpec> {
    state.core.terminals().shells()
}

#[tauri::command]
#[specta::specta]
fn list_agents(state: tauri::State<'_, State>) -> Vec<AgentCli> {
    state.core.terminals().agents()
}

#[derive(Deserialize, specta::Type)]
pub struct OpenTerminal {
    /// Shell id from `list_shells`; unset = configured or default shell.
    shell_id: Option<String>,
    cwd: Option<String>,
    cols: u16,
    rows: u16,
    /// Agent CLI id from `list_agents`, started in the new shell.
    agent_id: Option<String>,
    /// `Channel.toJSON()` of a channel receiving raw output bytes (`ArrayBuffer`).
    #[specta(type = String)]
    on_output: JavaScriptChannelId,
}

#[tauri::command]
#[specta::specta]
async fn terminal_open(
    req: OpenTerminal,
    state: tauri::State<'_, State>,
    webview: tauri::Webview<tauri::Wry>,
) -> Res<TerminalId> {
    let t = state.core.terminals();
    let shells = t.shells();
    let wanted = req.shell_id.as_ref().or(state.core.config().terminal.shell.as_ref());
    let shell = wanted
        .and_then(|id| shells.iter().find(|s| &s.id == id))
        .or(shells.first())
        .cloned()
        .unwrap_or_else(termflow_pty_default_shell);
    let mut initial_input = req.agent_id.map(|a| format!("{a}\r"));
    // Dev-only harness for perf/rendering checks: TERMFLOW_DEV_AUTORUN="yes | head -n 2000000".
    #[cfg(debug_assertions)]
    if initial_input.is_none()
        && let Ok(cmd) = std::env::var("TERMFLOW_DEV_AUTORUN")
    {
        initial_input = Some(format!("{cmd}\r"));
    }
    let channel: Channel<InvokeResponseBody> = req.on_output.channel_on(webview);
    t.open(
        &OpenOptions {
            shell,
            cwd: req.cwd.map(Into::into),
            cols: req.cols,
            rows: req.rows,
            initial_input,
        },
        Box::new(move |bytes| channel.send(InvokeResponseBody::Raw(bytes)).is_ok()),
    )
    .map_err(err)
}

fn termflow_pty_default_shell() -> ShellSpec {
    ShellSpec {
        id: "sh".into(),
        name: "sh".into(),
        program: if cfg!(windows) {
            "cmd.exe".into()
        } else {
            "/bin/sh".into()
        },
        args: vec![],
        kind: termflow_types::ShellKind::Other,
    }
}

#[tauri::command]
#[specta::specta]
fn terminal_write(id: TerminalId, data: String, state: tauri::State<'_, State>) -> Res<()> {
    state.core.terminals().write(id, data.as_bytes()).map_err(err)
}

#[tauri::command]
#[specta::specta]
fn terminal_resize(id: TerminalId, cols: u16, rows: u16, state: tauri::State<'_, State>) -> Res<()> {
    state.core.terminals().resize(id, cols, rows).map_err(err)
}

#[tauri::command]
#[specta::specta]
fn terminal_set_paused(id: TerminalId, paused: bool, state: tauri::State<'_, State>) -> Res<()> {
    state.core.terminals().set_paused(id, paused).map_err(err)
}

#[tauri::command]
#[specta::specta]
fn terminal_close(id: TerminalId, state: tauri::State<'_, State>) -> Res<()> {
    state.core.terminals().close(id).map_err(err)
}

#[derive(Debug, Serialize, specta::Type)]
pub struct BlockOutput {
    block: Block,
    output: String,
}

#[tauri::command]
#[specta::specta]
fn block_output(id: BlockId, state: tauri::State<'_, State>) -> Res<Option<BlockOutput>> {
    let found = state.core.terminals().block_output(id).map_err(err)?;
    Ok(found.map(|(block, output)| BlockOutput { block, output }))
}

#[tauri::command]
#[specta::specta]
fn recent_blocks(limit: u32, state: tauri::State<'_, State>) -> Res<Vec<Block>> {
    state.core.terminals().recent_blocks(limit).map_err(err)
}

#[tauri::command]
#[specta::specta]
fn setting_get(key: String, state: tauri::State<'_, State>) -> Res<Option<String>> {
    state.core.setting(&key).map_err(err)
}

#[tauri::command]
#[specta::specta]
async fn setting_set(key: String, value_json: String, state: tauri::State<'_, State>) -> Res<()> {
    state.core.set_setting(key, value_json).await.map_err(err)
}

/// Debug builds: the UI's frame-time monitor reports here so results land in the log.
#[tauri::command]
#[specta::specta]
fn perf_report(report: String) {
    tracing::info!(target: "perf", %report);
}

pub fn specta_builder() -> Builder<tauri::Wry> {
    Builder::<tauri::Wry>::new()
        .commands(collect_commands![
            app_info,
            app_config,
            list_shells,
            list_agents,
            terminal_open,
            terminal_write,
            terminal_resize,
            terminal_set_paused,
            terminal_close,
            block_output,
            recent_blocks,
            setting_get,
            setting_set,
            perf_report,
        ])
        .events(collect_events![CoreEvent])
}

pub fn export_bindings(builder: &Builder<tauri::Wry>) -> anyhow::Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(BINDINGS_PATH);
    let tmp = path.with_extension("ts.tmp");
    builder.export(
        specta_typescript::Typescript::default().header("// @ts-nocheck\n// Generated by tauri-specta. Do not edit.\n"),
        &tmp,
    )?;
    // Only touch the real file when it changed, so dev mode doesn't reload the UI on every launch.
    if std::fs::read(&tmp)? == std::fs::read(&path).unwrap_or_default() {
        std::fs::remove_file(&tmp)?;
    } else {
        std::fs::rename(&tmp, &path)?;
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> anyhow::Result<()> {
    let builder = specta_builder();
    #[cfg(debug_assertions)]
    export_bindings(&builder)?;

    let paths = Paths::os_default()?;
    let redactor = termflow_secrets::Redactor::new();
    let log_guard = termflow_core::logging::init(&paths.log_dir, redactor.clone());
    let core = App::new(&paths, redactor)?;

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            let mut rx = core.subscribe();
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use tokio::sync::broadcast::error::RecvError;
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            if let Err(e) = CoreEvent(ev).emit(&handle) {
                                tracing::warn!(error = %e, "emit to webview failed");
                            }
                        }
                        Err(RecvError::Lagged(n)) => tracing::warn!(n, "ui event forwarder lagged"),
                        Err(RecvError::Closed) => break,
                    }
                }
            });
            app.manage(State {
                core,
                _log_guard: log_guard,
            });
            Ok(())
        })
        .run(tauri::generate_context!())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Regenerates the TS bridge; CI fails if the checked-in file differs (`git diff --exit-code`).
    #[test]
    fn export_bindings() {
        super::export_bindings(&super::specta_builder()).unwrap();
    }
}
