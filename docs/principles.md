# devix — Principles

devix is built around nine architectural north stars, grouped into six themes. Each is a borrowed idea, not a devix invention. The point of borrowing is so the system stays cheap to extend and resilient to change as scope grows.

For each star: the principle in one sentence, what it *forbids* inside devix, and a primary source.

## Meta-principle

### SICP — primitives, combination, abstraction
**Principle.** Any expressive system has primitive elements, means of combining them, and means of naming combinations so they become primitives at the next level.
**Forbids.** A new feature whose layer doesn't have all three. If something is added without primitives, without a combinator, or without a naming/abstraction step, it isn't done — it's a one-off.
**Source.** Abelson & Sussman, *Structure and Interpretation of Computer Programs*, §1.1. https://web.mit.edu/6.001/6.037/sicp.pdf

## Structural composition

### MLIR (Lattner) — extend one primitive, don't add new ones
**Principle.** Every IR concept is an `Operation`; new domains are dialects over the same primitive instead of parallel concept hierarchies.
**Forbids.** Sibling subsystems for things that should be nodes in an existing tree. A debugger UI, a terminal, a settings page, a file tree — all variants in the existing layout/pane vocabulary, never a new top-level concept with its own lifecycle.
**Source.** Lattner et al., *MLIR: A Compiler Infrastructure for the End of Moore's Law*, arXiv:2002.11054. https://arxiv.org/abs/2002.11054

### Plan 9 (Pike, Thompson) — uniform addressing via namespaces
**Principle.** Every resource — local, remote, in-memory, hardware — is a node in a hierarchical namespace, accessed through one interface.
**Forbids.** A second URI scheme. Buffers, panes, LSPs, terminals, jobs, and settings live at paths in one per-session namespace, not in seven ad-hoc registries with different lookup APIs.
**Source.** Pike, Presotto, Thompson, Trickey, Winterbottom, *The Use of Name Spaces in Plan 9*, 1992. https://9p.io/sys/doc/names.pdf

### Smalltalk (Kay) — late binding and messaging
**Principle.** The kernel of object-orientation is *messaging*: extreme late binding of all things — what a name refers to, when a call dispatches, where state lives.
**Forbids.** Hard-wiring a dispatch that could be a pulse. If pane X reacts to event Y by direct method call, that's a fixed wire; if it subscribes to a named pulse, the binding is late and a plugin can intercept it later. Default to late.
**Source.** Alan Kay, *The Early History of Smalltalk*, ACM HOPL-II, 1993. https://worrydream.com/EarlyHistoryOfSmalltalk/

## Data shape and performance

### Data-oriented design (Acton) — programs are data transforms
**Principle.** The purpose of a program is to transform data; design the data layout first, then the transforms over it.
**Forbids.** `Box<dyn Trait>`-of-everything where a slotmap, a contiguous span, or a typed enum would do. Cache locality and batch shape are first-class design choices in hot paths (rendering, parsing, search), not optimizations bolted on later.
**Source.** Mike Acton, *Data-Oriented Design and C++*, CppCon 2014. https://www.youtube.com/watch?v=rX0ItVEVjHc

## Operational resilience

### Erlang/OTP (Armstrong) — supervised isolation, let it crash
**Principle.** Build from many small isolated components that fail independently, communicate by messages, and are organized into supervision trees that restart failed children.
**Forbids.** A wedged plugin, a runaway tree-sitter parse, or a crashed LSP taking the editor down. Each is a supervised actor with a restart policy; faults propagate to a supervisor, not a `try { ... }` two layers up.
**Source.** Joe Armstrong, *Making Reliable Distributed Systems in the Presence of Software Errors*, KTH, 2003. https://erlang.org/download/armstrong_thesis_2003.pdf

## Extension surface

### LSP — narrow protocol with optional capabilities
**Principle.** A narrow versioned protocol with optional capability flags turns an M×N integration problem into M+N and keeps old clients working as new capabilities land.
**Forbids.** A plugin API that is a Rust trait. Extension is a versioned protocol with negotiated capabilities — applied internally between core and plugins, not just externally to language servers.
**Source.** *Language Server Protocol Specification*. https://microsoft.github.io/language-server-protocol/

### VS Code — declarative contribution points
**Principle.** Extensions describe what they add (commands, keybindings, panes, settings) declaratively in a manifest; the host wires and lazily activates them.
**Forbids.** Plugins that must run their code before the host can answer "what does this plugin contribute?" Manifests are the source of truth for the palette, settings UI, and `:help`.
**Source.** *VS Code Contribution Points reference.* https://code.visualstudio.com/api/references/contribution-points

## Review discipline

### Hickey — simple is not easy
**Principle.** *Simple* (un-braided, one role, one concept) is an objective property; *easy* (familiar, near to hand) is about the observer. Trading simple for easy is the most common, costliest mistake.
**Forbids.** A type that braids state, time, identity, and behavior because it's faster to write that way. If a review reveals a complected type, splitting it is the work, not the cleanup.
**Source.** Rich Hickey, *Simple Made Easy*, Strange Loop 2011. https://www.infoq.com/presentations/Simple-Made-Easy/

---

## Using these

When a design choice is up in the air, the question is which star it answers to:

- *What is this thing?* → MLIR, Plan 9
- *How is it addressed?* → Plan 9
- *How is it bound?* → Smalltalk
- *How does it lay out in memory?* → Acton
- *What happens when it fails?* → Erlang
- *How does an extension reach it?* → LSP, VS Code
- *Is it doing one thing?* → Hickey
- *Does it have primitives, combination, and abstraction?* → SICP

If a proposal can't be named in those terms, it probably isn't ready.
