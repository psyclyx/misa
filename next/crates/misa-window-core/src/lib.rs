//! Backend-neutral events in physical pixels and elapsed time since window start.
//! Hosts translate native input at their boundary; the UI returns commands and
//! a scheduling hint, never a window or a presenter.
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Commands,
    Menu,
    Up,
    Down,
    Backspace,
    Delete,
    Left,
    Right,
    Home,
    End,
    Enter { newline: bool },
    Tab { backward: bool },
    Escape,
    Copy,
    SelectAll,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

/// The phase of a pointer gesture: a press, a move while held, or the release.
/// A click is a press and a release on one target; a drag is a press followed
/// by moves. Widgets must be able to tell them apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerPhase {
    Press,
    Move,
    Release,
}

impl Event {
    /// A pointer press at `(x, y)`.
    pub fn press(x: f32, y: f32) -> Self {
        Event::Pointer {
            x,
            y,
            phase: PointerPhase::Press,
        }
    }
    /// A pointer move while held at `(x, y)`.
    pub fn hold(x: f32, y: f32) -> Self {
        Event::Pointer {
            x,
            y,
            phase: PointerPhase::Move,
        }
    }
    /// A pointer release at `(x, y)`.
    pub fn release(x: f32, y: f32) -> Self {
        Event::Pointer {
            x,
            y,
            phase: PointerPhase::Release,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Key(Key),
    /// Committed IME text or printable/pasted text; never a physical keycode.
    Text(String),
    /// A pointer gesture. A click is a press and release on one target; a drag
    /// is a press followed by moves. Widgets distinguish them.
    Pointer {
        x: f32,
        y: f32,
        phase: PointerPhase,
    },
    /// Normalized context request in physical window pixels (pointer or keyboard).
    ContextMenu {
        x: f32,
        y: f32,
    },
    /// Positive scroll moves content down (in physical pixels).
    Wheel {
        delta: f32,
    },
    Resize(Size),
    Theme {
        light: bool,
    },
    /// Explicit paint request; no presentation backend is involved.
    Redraw(Size),
}

/// Commands are effects for the host; a frame is an optional value for its renderer.
/// `deadline` is an elapsed time relative to the same clock as the Redraw call.
#[derive(Debug)]
pub struct Output<C, F> {
    pub commands: Vec<C>,
    pub frame: Option<F>,
    pub redraw: bool,
    pub deadline: Option<Duration>,
}

impl<C, F> Default for Output<C, F> {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            frame: None,
            redraw: false,
            deadline: None,
        }
    }
}

pub trait Clock {
    fn elapsed(&self) -> Duration;
}

pub struct MonotonicClock {
    start: Instant,
}
impl MonotonicClock {
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
    pub fn instant(&self, elapsed: Duration) -> Instant {
        self.start + elapsed
    }
}
impl Default for MonotonicClock {
    fn default() -> Self {
        Self::new()
    }
}
impl Clock for MonotonicClock {
    fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }
}

/// Next phase boundary, anchored to clock origin rather than the last repaint.
pub fn next_deadline(elapsed: Duration, period: Duration) -> Duration {
    assert!(!period.is_zero());
    let nanos = period.as_nanos();
    let next = (elapsed.as_nanos() / nanos + 1) * nanos;
    Duration::from_nanos(u64::try_from(next).expect("pulse deadline exceeds Duration"))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeClock(Duration);
    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.0
        }
    }
    #[test]
    fn phases_are_anchored_and_fake_clock_is_deterministic() {
        let mut clock = FakeClock(Duration::ZERO);
        let period = Duration::from_millis(160);
        assert_eq!(next_deadline(clock.elapsed(), period), period);
        clock.0 = Duration::from_micros(160_001);
        assert_eq!(
            next_deadline(clock.elapsed(), period),
            Duration::from_millis(320)
        );
        clock.0 = Duration::from_millis(320);
        assert_eq!(
            next_deadline(clock.elapsed(), period),
            Duration::from_millis(480)
        );
    }
}
