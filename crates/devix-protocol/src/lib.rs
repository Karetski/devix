//! `devix-protocol` — pure data + serde, the cross-crate contract.
//!
//! This is the lowest layer above `devix-text` / `devix-syntax`. Every type
//! that crosses a lane boundary (client↔core, plugin↔core) lives here and
//! nowhere else; there is no business logic, no I/O, no traits with
//! non-trivial bodies — only data shapes, marker traits, and handle traits.
//! Plugin authors building third-party crates depend on this alone, never on
//! `devix-core`.
//!
//! Stage 1 task T-10 creates the crate skeleton. Concrete types are filled
//! in module-by-module during Stages 3–4; until then the modules are empty
//! and exist as anchor points so downstream crates can name their imports
//! against the final shape.
//!
//! Module map (each maps to one Stage-0 spec under `docs/specs/`):
//! * [`path`] — `Path`, `PathError`, `Lookup` (T-30, namespace.md).
//! * [`pulse`] — `Pulse`, `PulseKind`, `PulseField`, `PulseFilter` types
//!   (T-31, pulse-bus.md). The `PulseBus` *implementation* lives in
//!   `devix-core::bus`.
//! * [`protocol`] — `Envelope`, `ProtocolVersion`, `Capability`, lane
//!   enums, `Request`/`Response`, handle traits (T-32, protocol.md).
//! * [`manifest`] — `Manifest`, `Contributes`, `*Spec` (T-33,
//!   manifest.md).
//! * [`view`] — `View`, `ViewNodeId`, `Style`, `Color`, `Axis`,
//!   `SidebarSlot`, etc. (T-40 / T-41, frontend.md).
//! * [`input`] — `InputEvent`, `Chord`, `KeyCode`, `Modifiers` (T-42,
//!   frontend.md).

pub mod input;
pub mod manifest;
pub mod path;
pub mod protocol;
pub mod pulse;
pub mod view;

pub use input::{Chord, InputEvent, KeyCode, Modifiers, MouseButton, MouseKind};
pub use manifest::{
    CommandSpec, Contributes, Engines, KeymapSpec, Manifest, ManifestValidationError, PaneSpec,
    SettingSpec, SubscriptionSpec, ThemeSpec,
};
pub use path::{Lookup, Path, PathError};
pub use protocol::{
    Capability, ClientHello, ClientToCore, CoreHandle, CoreToClient, CoreToPlugin, Envelope,
    FrontendHandle, PathKind, PluginHandle, PluginHello, PluginToCore, PluginWelcome,
    ProtocolError, ProtocolVersion, Request, RequestError, Response, ServerWelcome, ViewResponse,
};
pub use pulse::{
    DirtyReason, InvocationSource, ModalKind, Pulse, PulseField, PulseFilter, PulseKind,
    SubscriptionId, ThemePalette,
};
pub use view::{Axis, Color, NamedColor, SidebarSlot, Style, View, ViewNodeId};

/// `HighlightSpan` is defined in `devix-syntax`; re-exported here so
/// consumers of the View IR (`view::View::Buffer.highlights`) reach for one
/// import path.
pub use devix_syntax::HighlightSpan;
