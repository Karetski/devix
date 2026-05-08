//! Supervised plugin runtime — owns a worker thread that hosts a
//! `PluginHost` plus the channel topology that survives restart.
//!
//! ## Restart with channel refresh (T-81)
//!
//! Editor-held senders ([`super::InvokeSender`], [`super::InputSender`])
//! are `Arc<Mutex<UnboundedSender<…>>>`. Each spawn:
//!
//! 1. Creates a fresh `(invoke_tx, invoke_rx)` and `(input_tx, input_rx)`
//!    pair locally.
//! 2. Locks the editor-held `Arc` and replaces its inner sender with
//!    the fresh one.
//! 3. Uses the fresh receivers in the worker's `tokio::select!` loop.
//!
//! Editor-side captures (`LuaAction.sender`, `LuaPane.input_tx`) hold
//! the `Arc` directly, so they auto-pick-up the new sender on restart
//! without recompiling the closure.
//!
//! Erlang principle: let it die, restart clean. The new `PluginHost`
//! re-runs the Lua entry script; the `next_handle` counter starts
//! fresh. Because Lua entry registration is deterministic (no
//! environment-dependent state), action handle 1 in the restarted
//! host names the same Lua function as handle 1 in the dead host —
//! the editor's `PluginCommandAction(handle=1, …)` keeps working
//! transparently.
//!
//! Limitations of v0 restart support:
//!
//! - Pane line content set during the dead host's lifetime persists
//!   in the shared `Arc<Mutex<Vec<String>>>` until the new host
//!   overwrites it. The new host's `register_pane` rewires the
//!   `Arc`s by writing fresh state into freshly-allocated `Arc`s —
//!   meaning the editor's installed `LuaPane` still points at the
//!   *old* `Arc`s and won't reflect the new host's mutations until
//!   reinstall happens. Plugin contribution re-registration is the
//!   next sprint's concern; this commit ships the topology.
//! - `register_pane` callbacks registered during the dead host do
//!   not survive: the new host's `Arc<Mutex<HashMap<u64, …>>>` of
//!   `callbacks` is fresh.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use tokio::sync::mpsc::{
    UnboundedReceiver, UnboundedSender, error::TryRecvError, unbounded_channel,
};
use tokio::sync::oneshot;

use crate::editor::{Command, CommandId, CommandRegistry, Editor, Keymap};
use crate::settings_store::SettingsStore;
use crate::SidebarSlot;

use super::bridge::make_command_action;
use super::host::{PluginHost, SharedSettingsStore};
use super::pane_handle::LuaPane;
use super::{
    Contributions, InputSender, InvokeSender, PaneSpec, PluginInput, PluginMsg,
    sanitize_plugin_segment, send_input,
};

/// Push-callback for plugin messages. Production callers pass one of
/// these to [`PluginRuntime::load_with_sink`]; the worker thread
/// invokes it directly for every emitted [`PluginMsg`], so the
/// editor's run loop never has to drain a queue. T-63 retired the
/// prior `Wakeup` hook — the MsgSink itself is the wake mechanism.
pub type MsgSink = Arc<dyn Fn(PluginMsg) + Send + Sync + 'static>;

/// Plugin runtime: owns the host on a dedicated thread and exposes
/// channel handles the editor uses to dispatch invokes / forward
/// input / drain status.
pub struct PluginRuntime {
    invoke_tx: InvokeSender,
    input_tx: InputSender,
    msg_rx: UnboundedReceiver<PluginMsg>,
    contributions: Contributions,
    /// Strings leaked to satisfy the `'static` lifetime on
    /// `CommandId(&'static str)` / `Command::label`. Lives as long
    /// as the runtime so registered commands stay valid.
    #[allow(dead_code)]
    leaked_strings: Vec<&'static str>,
    /// Active shutdown sender. The factory replaces this on each
    /// spawn; `Drop` takes from it to signal the *current* worker
    /// loop to exit cleanly. Senders held by the editor (e.g.,
    /// installed pane keeps `input_tx`) prevent the loop's
    /// `tokio::select!` from observing channel close, so an explicit
    /// signal is required.
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Supervised plugin worker thread. On panic the supervisor's
    /// restart policy decides whether to respawn (channel refresh
    /// happens inside the factory closure); on shutdown (drop sends
    /// `shutdown_tx`) the factory returns cleanly and the supervisor
    /// exits. Held only to keep the supervised thread alive for the
    /// runtime's lifetime.
    #[allow(dead_code)]
    supervised: Option<crate::supervise::SupervisedChild>,
}

impl Drop for PluginRuntime {
    fn drop(&mut self) {
        // Signal the active supervised loop to exit before any other
        // field drops run. Once the factory returns clean,
        // `SupervisedChild::drop` joins the supervisor thread
        // immediately. Without this signal the editor-held `input_tx`
        // clone would keep the select! alive forever and the join
        // would hang.
        if let Ok(mut slot) = self.shutdown_tx.lock() {
            if let Some(tx) = slot.take() {
                let _ = tx.send(());
            }
        }
    }
}

impl PluginRuntime {
    /// Load without a push-sink. Messages buffer on an internal
    /// queue; consumers drain via [`PluginRuntime::drain_messages`].
    /// Kept for tests; production uses [`Self::load_supervised`].
    pub fn load(path: &Path) -> Result<Self> {
        Self::load_full(path, None, None, None)
    }

    /// Load with a push-callback. Every emitted [`PluginMsg`] is
    /// handed directly to `sink` from the plugin worker thread;
    /// nothing is buffered on this side. PluginErrors land on a
    /// fresh bus that nothing else listens to — for the editor-bus
    /// path use [`Self::load_supervised`] instead.
    pub fn load_with_sink(path: &Path, sink: MsgSink) -> Result<Self> {
        Self::load_full(path, Some(sink), None, None)
    }

    /// Load with a push-callback and a bus to publish lifecycle pulses
    /// (`Pulse::PluginLoaded` / `Pulse::PluginError`) on. The
    /// supervisor wraps the plugin worker thread on this bus so a
    /// Lua-side panic escalates as `PluginError` and the supervisor
    /// respawns up to `RestartPolicy::max_restarts` times before
    /// escalating permanently.
    pub fn load_supervised(
        path: &Path,
        sink: MsgSink,
        bus: crate::PulseBus,
    ) -> Result<Self> {
        Self::load_full(path, Some(sink), Some(bus), None)
    }

    /// Like [`Self::load_supervised`] but also threads a shared
    /// `SettingsStore` into the plugin host so the Lua bridge's
    /// `devix.setting(key)` reads + `devix.on_setting_changed(cb)`
    /// observers see the editor's settings state. T-113 full close.
    pub fn load_supervised_with_settings(
        path: &Path,
        sink: MsgSink,
        bus: crate::PulseBus,
        settings: Arc<Mutex<SettingsStore>>,
    ) -> Result<Self> {
        Self::load_full(path, Some(sink), Some(bus), Some(settings))
    }

    fn load_full(
        path: &Path,
        msg_sink: Option<MsgSink>,
        bus: Option<crate::PulseBus>,
        settings_store: Option<SharedSettingsStore>,
    ) -> Result<Self> {
        let (msg_tx, msg_rx) = unbounded_channel::<PluginMsg>();

        // Initial sender placeholders — the factory immediately
        // replaces these on its first spawn. We construct dummy
        // closed channels so the `Arc<Mutex<…>>` are well-formed
        // before the factory has run.
        let (placeholder_invoke_tx, _) = unbounded_channel::<u64>();
        let (placeholder_input_tx, _) = unbounded_channel::<PluginInput>();
        let invoke_tx: InvokeSender = Arc::new(Mutex::new(placeholder_invoke_tx));
        let input_tx: InputSender = Arc::new(Mutex::new(placeholder_input_tx));
        let shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>> =
            Arc::new(Mutex::new(None));

        let (init_tx, init_rx) = std::sync::mpsc::channel::<Result<Contributions>>();
        let init_tx_slot: Arc<Mutex<Option<std::sync::mpsc::Sender<Result<Contributions>>>>> =
            Arc::new(Mutex::new(Some(init_tx)));

        let path = Arc::new(path.to_path_buf());
        // Bus the supervisor escalates on. When the caller didn't
        // pass one, use a fresh local bus — `Pulse::PluginError`
        // then falls on the floor instead of bubbling into the
        // editor (acceptable for tests; production uses
        // `load_supervised`).
        let bus = bus.unwrap_or_else(crate::PulseBus::new);

        // The setting-callbacks list is shared between the host (where
        // `devix.on_setting_changed(cb)` registers handles) and the
        // bus subscriber that pushes per-callback `PluginInput::SettingChanged`
        // when a `Pulse::SettingChanged` arrives. The runtime publishes
        // a clone into the bus subscription closure; the host writes
        // through its own clone during Lua execution.
        let setting_callbacks: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));

        let factory_invoke = invoke_tx.clone();
        let factory_input = input_tx.clone();
        let factory_shutdown = shutdown_tx.clone();
        let factory_init = init_tx_slot.clone();
        let factory_msg_tx = msg_tx.clone();
        let factory_msg_sink = msg_sink.clone();
        let factory_path = path.clone();
        let factory_settings = settings_store.clone();
        let factory_setting_cbs = setting_callbacks.clone();

        let factory = move || {
            // Per-spawn channels. The fresh receivers stay local to
            // the worker; the senders replace the editor-held
            // `Arc<Mutex<…>>` so existing `LuaAction` / `LuaPane`
            // captures auto-route to this incarnation of the worker.
            let (invoke_tx_local, invoke_rx) = unbounded_channel::<u64>();
            let (input_tx_local, input_rx) = unbounded_channel::<PluginInput>();
            if let Ok(mut slot) = factory_invoke.lock() {
                *slot = invoke_tx_local;
            }
            if let Ok(mut slot) = factory_input.lock() {
                *slot = input_tx_local;
            }
            let (shutdown_tx_local, shutdown_rx) = oneshot::channel::<()>();
            if let Ok(mut slot) = factory_shutdown.lock() {
                *slot = Some(shutdown_tx_local);
            }

            let path_owned: PathBuf = (*factory_path).clone();
            let init_tx = factory_init.lock().ok().and_then(|mut s| s.take());
            let msg_tx_clone = factory_msg_tx.clone();
            let msg_sink_clone = factory_msg_sink.clone();

            let runtime = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    if let Some(tx) = init_tx {
                        let _ = tx.send(Err(anyhow!(e)));
                    }
                    return;
                }
            };
            let settings_for_host = factory_settings.clone();
            let host_setting_cbs = factory_setting_cbs.clone();
            runtime.block_on(async move {
                let host = match PluginHost::new_with(settings_for_host) {
                    Ok(h) => h,
                    Err(e) => {
                        if let Some(tx) = init_tx {
                            let _ = tx.send(Err(e));
                        }
                        return;
                    }
                };
                let contributions = match host.load_file(&path_owned) {
                    Ok(c) => c,
                    Err(e) => {
                        if let Some(tx) = init_tx {
                            let _ = tx.send(Err(e));
                        }
                        return;
                    }
                };
                // Mirror the host's freshly-registered setting-changed
                // callbacks into the runtime-level shared list so the
                // bus subscriber sees them.
                if let (Ok(mut shared), Ok(host_cbs)) = (
                    host_setting_cbs.lock(),
                    host.setting_callbacks().lock(),
                ) {
                    *shared = host_cbs.clone();
                }
                forward_messages(&host, &msg_tx_clone, msg_sink_clone.as_ref());
                if let Some(tx) = init_tx {
                    if tx.send(Ok(contributions)).is_err() {
                        return;
                    }
                }
                // After the first spawn's init delivery, subsequent
                // restarts discard the fresh `Contributions`. The
                // editor's existing handle-keyed actions resolve into
                // the new host because Lua entry registration is
                // deterministic (handle 1 today == handle 1 next
                // restart). Auto re-registration into the editor is
                // the next sprint's concern.
                let mut shutdown_rx = shutdown_rx;
                let mut invoke_rx = invoke_rx;
                let mut input_rx = input_rx;
                loop {
                    tokio::select! {
                        _ = &mut shutdown_rx => break,
                        maybe_handle = invoke_rx.recv() => {
                            match maybe_handle {
                                Some(handle) => host.invoke(handle),
                                None => break,
                            }
                        }
                        maybe_input = input_rx.recv() => {
                            match maybe_input {
                                Some(input) => host.dispatch_input(input),
                                None => break,
                            }
                        }
                    }
                    forward_messages(&host, &msg_tx_clone, msg_sink_clone.as_ref());
                }
            });
        };

        // Restart budget: 3 panics within 30s before the supervisor
        // gives up and escalates with `Pulse::PluginError`. Aligns
        // with `supervise::RestartPolicy::default()`.
        let policy = crate::supervise::RestartPolicy {
            max_restarts: 3,
            window: Duration::from_secs(30),
        };
        let supervised = crate::supervise::supervise(
            "plugin",
            bus.clone(),
            policy,
            factory,
        )
        .context("spawning supervised plugin host thread")?;

        let contributions = init_rx
            .recv()
            .context("plugin host thread exited before reporting load result")??;

        // Bus subscription: when `Pulse::SettingChanged` fires, fan
        // out one `PluginInput::SettingChanged` per registered Lua
        // callback through the (channel-refresh-aware) input sender.
        // The subscriber runs on whichever thread published the
        // pulse; it never touches Lua directly. T-113.
        {
            use devix_protocol::pulse::{PulseFilter, PulseKind};
            let cb_handles = setting_callbacks.clone();
            let input_sender = input_tx.clone();
            bus.subscribe(
                PulseFilter::kind(PulseKind::SettingChanged),
                move |pulse| {
                    if let devix_protocol::pulse::Pulse::SettingChanged { setting, value } =
                        pulse
                    {
                        let key = setting_path_to_key(setting);
                        let handles: Vec<u64> = cb_handles
                            .lock()
                            .ok()
                            .map(|h| h.clone())
                            .unwrap_or_default();
                        for handle in handles {
                            let _ = send_input(
                                &input_sender,
                                PluginInput::SettingChanged {
                                    handle,
                                    key: key.clone(),
                                    value: value.clone(),
                                },
                            );
                        }
                    }
                },
            );
        }

        // Plugin lifecycle: announce the load on the bus the caller
        // cares about. Plugin name is taken from the source file's
        // stem; production uses the manifest's name when the
        // manifest-driven loader takes over.
        let plugin_segment = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(sanitize_plugin_segment)
            .unwrap_or_else(|| "plugin".to_string());
        if let Ok(plugin_path) =
            devix_protocol::path::Path::parse(&format!("/plugin/{}", plugin_segment))
        {
            bus.publish(devix_protocol::pulse::Pulse::PluginLoaded {
                plugin: plugin_path,
                version: "0.0.0".to_string(),
            });
        }

        Ok(Self {
            invoke_tx,
            input_tx,
            msg_rx,
            contributions,
            leaked_strings: Vec::new(),
            shutdown_tx,
            supervised: Some(supervised),
        })
    }

    pub fn contributions(&self) -> &Contributions {
        &self.contributions
    }

    pub fn invoke_sender(&self) -> InvokeSender {
        self.invoke_tx.clone()
    }

    pub fn input_sender(&self) -> InputSender {
        self.input_tx.clone()
    }

    /// Drain any messages currently buffered. Non-blocking.
    pub fn drain_messages(&mut self) -> Vec<PluginMsg> {
        let mut out = Vec::new();
        loop {
            match self.msg_rx.try_recv() {
                Ok(m) => out.push(m),
                Err(TryRecvError::Empty) => return out,
                Err(TryRecvError::Disconnected) => return out,
            }
        }
    }

    /// Wire this runtime's contributions into the editor:
    /// - register every contributed command in `commands`,
    /// - bind every contributed chord in `keymap`,
    /// - install every contributed pane onto its sidebar slot in
    ///   `editor`.
    pub fn install(
        &mut self,
        commands: &mut CommandRegistry,
        keymap: &mut Keymap,
        editor: &mut Editor,
    ) {
        let sender = self.invoke_tx.clone();
        for spec in &self.contributions.commands {
            let id_static: &'static str = leak_str(&spec.id);
            let label_static: &'static str = leak_str(&spec.label);
            self.leaked_strings.push(id_static);
            self.leaked_strings.push(label_static);

            let id = CommandId::builtin(id_static);
            let action = make_command_action(spec, sender.clone());
            commands.register(Command {
                id,
                label: label_static,
                category: Some("Plugin"),
                action,
            });
            if let Some(chord) = spec.chord {
                keymap.bind_command(chord, id);
            }
        }
        let pane_specs: Vec<(SidebarSlot, PaneSpec)> = self
            .contributions
            .panes
            .iter()
            .map(|p| (p.slot, p.clone()))
            .collect();
        for (slot, spec) in pane_specs {
            let pane = LuaPane::new(
                spec.pane_id,
                spec.lines.clone(),
                spec.scroll.clone(),
                spec.visible_rows.clone(),
                spec.has_on_key.clone(),
                spec.has_on_click.clone(),
                self.input_tx.clone(),
            );
            editor.install_sidebar_pane(slot, Box::new(pane));
        }
    }

    /// Install this runtime's contributions into the editor under
    /// the manifest's plugin namespace (T-110).
    pub fn install_with_manifest(
        &mut self,
        commands: &mut CommandRegistry,
        keymap: &mut Keymap,
        editor: &mut Editor,
        manifest: &devix_protocol::manifest::Manifest,
        bus: &crate::PulseBus,
    ) -> usize {
        let plugin_name: &'static str = leak_str(&manifest.name);
        self.leaked_strings.push(plugin_name);

        let plugin_path = devix_protocol::path::Path::parse(&format!(
            "/plugin/{}",
            manifest.name
        ))
        .ok();
        let sender = self.invoke_tx.clone();

        let mut count = 0usize;
        for decl in &manifest.contributes.commands {
            let runtime_spec = self
                .contributions
                .commands
                .iter()
                .find(|c| c.id == decl.id);
            let Some(runtime_spec) = runtime_spec else {
                if let Some(ref pp) = plugin_path {
                    bus.publish(devix_protocol::pulse::Pulse::PluginError {
                        plugin: pp.clone(),
                        message: format!(
                            "manifest declares command `{}` but the plugin's Lua \
                             entry never registered a handler with that id",
                            decl.id,
                        ),
                    });
                }
                continue;
            };

            let id_static: &'static str = leak_str(&decl.id);
            let label_static: &'static str = leak_str(&decl.label);
            self.leaked_strings.push(id_static);
            self.leaked_strings.push(label_static);

            let id = CommandId::plugin(plugin_name, id_static);
            let action = make_command_action(runtime_spec, sender.clone());
            commands.register(Command {
                id,
                label: label_static,
                category: Some("Plugin"),
                action,
            });
            if let Some(chord) = runtime_spec.chord {
                if !keymap.bind_command_if_free(chord, id) {
                    if let Some(ref pp) = plugin_path {
                        bus.publish(devix_protocol::pulse::Pulse::PluginError {
                            plugin: pp.clone(),
                            message: format!(
                                "chord conflict for command `{}`: chord already \
                                 bound by an earlier plugin or built-in",
                                decl.id,
                            ),
                        });
                    }
                }
            }
            count += 1;
        }

        match crate::manifest_loader::register_keymap_contributions_with_policy(
            keymap,
            manifest,
            commands,
            crate::manifest_loader::BindPolicy::IfFree,
        ) {
            Ok(_) => {}
            Err(e) => {
                if let Some(ref pp) = plugin_path {
                    bus.publish(devix_protocol::pulse::Pulse::PluginError {
                        plugin: pp.clone(),
                        message: format!("manifest keymap registration failed: {e}"),
                    });
                }
            }
        }

        for decl in &manifest.contributes.panes {
            let core_slot: SidebarSlot = match decl.slot {
                devix_protocol::view::SidebarSlot::Left => SidebarSlot::Left,
                devix_protocol::view::SidebarSlot::Right => SidebarSlot::Right,
            };
            let registered =
                self.contributions.panes.iter().any(|p| p.slot == core_slot);
            if !registered {
                if let Some(ref pp) = plugin_path {
                    bus.publish(devix_protocol::pulse::Pulse::PluginError {
                        plugin: pp.clone(),
                        message: format!(
                            "manifest declares pane `{}` on slot `{:?}` but the \
                             plugin's Lua entry never called `register_pane` \
                             for that slot",
                            decl.id, decl.slot,
                        ),
                    });
                }
            }
        }
        let pane_specs: Vec<(SidebarSlot, PaneSpec)> = self
            .contributions
            .panes
            .iter()
            .map(|p| (p.slot, p.clone()))
            .collect();
        for (slot, spec) in pane_specs {
            let pane = LuaPane::new(
                spec.pane_id,
                spec.lines.clone(),
                spec.scroll.clone(),
                spec.visible_rows.clone(),
                spec.has_on_key.clone(),
                spec.has_on_click.clone(),
                self.input_tx.clone(),
            );
            editor.install_sidebar_pane(slot, Box::new(pane));
        }
        count
    }

    /// Pane handle for `slot`, if the plugin contributed one.
    pub fn pane_for(&self, slot: SidebarSlot) -> Option<super::PluginPane> {
        let spec = self
            .contributions
            .panes
            .iter()
            .find(|p| p.slot == slot)?;
        Some(super::PluginPane {
            pane_id: spec.pane_id,
            lines: spec.lines.clone(),
            scroll: spec.scroll.clone(),
            visible_rows: spec.visible_rows.clone(),
            has_on_key: spec.has_on_key.clone(),
            has_on_click: spec.has_on_click.clone(),
            input_tx: self.input_tx.clone(),
        })
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Decode a `/setting/<key>` path back to the dotted key. Returns
/// the path's last segment (best-effort); paths shaped differently
/// fall back to the full string form.
fn setting_path_to_key(path: &devix_protocol::path::Path) -> String {
    let mut segs = path.segments();
    if segs.next() == Some("setting") {
        if let Some(key) = segs.next() {
            if segs.next().is_none() {
                return key.to_string();
            }
        }
    }
    path.as_str().to_string()
}

fn forward_messages(
    host: &PluginHost,
    msg_tx: &UnboundedSender<PluginMsg>,
    msg_sink: Option<&MsgSink>,
) {
    for msg in host.drain_messages() {
        match msg_sink {
            Some(sink) => sink(msg),
            None => {
                if msg_tx.send(msg).is_err() {
                    break;
                }
            }
        }
    }
}
