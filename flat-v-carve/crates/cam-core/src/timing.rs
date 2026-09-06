//! Opt-in wall-clock stage timings on stderr; artifact output is unaffected.
use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

pub(crate) struct Timer {
    start: Option<Instant>,
    stage: &'static str,
}
impl Timer {
    pub fn new(stage: &'static str) -> Self {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        Self {
            start: ENABLED
                .get_or_init(|| std::env::var_os("CAM_TIMINGS").is_some_and(|v| v == "1"))
                .then(Instant::now),
            stage,
        }
    }
    pub fn lap(&mut self, label: &str) {
        if let Some(start) = &mut self.start {
            eprintln!(
                "CAM_TIMING {} / {label}: {:.3} s",
                self.stage,
                start.elapsed().as_secs_f64()
            );
            *start = Instant::now();
        }
    }
    pub fn start_sample(&self) -> Option<Instant> {
        self.start.map(|_| Instant::now())
    }
    pub fn accumulated(&self, label: &str, duration: Duration) {
        if self.start.is_some() {
            eprintln!(
                "CAM_TIMING {} / {label} (accumulated): {:.3} s",
                self.stage,
                duration.as_secs_f64()
            );
        }
    }
}
impl Drop for Timer {
    fn drop(&mut self) {
        self.lap("done");
    }
}
