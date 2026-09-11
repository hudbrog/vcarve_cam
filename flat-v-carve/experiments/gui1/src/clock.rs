//! Monotonic timers that exist on both targets.
//!
//! `std::time::Instant::now()` panics on `wasm32-unknown-unknown` with
//! "time not implemented on this platform". That panic trapped the whole browser
//! application on the first frame after a scene loaded, which the native build
//! could not show. Native keeps `Instant` behind a lazily initialised baseline;
//! WASM reads the browser's own `performance.now()`.

pub struct Timer {
    start_ms: f64,
}

impl Timer {
    pub fn start() -> Self {
        Self { start_ms: now_ms() }
    }

    /// Elapsed milliseconds. Never negative, so a coarse clock cannot produce a
    /// nonsensical measurement.
    pub fn elapsed_ms(&self) -> f64 {
        (now_ms() - self.start_ms).max(0.)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_ms() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64() * 1000.
}

#[cfg(target_arch = "wasm32")]
fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|window| window.performance())
        .map_or(0., |performance| performance.now())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_measures_a_real_span_without_going_backwards() {
        let timer = Timer::start();
        std::thread::sleep(std::time::Duration::from_millis(3));
        let elapsed = timer.elapsed_ms();
        assert!(elapsed >= 2., "expected at least 2 ms, measured {elapsed}");
        assert!(elapsed < 5_000., "implausible span {elapsed}");
        let before = now_ms();
        std::thread::sleep(std::time::Duration::from_millis(1));
        assert!(now_ms() >= before, "monotonic clock went backwards");
    }
}
