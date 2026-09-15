//! The display's own playback clock.
//!
//! The compute process keeps authority over the displayed position: a scrub is
//! still an exact seek against the retained execution. What it cannot do is run
//! a clock — a round trip per frame would serialize the whole scene — so the
//! display holds the same field locally between seeks and advances *inside* a
//! motion with [`sim::Field::apply`]'s fractional window.
//!
//! Every state this clock reaches is `(prefix, fraction)`: the exact state at a
//! motion boundary plus the part of one move that has been cut. That is the
//! same state a cold replay reaches, which
//! `an_advanced_frame_equals_a_cold_replay_at_the_same_position` checks.
use crate::sim::{Field, Motion, Playback, SeekReport, Stats, TILE, TimeTable};
use std::sync::Arc;

/// What one advance did.
#[derive(Clone, Debug, PartialEq)]
pub enum Advance {
    /// The field moved, and the program has not finished. `dirty` names the
    /// tiles whose contents changed, so the caller can patch the raster it
    /// hands the renderer.
    Moved { dirty: Vec<u32>, seconds: f64 },
    /// The same, at the end of the program.
    Finished { dirty: Vec<u32>, seconds: f64 },
    /// The target sits behind the current position, or further than one frame
    /// may replay. The caller restores an exact state and advances again.
    NeedsRestore { prefix: usize, fraction: f64 },
}

pub struct LocalClock {
    playback: Playback,
    motions: Arc<Vec<Motion>>,
    time: TimeTable,
    /// The untouched stock, kept so a transported state can be adopted without
    /// rebuilding the field from the payload.
    pristine: Field,
    /// Versions last handed to the renderer, so an advance can name the tiles
    /// that changed instead of re-packing the whole raster.
    reported_versions: Vec<u32>,
}

impl LocalClock {
    /// Seed from an exact transported state: `field` is the raster at `points`'
    /// last prefix, `pristine` is the untouched stock, and `points` are the
    /// checkpoints that bound local replay.
    pub fn seed(
        field: Field,
        pristine: Field,
        points: Vec<(usize, Field)>,
        motions: Vec<Motion>,
        time: TimeTable,
        checkpoint_budget: usize,
    ) -> Self {
        let playback = Playback::seed(field, pristine.clone(), points, checkpoint_budget);
        let mut clock = Self {
            playback,
            motions: Arc::new(motions),
            time,
            pristine,
            reported_versions: Vec::new(),
        };
        clock.reported_versions = clock.playback.field.versions.clone();
        clock
    }

    /// Adopt an exact state transported from the compute process — a seek
    /// response, or a preset change — keeping the motion stream and the time
    /// table. The caller re-uploads whatever tiles this changed.
    pub fn reseed(&mut self, field: Field, points: Vec<(usize, Field)>, checkpoint_budget: usize) {
        self.playback = Playback::seed(field, self.pristine.clone(), points, checkpoint_budget);
        self.reported_versions = self.playback.field.versions.clone();
    }

    /// Positions of the clock: the motion boundary it sits on and how much of
    /// the next move is cut.
    pub fn position(&self) -> (usize, f64) {
        (self.playback.position, self.playback.fraction())
    }

    pub fn seconds(&self) -> f64 {
        self.time
            .seconds_at(self.playback.position, self.playback.fraction())
    }

    pub fn total_seconds(&self) -> f64 {
        self.time.total_seconds()
    }

    pub fn time(&self) -> &TimeTable {
        &self.time
    }

    pub fn field(&self) -> &Field {
        &self.playback.field
    }

    pub fn stats(&self) -> &Stats {
        &self.playback.field.stats
    }

    pub fn versions(&self) -> Vec<u32> {
        self.playback.field.versions.clone()
    }

    pub fn motions(&self) -> usize {
        self.motions.len()
    }

    /// The move the clock is on, whether or not it has started cutting: what the
    /// transport names as the current tool, stage and feed.
    pub fn motion(&self) -> Option<&Motion> {
        self.motions.get(self.playback.position)
    }

    /// The tool the clock is using: the in-flight move's, or the last completed
    /// move's at the end of the program, where there is no move in flight.
    pub fn tool(&self) -> Option<usize> {
        self.motion().map(|motion| motion.tool).or_else(|| {
            self.motions
                .get(self.playback.position.saturating_sub(1))
                .map(|motion| motion.tool)
        })
    }

    /// The move the animation is inside, and where the cutter tip is. `None`
    /// at the end of the program or on a move with no length.
    pub fn tip(&self) -> Option<([f64; 3], &Motion)> {
        let (prefix, fraction) = self.position();
        let motion = self.motions.get(prefix)?;
        if fraction <= 0. {
            return None;
        }
        Some((motion.point_at(fraction), motion))
    }

    /// Advance by `seconds` of program time: playback's own move.
    pub fn advance(&mut self, seconds: f64, limit: usize) -> Advance {
        self.move_to(self.seconds() + seconds, limit)
    }

    /// Move to an absolute program time, applying at most `limit` motions
    /// locally. Forward motion advances the field and returns the tiles it
    /// changed; a target behind the clock, or one further than a frame may
    /// replay, asks the caller to restore an exact state first.
    pub fn move_to(&mut self, seconds: f64, limit: usize) -> Advance {
        let target = seconds.clamp(0., self.total_seconds());
        if self.is_finished() && target >= self.total_seconds() {
            return Advance::Finished {
                dirty: Vec::new(),
                seconds: self.seconds(),
            };
        }
        let (prefix, fraction) = self.time.position_at(target);
        self.move_to_position(prefix, fraction, limit)
    }

    /// Move to an exact `(prefix, fraction)` position, applying only the windows
    /// in between. This is the slider's entry point as well as playback's.
    pub fn move_to_position(&mut self, prefix: usize, fraction: f64, limit: usize) -> Advance {
        match self
            .playback
            .advance_to(&self.motions, prefix, fraction, limit)
        {
            Ok(true) => {
                let dirty = self.changed_tiles();
                self.reported_versions = self.playback.field.versions.clone();
                let seconds = self.seconds();
                if self.is_finished() {
                    return Advance::Finished { dirty, seconds };
                }
                Advance::Moved { dirty, seconds }
            }
            Ok(false) => Advance::NeedsRestore { prefix, fraction },
            // A refused window (a feed with no rate, say) must not stop the
            // display: the caller falls back to stepping whole motions.
            Err(_) => Advance::NeedsRestore { prefix, fraction },
        }
    }

    pub fn is_finished(&self) -> bool {
        self.playback.position >= self.motions.len() && self.playback.fraction() <= 0.
    }

    /// Restore an exact state — a scrub to a program time, or the worker's
    /// answer to one. This replays from the nearest checkpoint, or from the
    /// pristine field, and is bit-identical to a cold replay.
    pub fn restore(&mut self, prefix: usize, fraction: f64) -> Result<SeekReport, String> {
        let report = self
            .playback
            .seek_position(&self.motions, prefix, fraction)?;
        self.reported_versions = self.playback.field.versions.clone();
        Ok(report)
    }

    /// Exact state of the field, for a caller that has to rebuild the raster.
    pub fn cell_bytes(&self) -> Vec<u8> {
        self.playback.field.cell_bytes()
    }

    /// Files changed since the renderer last received the raster.
    fn changed_tiles(&self) -> Vec<u32> {
        crate::stock_preview::dirty_tiles(&self.playback.field.versions, &self.reported_versions)
    }

    /// Patch `raster` (the tile-major packed grid the renderer draws) with the
    /// tiles the last advance changed. Bytes outside those tiles are untouched,
    /// so the caller can keep one buffer alive instead of re-packing a raster
    /// every frame.
    pub fn patch(&self, raster: &mut [u8], dirty: &[u32]) -> Result<(), String> {
        for tile in dirty {
            let base = Field::tile_byte_offset(*tile as usize);
            let end = base + TILE * TILE * 4;
            let target = raster
                .get_mut(base..end)
                .ok_or("Simulator raster is smaller than its tile grid")?;
            self.playback.field.pack_tile(*tile as usize, target)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Interpolation, Stock, ToolSpec};

    fn motion(x: f64, feed: f64) -> Motion {
        Motion {
            kind: "cut".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(feed),
            x0: x,
            y0: -4.,
            z0: -1.,
            x1: x,
            y1: 4.,
            z1: -1.,
        }
    }

    fn clock(motions: Vec<Motion>) -> LocalClock {
        let stock = Stock {
            x0: -6.,
            y0: -6.,
            x1: 6.,
            y1: 6.,
            thickness_mm: 5.,
        };
        let tools = [ToolSpec::Endmill {
            diameter: 2.,
            cutting_length: 12.,
        }];
        let field = Field::new(stock, &tools, 0.1).unwrap();
        let time = TimeTable::build(&motions, Some(5000.)).unwrap();
        LocalClock::seed(
            field.clone(),
            field.clone(),
            vec![(0, field)],
            motions,
            time,
            32 * 1024 * 1024,
        )
    }

    #[test]
    fn an_advance_inside_a_move_moves_the_tip_and_returns_seconds_not_frames() {
        // 8 mm at 480 mm/min is one second of program time.
        let mut clock = clock(vec![motion(0., 480.)]);
        assert_eq!(clock.total_seconds(), 1.);
        assert_eq!(clock.position(), (0, 0.));
        assert!(
            clock.tip().is_none(),
            "no material cut before the move starts"
        );
        let Advance::Moved { dirty, seconds } = clock.advance(0.25, 64) else {
            panic!("a quarter of a second must advance inside the move");
        };
        assert!((seconds - 0.25).abs() < 1e-9);
        assert!(!dirty.is_empty(), "the first quarter removes material");
        let (prefix, fraction) = clock.position();
        assert_eq!(prefix, 0);
        assert!((fraction - 0.25).abs() < 1e-9);
        let (tip, _) = clock.tip().expect("inside the move");
        // A quarter of the way from y = -4 to y = 4.
        assert!((tip[1] - -2.).abs() < 1e-9, "the tip is at y = -2, {tip:?}");
        // Four such frames finish the move exactly.
        for _ in 0..3 {
            clock.advance(0.25, 64);
        }
        assert!(clock.is_finished());
        assert_eq!(clock.position(), (1, 0.));
        assert_eq!(clock.seconds(), clock.total_seconds());
    }

    #[test]
    fn a_target_behind_the_clock_asks_for_a_restore_instead_of_rewinding() {
        let mut clock = clock(vec![motion(0., 240.), motion(3., 240.)]);
        clock.advance(0.5, 64);
        let (prefix, fraction) = clock.position();
        assert_eq!(prefix, 0);
        assert!(fraction > 0.);
        assert!(matches!(
            clock.advance(-10., 64),
            Advance::NeedsRestore { .. }
        ));
        // The same thing stated as a target: a scrub to an earlier time needs
        // the exact state that only a restore can produce.
        assert!(matches!(
            clock.move_to(0.25, 64),
            Advance::NeedsRestore { .. }
        ));
        // A restore lands on an exact state and can then run forward again.
        clock.restore(0, 0.75).unwrap();
        assert_eq!(clock.position(), (0, 0.75));
        let mut cold = clock.field().clone();
        assert_eq!(cold.checksum(), clock.field().checksum());
        assert_eq!(cold.cell_bytes(), clock.field().cell_bytes());
        assert!(matches!(clock.move_to(1.5, 64), Advance::Moved { .. }));
        cold = clock.field().clone();
        assert_eq!(cold.cell_bytes(), clock.field().cell_bytes());
    }

    #[test]
    fn an_advanced_frame_equals_a_cold_replay_at_the_same_position() {
        let motions = vec![motion(0., 300.), motion(2., 600.), motion(4., 1200.)];
        let mut clock = clock(motions.clone());
        let pristine = clock.field().clone();
        for step in 1..=12 {
            clock.advance(0.1, 64);
            let (prefix, fraction) = clock.position();
            let mut cold = pristine.clone();
            for motion in &motions[..prefix.min(motions.len())] {
                cold.apply(motion, 0., 1.).unwrap();
            }
            if prefix < motions.len() && fraction > 0. {
                cold.apply(&motions[prefix], 0., fraction).unwrap();
            }
            assert_eq!(
                clock.field().cell_bytes(),
                cold.cell_bytes(),
                "step {step} at ({prefix}, {fraction})"
            );
        }
    }

    #[test]
    fn a_raster_patch_touches_only_the_tiles_that_changed() {
        // 1 s of cutting, then a long rapid that removes nothing.
        let rapid = Motion {
            interpolation: Interpolation::Rapid,
            feed_mm_min: None,
            x0: 0.,
            y0: 4.,
            z0: 5.,
            x1: 100.,
            y1: 4.,
            z1: 5.,
            ..motion(0., 480.)
        };
        let motions = vec![motion(0., 480.), rapid];
        let mut clock = clock(motions);
        let mut raster = clock.field().packed_tile_bytes();
        // Past the end of the cut and into the rapid: the step still finishes
        // the cut, and the raster it patches must match the field exactly.
        let Advance::Moved { dirty, .. } = clock.advance(1.2, 64) else {
            panic!("the cut finishes inside this step");
        };
        assert!(!dirty.is_empty());
        clock.patch(&mut raster, &dirty).unwrap();
        assert_eq!(raster, clock.field().packed_tile_bytes());
        // Now inside the rapid: material cannot change, so no tile is dirty and
        // the raster is untouched even though the clock moved.
        let Advance::Moved { dirty, seconds } = clock.advance(0.2, 64) else {
            panic!("the rapid is inside the program, not past its end");
        };
        assert!(seconds > 1., "the clock is inside the rapid");
        assert!(dirty.is_empty(), "a rapid removes nothing");
        clock.patch(&mut raster, &dirty).unwrap();
        assert_eq!(raster, clock.field().packed_tile_bytes());
    }
}
