//! Lane-message types — `docs/specs/protocol.md`.
//!
//! Three lanes share these envelopes today:
//! 1. **Client ↔ Core** — TUI today; GUI / mobile / web later.
//! 2. **Plugin ↔ Core** — Lua plugins today.
//! 3. **Core ↔ Core internal** — supervised actors. No envelope at v0
//!    per protocol.md *Lane 3*: actors communicate via the in-process
//!    `PulseBus` plus direct calls into their supervisor; an
//!    envelope-bound mailbox is added only when an actor needs control
//!    messages beyond what pulses carry.
//!
//! Capability mismatch policy (resolved 2026-05-07): warn-and-degrade
//! with plugin opt-out (VS Code style). Unsupported contributions
//! silently no-op with a `Pulse::PluginError` warning; the plugin can
//! inspect the negotiated set on `Welcome` and refuse to run if it
//! requires what's missing. Concrete enforcement lands in T-110.

use std::collections::HashSet;
use std::path::PathBuf;

use serde::de::{self, Deserializer, Visitor};
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::path::Path;
use crate::pulse::{Pulse, PulseFilter, SubscriptionId};
use crate::view::View;

// -- Versioning --------------------------------------------------------------

/// Versioned wrapper around any lane payload. `seq` is monotonic per
/// producer and used for request/response correlation, ordered
/// delivery, and log correlation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub protocol_version: ProtocolVersion,
    pub seq: u64,
    pub payload: T,
}

/// Versioned semver-shaped pair. Wire form is the canonical
/// `"<major>.<minor>"` string (custom serde, locked by
/// foundations-review's *String-canonical serialization pattern*).
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    pub const fn new(major: u16, minor: u16) -> Self {
        Self { major, minor }
    }
}

impl std::fmt::Display for ProtocolVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl schemars::JsonSchema for ProtocolVersion {
    fn schema_name() -> String {
        "ProtocolVersion".to_string()
    }
    fn json_schema(_: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(InstanceType::String.into()),
            metadata: Some(Box::new(Metadata {
                description: Some(
                    "Semver-like '<major>.<minor>' (e.g. '0.1', '1.42').".into(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ProtocolVersion;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a `<major>.<minor>` semver-shaped string (e.g. `0.1`)")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ProtocolVersion, E> {
                let mut parts = v.split('.');
                let major = parts
                    .next()
                    .ok_or_else(|| de::Error::custom("missing major"))?
                    .parse::<u16>()
                    .map_err(de::Error::custom)?;
                let minor = parts
                    .next()
                    .ok_or_else(|| de::Error::custom("missing minor"))?
                    .parse::<u16>()
                    .map_err(de::Error::custom)?;
                if parts.next().is_some() {
                    return Err(de::Error::custom("trailing `.`-separated segment"));
                }
                Ok(ProtocolVersion { major, minor })
            }
        }
        d.deserialize_str(V)
    }
}

// -- Capabilities ------------------------------------------------------------

/// Fine-grained capability bits per `docs/specs/protocol.md` §
/// *Capability negotiation*.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    // ---- Frontend rendering ----
    ViewTree,
    StableViewIds,
    UnicodeFull,
    TruecolorStyles,
    Animations,

    // ---- Plugin contributions (declarative) ----
    ContributeCommands,
    ContributeKeymaps,
    ContributeSidebarPane,
    ContributeOverlayPane,
    ContributeStatusItem,
    ContributeThemes,
    ContributeSettings,

    // ---- Plugin runtime API ----
    SubscribePulses,
    InvokeCommands,
    OpenPath,
    ReadDir,
}

// -- Lane payloads -----------------------------------------------------------

// Lane payloads are internally tagged ("kind"); to avoid serde
// duplicate-key collisions when a variant carries another tagged
// enum, those variants are *struct* variants nesting the tagged
// payload under a typed field instead of tuple variants. The nested
// tag lives one object deeper, so the wire stays unambiguous.

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClientToCore {
    Hello(ClientHello),
    Pulse {
        pulse: Pulse,
    },
    Subscribe {
        id: SubscriptionId,
        filter: PulseFilter,
    },
    Unsubscribe {
        id: SubscriptionId,
    },
    Request {
        request: Request,
    },
    Save {
        buffer: Path,
    },
    OpenPath {
        fs_path: PathBuf,
    },
    Goodbye,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreToClient {
    Welcome(ServerWelcome),
    Pulse {
        pulse: Pulse,
    },
    Response {
        response: Response,
    },
    Error {
        error: ProtocolError,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PluginToCore {
    Hello(PluginHello),
    Pulse {
        pulse: Pulse,
    },
    InvokeCommand {
        command: Path,
        args: Option<serde_json::Value>,
    },
    Subscribe {
        id: SubscriptionId,
        filter: PulseFilter,
    },
    Unsubscribe {
        id: SubscriptionId,
    },
    OpenPath {
        fs_path: PathBuf,
    },
    Goodbye,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoreToPlugin {
    Welcome(PluginWelcome),
    Deliver {
        subscription: SubscriptionId,
        pulse: Pulse,
    },
    InvokeCallback {
        handle: u64,
        args: serde_json::Value,
    },
    Error {
        error: ProtocolError,
    },
}

// -- Typed request / response ------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Request {
    View {
        root: Path,
    },
    InvokeCommand {
        command: Path,
        args: Option<serde_json::Value>,
    },
    ListPaths {
        prefix: Path,
        kinds: Option<Vec<PathKind>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Response {
    View {
        result: Result<ViewResponse, RequestError>,
    },
    InvokeCommand {
        result: Result<serde_json::Value, RequestError>,
    },
    ListPaths {
        result: Result<Vec<Path>, RequestError>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewResponse {
    pub root: Path,
    pub view: View,
    /// Monotonic per-frontend counter so the frontend can detect stale
    /// views (out-of-order responses on a future async transport).
    pub version: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RequestError {
    UnknownPath(Path),
    UnknownCommand(Path),
    InvalidArgs(String),
    Cancelled,
    Internal(String),
}

/// Resource kinds the frontend / plugins can enumerate via
/// `Request::ListPaths`. Locked v0 set (resolved 2026-05-07): one
/// variant per Stage-5 namespace migration root. Adding a kind is a
/// minor protocol bump.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathKind {
    /// `/buf/<id>` — open documents.
    Buffer,
    /// `/cur/<id>` — cursors.
    Cursor,
    /// `/pane(/<i>)*` — layout tree nodes.
    Pane,
    /// `/cmd/<dotted-id>` — command registry entries.
    Command,
    /// `/keymap/<chord>` — chord bindings.
    Keymap,
    /// `/theme/<scope>` — active theme scope entries.
    Theme,
    /// `/plugin/<name>` — loaded plugin namespace roots.
    Plugin,
}

/// Host's negotiated wire-protocol version. Plugins declare a
/// required version in `engines.devix`; hosts negotiate the lower
/// of the two minor versions when the majors match, refuse to load
/// when majors mismatch (per `foundations-review.md` §
/// *Versioning alignment*).
pub const HOST_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion::new(0, 1);
/// Host's pulse-bus catalog version. Plugins declare a required
/// version in `engines.pulse_bus`; same negotiation rules as
/// `HOST_PROTOCOL_VERSION`.
pub const HOST_PULSE_BUS_VERSION: ProtocolVersion = ProtocolVersion::new(0, 1);
/// Host's manifest schema version. Plugins declare a required
/// version in `engines.manifest`; same negotiation rules.
pub const HOST_MANIFEST_VERSION: ProtocolVersion = ProtocolVersion::new(0, 1);

// -- Hello / Welcome --------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientHello {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    pub capabilities: HashSet<Capability>,
    pub client_name: String,
    pub client_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerWelcome {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    /// **Negotiated** set — intersection of client and server. The
    /// client should inspect this and decide whether to keep running
    /// if a required capability was dropped (warn-and-degrade-with-
    /// opt-out, resolved 2026-05-07).
    pub capabilities: HashSet<Capability>,
    pub session_id: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginHello {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    pub capabilities: HashSet<Capability>,
    pub plugin_name: String,
    pub plugin_version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginWelcome {
    pub protocol_version: ProtocolVersion,
    pub pulse_bus_version: ProtocolVersion,
    pub manifest_version: ProtocolVersion,
    pub capabilities: HashSet<Capability>,
    pub session_id: u64,
}

// -- Errors -----------------------------------------------------------------

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProtocolError {
    IncompatibleProtocol {
        ours: ProtocolVersion,
        theirs: ProtocolVersion,
    },
    IncompatiblePulseBus {
        ours: ProtocolVersion,
        theirs: ProtocolVersion,
    },
    IncompatibleManifest {
        ours: ProtocolVersion,
        theirs: ProtocolVersion,
    },
    UnknownPath(Path),
    UnknownCommand(Path),
    UnknownSubscription(SubscriptionId),
    DeserializationFailure {
        what: String,
        detail: String,
    },
    InternalError(String),
}

// -- Handle traits ----------------------------------------------------------

/// Frontend-side handle: core invokes this to deliver a
/// `CoreToClient` to the attached frontend.
pub trait FrontendHandle: Send + Sync {
    fn deliver(&self, msg: CoreToClient);
}

/// Frontend's view of core: submit `ClientToCore` messages.
pub trait CoreHandle: Send + Sync {
    fn submit(&self, msg: ClientToCore);
}

/// Plugin-side handle: core invokes this to deliver a `CoreToPlugin`
/// to the attached plugin.
pub trait PluginHandle: Send + Sync {
    fn deliver(&self, msg: CoreToPlugin);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_version_string_round_trips() {
        let v = ProtocolVersion::new(0, 1);
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "\"0.1\"");
        let back: ProtocolVersion = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);

        let v = ProtocolVersion::new(1, 42);
        assert_eq!(serde_json::to_string(&v).unwrap(), "\"1.42\"");
    }

    #[test]
    fn protocol_version_deserialize_rejects_malformed() {
        assert!(serde_json::from_str::<ProtocolVersion>("\"0\"").is_err());
        assert!(serde_json::from_str::<ProtocolVersion>("\"0.1.2\"").is_err());
        assert!(serde_json::from_str::<ProtocolVersion>("\"x.y\"").is_err());
    }

    #[test]
    fn capability_round_trips_serde() {
        let caps = [
            Capability::ViewTree,
            Capability::StableViewIds,
            Capability::UnicodeFull,
            Capability::TruecolorStyles,
            Capability::Animations,
            Capability::ContributeCommands,
            Capability::ContributeKeymaps,
            Capability::ContributeSidebarPane,
            Capability::ContributeOverlayPane,
            Capability::ContributeStatusItem,
            Capability::ContributeThemes,
            Capability::ContributeSettings,
            Capability::SubscribePulses,
            Capability::InvokeCommands,
            Capability::OpenPath,
            Capability::ReadDir,
        ];
        for c in caps {
            let s = serde_json::to_string(&c).unwrap();
            let back: Capability = serde_json::from_str(&s).unwrap();
            assert_eq!(c, back);
        }
        assert_eq!(caps.len(), 16, "v0 capability count");
    }

    #[test]
    fn path_kind_locked_seven_variants() {
        let kinds = [
            PathKind::Buffer,
            PathKind::Cursor,
            PathKind::Pane,
            PathKind::Command,
            PathKind::Keymap,
            PathKind::Theme,
            PathKind::Plugin,
        ];
        for k in kinds {
            let s = serde_json::to_string(&k).unwrap();
            let back: PathKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
        assert_eq!(kinds.len(), 7);
    }

    #[test]
    fn lane_payloads_round_trip_serde() {
        let req = ClientToCore::Request {
            request: Request::View {
                root: Path::parse("/pane").unwrap(),
            },
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ClientToCore = serde_json::from_str(&json).unwrap();
        match back {
            ClientToCore::Request {
                request: Request::View { root },
            } => {
                assert_eq!(root.as_str(), "/pane");
            }
            _ => panic!("variant mismatch"),
        }
    }

    #[test]
    fn envelope_carries_payload_with_versioning() {
        let env = Envelope {
            protocol_version: ProtocolVersion::new(0, 1),
            seq: 42,
            payload: ClientToCore::Goodbye,
        };
        let json = serde_json::to_string(&env).unwrap();
        let back: Envelope<ClientToCore> = serde_json::from_str(&json).unwrap();
        assert_eq!(back.protocol_version, ProtocolVersion::new(0, 1));
        assert_eq!(back.seq, 42);
        match back.payload {
            ClientToCore::Goodbye => {}
            _ => panic!("payload mismatch"),
        }
    }

    #[test]
    fn hello_welcome_three_versions_independent() {
        // The three versions evolve independently — Hello carries each.
        let hello = ClientHello {
            protocol_version: ProtocolVersion::new(0, 1),
            pulse_bus_version: ProtocolVersion::new(0, 3),
            manifest_version: ProtocolVersion::new(0, 1),
            capabilities: HashSet::from([Capability::ViewTree]),
            client_name: "devix-tui".to_string(),
            client_version: "0.1.0".to_string(),
        };
        let json = serde_json::to_string(&hello).unwrap();
        let back: ClientHello = serde_json::from_str(&json).unwrap();
        assert_eq!(back.protocol_version, ProtocolVersion::new(0, 1));
        assert_eq!(back.pulse_bus_version, ProtocolVersion::new(0, 3));
        assert_eq!(back.manifest_version, ProtocolVersion::new(0, 1));
    }
}
