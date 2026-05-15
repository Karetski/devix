//! Terminal input thread.
//!
//! `crossterm::event::read()` has no push API and no interrupt, so this
//! is the one place in the runtime that genuinely owns a poll thread.
//! `InputThread::spawn` starts it; `shutdown` flips an atomic flag the
//! poll loop checks each iteration and joins, with a deadline so we
//! don't hang on a wedged terminal.
//!
//! T-63 wires a parallel `Pulse::InputReceived` publish: every input
//! event also goes onto the editor's bus as a typed pulse, so plugins
//! and other bus subscribers observe input without depending on the
//! TUI-specific `EventSink::Input(crossterm::Event)` channel. Dispatch
//! to keymap continues through `EventSink::Input` because the
//! crossterm event carries information beyond what `InputEvent` covers
//! (modifier states the keymap consults directly, paste text shape).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind, MouseButton as CtMouseButton, MouseEventKind};
use devix_core::PulseBus;
use devix_protocol::input::{
    Chord, InputEvent, KeyCode as PKey, Modifiers, MouseButton, MouseKind,
};

const POLL_TIMEOUT: Duration = Duration::from_millis(100);

pub(crate) struct InputThread {
    join: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl InputThread {
    pub(crate) fn spawn(sink: crate::event_sink::EventSink, bus: PulseBus) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = stop.clone();
        let join = thread::Builder::new()
            .name("devix-input".into())
            .spawn(move || {
                while !stop_clone.load(Ordering::Acquire) {
                    match event::poll(POLL_TIMEOUT) {
                        Ok(true) => match event::read() {
                            Ok(ev) => {
                                // Publish typed pulse first (observer
                                // notification); send to dispatch
                                // channel second. Dropping the typed
                                // pulse on a wedged bus is fine — the
                                // dispatch channel still wakes the
                                // main loop. (F-1 follow-up
                                // 2026-05-12: publish_async is
                                // non-blocking.)
                                if let Some(input) = crossterm_to_input_event(&ev) {
                                    let _ = bus.publish_async(
                                        devix_protocol::pulse::Pulse::InputReceived {
                                            event: input,
                                        },
                                    );
                                }
                                if sink.input(ev).is_err() {
                                    return;
                                }
                            }
                            Err(_) => return,
                        },
                        Ok(false) => continue,
                        Err(_) => return,
                    }
                }
            })
            .context("spawning devix-input thread")?;
        Ok(Self { join: Some(join), stop })
    }

    /// Signal the thread to stop and join with a deadline. The thread
    /// observes `stop` within `POLL_TIMEOUT` once the flag is set.
    pub(crate) fn shutdown(mut self, deadline: Duration) {
        self.stop.store(true, Ordering::Release);
        let Some(join) = self.join.take() else { return };
        let start = Instant::now();
        while !join.is_finished() {
            if start.elapsed() >= deadline {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        let _ = join.join();
    }
}

/// Convert a crossterm `Event` into the protocol-shape `InputEvent`
/// for publishing as `Pulse::InputReceived`. Returns `None` for events
/// that don't fit the v0 InputEvent shape (resize, anything we
/// haven't mapped yet); the dispatch path still sees the original
/// crossterm event regardless.
pub(crate) fn crossterm_to_input_event(ev: &Event) -> Option<InputEvent> {
    use crossterm::event::KeyCode as CtKey;
    match ev {
        Event::Key(k) if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            let key = match k.code {
                CtKey::Char(c) => PKey::Char(c.to_ascii_lowercase()),
                CtKey::Enter => PKey::Enter,
                CtKey::Tab => PKey::Tab,
                CtKey::BackTab => PKey::BackTab,
                CtKey::Esc => PKey::Esc,
                CtKey::Backspace => PKey::Backspace,
                CtKey::Delete => PKey::Delete,
                CtKey::Insert => PKey::Insert,
                CtKey::Left => PKey::Left,
                CtKey::Right => PKey::Right,
                CtKey::Up => PKey::Up,
                CtKey::Down => PKey::Down,
                CtKey::Home => PKey::Home,
                CtKey::End => PKey::End,
                CtKey::PageUp => PKey::PageUp,
                CtKey::PageDown => PKey::PageDown,
                CtKey::F(n) if (1..=12).contains(&n) => PKey::F(n),
                _ => return None,
            };
            let modifiers = modifiers_from_crossterm(k.modifiers);
            let chord = Chord { key, modifiers };
            let text = match k.code {
                CtKey::Char(c) if c.is_ascii_graphic() || c == ' ' => Some(c),
                _ => None,
            };
            Some(InputEvent::Key { chord, text })
        }
        Event::Mouse(m) => {
            let modifiers = modifiers_from_crossterm(m.modifiers);
            match m.kind {
                MouseEventKind::Down(b) => Some(InputEvent::Mouse {
                    x: m.column,
                    y: m.row,
                    button: Some(crossterm_button(b)),
                    press: MouseKind::Down,
                    modifiers,
                }),
                MouseEventKind::Up(b) => Some(InputEvent::Mouse {
                    x: m.column,
                    y: m.row,
                    button: Some(crossterm_button(b)),
                    press: MouseKind::Up,
                    modifiers,
                }),
                MouseEventKind::Drag(b) => Some(InputEvent::Mouse {
                    x: m.column,
                    y: m.row,
                    button: Some(crossterm_button(b)),
                    press: MouseKind::Drag,
                    modifiers,
                }),
                MouseEventKind::Moved => Some(InputEvent::Mouse {
                    x: m.column,
                    y: m.row,
                    button: None,
                    press: MouseKind::Move,
                    modifiers,
                }),
                MouseEventKind::ScrollUp => Some(InputEvent::Scroll {
                    x: m.column,
                    y: m.row,
                    delta_x: 0,
                    delta_y: -1,
                    modifiers,
                }),
                MouseEventKind::ScrollDown => Some(InputEvent::Scroll {
                    x: m.column,
                    y: m.row,
                    delta_x: 0,
                    delta_y: 1,
                    modifiers,
                }),
                MouseEventKind::ScrollLeft => Some(InputEvent::Scroll {
                    x: m.column,
                    y: m.row,
                    delta_x: -1,
                    delta_y: 0,
                    modifiers,
                }),
                MouseEventKind::ScrollRight => Some(InputEvent::Scroll {
                    x: m.column,
                    y: m.row,
                    delta_x: 1,
                    delta_y: 0,
                    modifiers,
                }),
            }
        }
        Event::Paste(s) => Some(InputEvent::Paste(s.clone())),
        Event::FocusGained => Some(InputEvent::FocusGained),
        Event::FocusLost => Some(InputEvent::FocusLost),
        // Resize doesn't fit InputEvent; T-? wires Pulse::ViewportChanged
        // separately when the resize handler runs.
        _ => None,
    }
}

fn modifiers_from_crossterm(m: crossterm::event::KeyModifiers) -> Modifiers {
    use crossterm::event::KeyModifiers as Km;
    Modifiers {
        ctrl: m.contains(Km::CONTROL),
        alt: m.contains(Km::ALT),
        shift: m.contains(Km::SHIFT),
        super_key: m.contains(Km::SUPER),
    }
}

fn crossterm_button(b: CtMouseButton) -> MouseButton {
    match b {
        CtMouseButton::Left => MouseButton::Left,
        CtMouseButton::Right => MouseButton::Right,
        CtMouseButton::Middle => MouseButton::Middle,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode as CtKey, KeyEvent, KeyEventKind, KeyModifiers as Km};

    #[test]
    fn key_event_press_maps_to_chord() {
        let ev = Event::Key(KeyEvent::new_with_kind(
            CtKey::Char('s'),
            Km::CONTROL,
            KeyEventKind::Press,
        ));
        match crossterm_to_input_event(&ev).unwrap() {
            InputEvent::Key { chord, text } => {
                assert_eq!(chord.key, PKey::Char('s'));
                assert!(chord.modifiers.ctrl);
                assert_eq!(text, Some('s'));
            }
            _ => panic!("variant mismatch"),
        }
    }

    #[test]
    fn key_release_returns_none() {
        let ev = Event::Key(KeyEvent::new_with_kind(
            CtKey::Char('s'),
            Km::CONTROL,
            KeyEventKind::Release,
        ));
        assert!(crossterm_to_input_event(&ev).is_none());
    }

    #[test]
    fn focus_events_map() {
        assert!(matches!(
            crossterm_to_input_event(&Event::FocusGained),
            Some(InputEvent::FocusGained)
        ));
        assert!(matches!(
            crossterm_to_input_event(&Event::FocusLost),
            Some(InputEvent::FocusLost)
        ));
    }

    #[test]
    fn paste_carries_string() {
        let ev = Event::Paste("hello".into());
        match crossterm_to_input_event(&ev).unwrap() {
            InputEvent::Paste(s) => assert_eq!(s, "hello"),
            _ => panic!("variant mismatch"),
        }
    }
}
