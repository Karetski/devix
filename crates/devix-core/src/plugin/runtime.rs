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

use std::collections::HashSet;

use devix_protocol::protocol::Capability;

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
    /// Capabilities negotiated for this plugin. T-110 warn-and-degrade:
    /// `install_with_manifest` skips contribution kinds whose
    /// capability bit is missing and publishes `Pulse::PluginError`
    /// describing the degradation. Defaults to the host's full set
    /// (`host_capabilities()`); tests / future configurable hosts
    /// pass restricted sets through `load_supervised_with_caps`.
    capabilities: HashSet<Capability>,
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
    /// Restart-aware mirror of `contributions`. The factory
    /// overwrites this on every successful (re)load so consumers
    /// — chiefly [`PluginRuntime::reinstall_panes`] — can see the
    /// current incarnation's pane Arcs after a supervised restart.
    /// The original `contributions` field stays a frozen snapshot
    /// of the first load (existing call sites are stable across
    /// restarts because the Lua entry's registration order is
    /// deterministic — handle N in the new host == handle N in the
    /// dead host).
    live_contributions: Arc<Mutex<Contributions>>,
    /// Manifest name override for `Pulse::PluginLoaded` segment.
    /// `None` → factory falls back to the entry file's stem; once
    /// `install_with_manifest` ran the slot carries
    /// `manifest.name`. Restart-spawn lifecycle pulses then carry
    /// the manifest name so the Application's per-runtime routing
    /// (F-4 follow-up 2026-05-12) can look the runtime up by its
    /// HashMap key. Shared `Arc<Mutex<…>>` so the factory closure
    /// observes the updated value across restarts.
    manifest_name: Arc<Mutex<Option<String>>>,
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
        Self::load_full(path, None, None, None, host_capabilities())
    }

    /// Load with a push-callback. Every emitted [`PluginMsg`] is
    /// handed directly to `sink` from the plugin worker thread;
    /// nothing is buffered on this side. PluginErrors land on a
    /// fresh bus that nothing else listens to — for the editor-bus
    /// path use [`Self::load_supervised`] instead.
    pub fn load_with_sink(path: &Path, sink: MsgSink) -> Result<Self> {
        Self::load_full(path, Some(sink), None, None, host_capabilities())
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
        Self::load_full(path, Some(sink), Some(bus), None, host_capabilities())
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
        Self::load_full(
            path,
            Some(sink),
            Some(bus),
            Some(settings),
            host_capabilities(),
        )
    }

    /// Like [`Self::load_supervised_with_settings`] but pins the
    /// negotiated capability set explicitly. Used by tests that
    /// exercise the warn-and-degrade path; future configurable
    /// hosts can call this with a restricted set to enforce
    /// contribution-level gating. T-110.
    pub fn load_supervised_with_caps(
        path: &Path,
        sink: MsgSink,
        bus: crate::PulseBus,
        settings: Option<Arc<Mutex<SettingsStore>>>,
        capabilities: HashSet<Capability>,
    ) -> Result<Self> {
        Self::load_full(path, Some(sink), Some(bus), settings, capabilities)
    }

    fn load_full(
        path: &Path,
        msg_sink: Option<MsgSink>,
        bus: Option<crate::PulseBus>,
        settings_store: Option<SharedSettingsStore>,
        capabilities: HashSet<Capability>,
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
        let bus = bus.unwrap_or_default();

        // The setting-callbacks list is shared between the host (where
        // `devix.on_setting_changed(cb)` registers handles) and the
        // bus subscriber that pushes per-callback `PluginInput::SettingChanged`
        // when a `Pulse::SettingChanged` arrives. The runtime publishes
        // a clone into the bus subscription closure; the host writes
        // through its own clone during Lua execution.
        let setting_callbacks: Arc<Mutex<Vec<u64>>> = Arc::new(Mutex::new(Vec::new()));

        // Restart-aware Contributions mirror. Filled with the
        // first-spawn snapshot below (after `init_rx.recv()`); the
        // factory writes a fresh value on every successful (re)load
        // so reinstall_panes can see post-restart Arcs.
        let live_contributions: Arc<Mutex<Contributions>> =
            Arc::new(Mutex::new(Contributions::default()));

        // Shared manifest-name slot — populated by
        // `install_with_manifest`. Used by the factory closure (on
        // each spawn, including restarts) to choose between the
        // entry's file-stem and the manifest name when forming the
        // `Pulse::PluginLoaded` path. F-4 follow-up 2026-05-12.
        let manifest_name: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

        let factory_invoke = invoke_tx.clone();
        let factory_input = input_tx.clone();
        let factory_shutdown = shutdown_tx.clone();
        let factory_init = init_tx_slot.clone();
        let factory_msg_tx = msg_tx.clone();
        let factory_msg_sink = msg_sink.clone();
        let factory_path = path.clone();
        let factory_settings = settings_store.clone();
        let factory_live_contributions = live_contributions.clone();
        let factory_bus = bus.clone();
        let factory_plugin_segment = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(sanitize_plugin_segment)
            .unwrap_or_else(|| "plugin".to_string());
        let factory_setting_cbs = setting_callbacks.clone();
        let factory_manifest_name = manifest_name.clone();

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
            let live_contributions_clone = factory_live_contributions.clone();
            let bus_clone = factory_bus.clone();
            let plugin_segment = factory_plugin_segment.clone();
            let factory_manifest_name = factory_manifest_name.clone();
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
                // Mirror the freshly-loaded contributions into the
                // restart-aware shared slot so Application can re-
                // build `LuaPane`s with the new host's Arcs (T-111
                // pane-reinstall on Lua restart).
                if let Ok(mut shared_live) = live_contributions_clone.lock() {
                    *shared_live = contributions.clone();
                }
                forward_messages(&host, &msg_tx_clone, msg_sink_clone.as_ref());
                if let Some(tx) = init_tx {
                    if tx.send(Ok(contributions)).is_err() {
                        return;
                    }
                }
                // Per-spawn lifecycle pulse. Initial spawn fires
                // before main.rs's `install_with_manifest`; each
                // restart fires too so subscribers (Application's
                // typed-pulse dispatcher) re-build pane content from
                // the new host's Arcs.
                //
                // F-4 follow-up 2026-05-12: prefer the manifest
                // name (set by `install_with_manifest`) over the
                // entry file's stem when forming the path, so the
                // pulse routes to the correct runtime in
                // `Application::plugins` after a supervised
                // restart. Initial spawn — before install runs —
                // falls back to file-stem and is harmless: the
                // application hasn't stored the runtime yet, so
                // the lookup misses; `install_with_manifest`
                // installs panes the next moment regardless.
                let segment = factory_manifest_name
                    .lock()
                    .ok()
                    .and_then(|g| g.clone())
                    .map(|n| sanitize_plugin_segment(&n))
                    .unwrap_or_else(|| plugin_segment.clone());
                if let Ok(plugin_path) = devix_protocol::path::Path::parse(&format!(
                    "/plugin/{}",
                    segment,
                )) {
                    let _ = bus_clone.publish_async(devix_protocol::pulse::Pulse::PluginLoaded {
                        plugin: plugin_path,
                        version: "0.0.0".to_string(),
                    });
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

        // Plugin lifecycle pulses fire from the factory itself
        // (per-spawn), so initial + every restart publish a
        // `Pulse::PluginLoaded`. The historical post-init publish
        // (factory → fresh main-thread `bus.publish`) is replaced
        // by the factory's `bus.publish_async` to keep restart
        // semantics symmetric.

        Ok(Self {
            invoke_tx,
            input_tx,
            msg_rx,
            contributions,
            capabilities,
            leaked_strings: Vec::new(),
            shutdown_tx,
            live_contributions,
            manifest_name,
            supervised: Some(supervised),
        })
    }

    /// Manifest name used to form `Pulse::PluginLoaded`'s segment.
    /// `None` until `install_with_manifest` has run. Exposed for
    /// tests; production reads through the pulse payload.
    pub fn manifest_name(&self) -> Option<String> {
        self.manifest_name.lock().ok().and_then(|g| g.clone())
    }

    pub fn contributions(&self) -> &Contributions {
        &self.contributions
    }

    /// Snapshot of the current incarnation's `Contributions` —
    /// includes the post-restart Arcs the active host wrote into.
    /// Use [`Self::reinstall_panes`] to rebuild the editor's
    /// installed panes against this state after a supervised
    /// restart fires `Pulse::PluginLoaded`. T-111 follow-up.
    pub fn current_contributions(&self) -> Contributions {
        self.live_contributions
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// Replace the editor's installed sidebar panes with fresh
    /// `LuaPane` instances built against the *current*
    /// `Contributions` (post-restart Arcs). Idempotent: calling on
    /// initial load reinstalls the same Arcs main.rs's
    /// `install_with_manifest` already mounted; calling after a
    /// supervised restart swaps in panes whose shared state points
    /// at the new host. T-111 follow-up — wired into Application's
    /// `Pulse::PluginLoaded` typed-pulse handler.
    pub fn reinstall_panes(&self, editor: &mut Editor) {
        let pane_specs: Vec<(SidebarSlot, PaneSpec)> = match self.live_contributions.lock() {
            Ok(c) => c.panes.iter().map(|p| (p.slot, p.clone())).collect(),
            Err(_) => return,
        };
        for (slot, spec) in pane_specs {
            let pane = LuaPane::new(
                spec.pane_id,
                spec.lines.clone(),
                spec.scroll.clone(),
                spec.visible_rows.clone(),
                spec.has_on_key.clone(),
                spec.has_on_click.clone(),
                spec.view.clone(),
                self.input_tx.clone(),
            );
            editor.install_sidebar_pane(slot, Box::new(pane));
        }
    }

    /// Negotiated capabilities for this plugin. Read by the editor
    /// when wiring contributions through `install_with_manifest`;
    /// missing bits cause that contribution kind to be skipped with
    /// a `Pulse::PluginError` describing the degradation.
    pub fn capabilities(&self) -> &HashSet<Capability> {
        &self.capabilities
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
                spec.view.clone(),
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

        // F-4 follow-up 2026-05-12: announce the manifest name to
        // the factory closure so subsequent restart spawns publish
        // `Pulse::PluginLoaded` with the manifest-name segment that
        // matches the Application's plugin-map key.
        if let Ok(mut slot) = self.manifest_name.lock() {
            *slot = Some(manifest.name.clone());
        }

        let plugin_path = devix_protocol::path::Path::parse(&format!(
            "/plugin/{}",
            manifest.name
        ))
        .ok();

        // Engines version negotiation per `foundations-review.md`
        // § *Versioning alignment*. Three independently-versioned
        // surfaces (protocol, pulse bus, manifest) — each compared
        // against the host's. Major mismatch is fatal: skip every
        // contribution and publish `Pulse::PluginError`. Minor: the
        // negotiated value is `min(declared_minor, host_minor)`,
        // but enforcement is implicit — features added in higher
        // minors aren't visible to a plugin asking for a lower
        // minor (T-110 keeps capability bits as the visibility
        // gate; this check only refuses majors).
        if !engines_compatible(manifest, &plugin_path, bus) {
            return 0;
        }
        let sender = self.invoke_tx.clone();

        // Capability gate (T-110, warn-and-degrade per `protocol.md` Q2).
        // Each manifest-declared contribution kind needs the host to
        // advertise the matching capability bit; missing bit → publish
        // `Pulse::PluginError` and skip every contribution of that
        // kind.
        let allow_commands = self.capability_allowed(
            Capability::ContributeCommands,
            &plugin_path,
            "contributes.commands",
            !manifest.contributes.commands.is_empty(),
            bus,
        );
        let allow_keymaps = self.capability_allowed(
            Capability::ContributeKeymaps,
            &plugin_path,
            "contributes.keymaps",
            !manifest.contributes.keymaps.is_empty(),
            bus,
        );
        let allow_panes = self.capability_allowed(
            Capability::ContributeSidebarPane,
            &plugin_path,
            "contributes.panes",
            !manifest.contributes.panes.is_empty(),
            bus,
        );

        let mut count = 0usize;
        if !allow_commands {
            // Skip commands entirely; manifest-declared keymaps that
            // resolve through these commands will report unknown-
            // command errors below as a side effect, which is
            // acceptable degraded behaviour.
            return count;
        }
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

        if allow_keymaps {
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
        }

        if !allow_panes {
            return count;
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
                spec.view.clone(),
                self.input_tx.clone(),
            );
            editor.install_sidebar_pane(slot, Box::new(pane));
        }
        // Register `/plugin/<name>/pane/<id>` addressing for every
        // manifest-declared pane that has a matching slot in the
        // runtime's contributions. T-111 follow-up — closes the
        // path-based addressing deferred from the original Stage-11
        // partial. The mapping is keyed (name, id) so multiple panes
        // per plugin (future) just register each entry.
        for decl in &manifest.contributes.panes {
            let core_slot: SidebarSlot = match decl.slot {
                devix_protocol::view::SidebarSlot::Left => SidebarSlot::Left,
                devix_protocol::view::SidebarSlot::Right => SidebarSlot::Right,
            };
            if self.contributions.panes.iter().any(|p| p.slot == core_slot) {
                editor
                    .panes
                    .register_plugin_pane(&manifest.name, &decl.id, core_slot);
            }
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
            view: spec.view.clone(),
            input_tx: self.input_tx.clone(),
        })
    }
}

fn leak_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

impl PluginRuntime {
    /// Capability gate helper: if `needed` is missing from
    /// `self.capabilities` AND the manifest carries this kind, fire
    /// `Pulse::PluginError` once + return false. Otherwise return
    /// true. T-110 warn-and-degrade.
    fn capability_allowed(
        &self,
        needed: Capability,
        plugin_path: &Option<devix_protocol::path::Path>,
        kind: &str,
        manifest_uses_kind: bool,
        bus: &crate::PulseBus,
    ) -> bool {
        if self.capabilities.contains(&needed) {
            return true;
        }
        if !manifest_uses_kind {
            // Manifest doesn't use this kind — silent.
            return true;
        }
        if let Some(pp) = plugin_path {
            bus.publish(devix_protocol::pulse::Pulse::PluginError {
                plugin: pp.clone(),
                message: format!(
                    "host does not advertise `{:?}`; `{}` contributions skipped",
                    needed, kind,
                ),
            });
        }
        false
    }
}

/// Compare a manifest's `engines` block against the host's wire
/// versions. Returns `true` when every surface's major version
/// matches the host's. Major mismatches publish a single
/// `Pulse::PluginError` per offending surface and return `false`
/// so the caller can skip every contribution. Minor mismatches are
/// allowed silently — the negotiated value is `min(declared_minor,
/// host_minor)`, but capability bits gate visibility of new
/// features rather than version numbers.
fn engines_compatible(
    manifest: &devix_protocol::manifest::Manifest,
    plugin_path: &Option<devix_protocol::path::Path>,
    bus: &crate::PulseBus,
) -> bool {
    use devix_protocol::protocol::{
        HOST_MANIFEST_VERSION, HOST_PROTOCOL_VERSION, HOST_PULSE_BUS_VERSION, ProtocolVersion,
    };
    let mut ok = true;
    let mut emit_mismatch = |surface: &str, declared: ProtocolVersion, host: ProtocolVersion| {
        ok = false;
        if let Some(pp) = plugin_path {
            bus.publish(devix_protocol::pulse::Pulse::PluginError {
                plugin: pp.clone(),
                message: format!(
                    "engines.{} major version mismatch: plugin declares {}.{}, host \
                     supports {}.{}",
                    surface, declared.major, declared.minor, host.major, host.minor,
                ),
            });
        }
    };
    if manifest.engines.protocol_version.major != HOST_PROTOCOL_VERSION.major {
        emit_mismatch("devix", manifest.engines.protocol_version, HOST_PROTOCOL_VERSION);
    }
    if manifest.engines.pulse_bus.major != HOST_PULSE_BUS_VERSION.major {
        emit_mismatch("pulse_bus", manifest.engines.pulse_bus, HOST_PULSE_BUS_VERSION);
    }
    if manifest.engines.manifest.major != HOST_MANIFEST_VERSION.major {
        emit_mismatch("manifest", manifest.engines.manifest, HOST_MANIFEST_VERSION);
    }
    ok
}

/// The host's negotiated capability set. Today every bit is set —
/// the warn-and-degrade path is dormant in production. Future
/// configurable hosts (CI runners, restricted environments) will
/// pin a narrower set through `PluginRuntime::load_supervised_with_caps`.
pub fn host_capabilities() -> HashSet<Capability> {
    let mut s = HashSet::new();
    s.insert(Capability::ViewTree);
    s.insert(Capability::StableViewIds);
    s.insert(Capability::UnicodeFull);
    s.insert(Capability::TruecolorStyles);
    s.insert(Capability::Animations);
    s.insert(Capability::ContributeCommands);
    s.insert(Capability::ContributeKeymaps);
    s.insert(Capability::ContributeSidebarPane);
    s.insert(Capability::ContributeOverlayPane);
    s.insert(Capability::ContributeStatusItem);
    s.insert(Capability::ContributeThemes);
    s.insert(Capability::ContributeSettings);
    s.insert(Capability::SubscribePulses);
    s.insert(Capability::InvokeCommands);
    s.insert(Capability::OpenPath);
    s.insert(Capability::ReadDir);
    s
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
