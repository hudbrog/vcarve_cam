//! Motion-shape measurement published with every export.
//!
//! The report answers the question an operator and a planner change both need
//! before anyone runs the program: how many moves it contains, how long they
//! are, how many it spends per millimetre of cut, whether the machine asked
//! for a coarser precision than the plan needed, and which stages therefore
//! run thousands of sub-tolerance moves.
//!
//! This is measurement, never a gate. It reports what was emitted; the plan
//! checks and the knife replay still decide whether output may exist.
use crate::{
    checks::CheckFinding,
    post::{profile::PathControl, sequence::PreparedStage},
    sequence::StageRole,
    toolpath::{Interpolation, PlannedMotion},
};
use serde::{Deserialize, Serialize};

/// Upper bounds of the published move-length histogram, in mm. The histogram
/// adds one open-ended bucket above the last entry.
pub const MOTION_LENGTH_LADDER_MM: [f64; 11] =
    [0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1., 2., 5.];

/// A feed move this short is inside the noise floor of a woodworking machine:
/// the controller cannot accelerate to the programmed feed and back within it.
pub const MICRO_MOVE_MM: f64 = 0.05;
/// A stage whose feed moves are at least this dense is program-bound rather
/// than geometry-bound, whatever its feed rate says.
pub const MICRO_MOVE_DENSITY_PER_MM: f64 = 4.;
/// Share of feed moves below [`MICRO_MOVE_MM`] that makes a stage worth
/// reporting by name.
pub const MICRO_MOVE_SHARE: f64 = 0.25;

/// One bucket of the published histogram; the last bucket is open-ended.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionLengthBucket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper_mm: Option<f64>,
    pub motions: usize,
}

/// XY move-length statistics of one stage or of a whole program. Only feed
/// moves that actually travel in XY take part in the lengths, quantiles and
/// shares: a vertical entry or a same-position feed command is a real motion
/// but not a lateral move of the machine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionLengthProfile {
    pub motions: usize,
    pub feed_motions: usize,
    pub rapid_motions: usize,
    /// Feed motions programmed as arcs (`G2`/`G3`) rather than straight moves.
    pub arc_feed_motions: usize,
    /// Feed moves with a nonzero XY length: the population below.
    pub xy_feed_moves: usize,
    /// Summed XY length of every feed move, mm.
    pub feed_length_mm: f64,
    /// Summed XY length of every motion including rapids, mm.
    pub xy_travel_mm: f64,
    pub shortest_feed_mm: f64,
    pub p10_feed_mm: f64,
    pub median_feed_mm: f64,
    pub longest_feed_mm: f64,
    pub share_feed_under_0p01_mm: f64,
    pub share_feed_under_0p05_mm: f64,
    /// Feed moves per millimetre of feed path: the micro-move density.
    pub feed_moves_per_mm: f64,
    pub histogram: Vec<MotionLengthBucket>,
}

impl Default for MotionLengthProfile {
    /// Nothing measured, with the whole ladder present so every report is
    /// comparable bucket by bucket.
    fn default() -> Self {
        Self {
            motions: 0,
            feed_motions: 0,
            rapid_motions: 0,
            arc_feed_motions: 0,
            xy_feed_moves: 0,
            feed_length_mm: 0.,
            xy_travel_mm: 0.,
            shortest_feed_mm: 0.,
            p10_feed_mm: 0.,
            median_feed_mm: 0.,
            longest_feed_mm: 0.,
            share_feed_under_0p01_mm: 0.,
            share_feed_under_0p05_mm: 0.,
            feed_moves_per_mm: 0.,
            histogram: buckets_of(&[]),
        }
    }
}

impl MotionLengthProfile {
    /// Measure one slice of an ordered plan. `MOTION_LENGTH_LADDER_MM` is
    /// always present in the result, zeros included, so two reports can be
    /// compared bucket by bucket.
    pub fn of(motions: &[PlannedMotion]) -> Self {
        let mut lengths = vec![];
        let mut profile = Self {
            motions: motions.len(),
            histogram: buckets_of(&[]),
            ..Self::default()
        };
        for motion in motions {
            // An arc's programmed move is its swing, not its chord: reporting
            // the chord would make an arc-fitted program look shorter than it
            // is and hide the length a feed-rate estimate depends on.
            let length = match motion.interpolation {
                Interpolation::ArcFeed(arc) => arc
                    .length(motion.start.xy(), motion.end.xy())
                    .unwrap_or_else(|| {
                        (motion.end.x - motion.start.x).hypot(motion.end.y - motion.start.y)
                    }),
                _ => (motion.end.x - motion.start.x).hypot(motion.end.y - motion.start.y),
            };
            profile.xy_travel_mm += length;
            match motion.interpolation {
                Interpolation::Rapid => profile.rapid_motions += 1,
                // A dwell halts the axes; it is timed by its duration, not by
                // any travel, and is neither a rapid nor a feed motion.
                Interpolation::Dwell { .. } => {}
                Interpolation::ArcFeed(_) => {
                    profile.feed_motions += 1;
                    profile.arc_feed_motions += 1;
                    profile.feed_length_mm += length;
                    if length > 0. {
                        lengths.push(length);
                    }
                }
                Interpolation::LinearFeed => {
                    profile.feed_motions += 1;
                    profile.feed_length_mm += length;
                    if length > 0. {
                        lengths.push(length);
                    }
                }
            }
        }
        lengths.sort_by(f64::total_cmp);
        profile.xy_feed_moves = lengths.len();
        profile.histogram = buckets_of(&lengths);
        if lengths.is_empty() {
            return profile;
        }
        let quantile = |share: f64| {
            let index = ((lengths.len() as f64 * share) as usize).min(lengths.len() - 1);
            lengths[index]
        };
        let share_below = |limit: f64| {
            lengths.partition_point(|length| *length < limit) as f64 / lengths.len() as f64
        };
        profile.shortest_feed_mm = lengths[0];
        profile.p10_feed_mm = quantile(0.1);
        profile.median_feed_mm = quantile(0.5);
        profile.longest_feed_mm = lengths[lengths.len() - 1];
        profile.share_feed_under_0p01_mm = share_below(0.01);
        profile.share_feed_under_0p05_mm = share_below(MICRO_MOVE_MM);
        if profile.feed_length_mm > 0. {
            profile.feed_moves_per_mm = lengths.len() as f64 / profile.feed_length_mm;
        }
        profile
    }

    /// True when this population is dominated by moves the machine cannot
    /// execute at feed.
    pub fn is_micro_move_bound(&self) -> bool {
        self.xy_feed_moves > 0
            && self.share_feed_under_0p05_mm >= MICRO_MOVE_SHARE
            && self.feed_moves_per_mm >= MICRO_MOVE_DENSITY_PER_MM
    }
}

fn buckets_of(sorted: &[f64]) -> Vec<MotionLengthBucket> {
    let mut buckets: Vec<MotionLengthBucket> = MOTION_LENGTH_LADDER_MM
        .iter()
        .map(|upper| MotionLengthBucket {
            upper_mm: Some(*upper),
            motions: 0,
        })
        .collect();
    buckets.push(MotionLengthBucket {
        upper_mm: None,
        motions: 0,
    });
    let mut next = 0;
    for length in sorted {
        while next < MOTION_LENGTH_LADDER_MM.len() && *length >= MOTION_LENGTH_LADDER_MM[next] {
            next += 1;
        }
        buckets[next].motions += 1;
    }
    buckets
}

/// One prepared stage's motion shape, including the path-control mode it was
/// written under: the same move count means something very different under
/// exact path and under tolerance blending.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StageMotionProfile {
    pub stage_id: String,
    pub operation_id: String,
    pub role: StageRole,
    pub tool_number: u32,
    pub motion_range: (usize, usize),
    pub path_control: PathControl,
    pub profile: MotionLengthProfile,
}

/// Motion shape of a whole prepared execution.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionProfileReport {
    pub stages: Vec<StageMotionProfile>,
    pub total: MotionLengthProfile,
    /// Decimal places the machine profile asked for.
    pub decimal_places_requested: usize,
    /// Decimal places the emitted program actually uses. Higher than the
    /// request means the plan holds moves finer than the machine's own
    /// resolution and the writer had to keep them.
    pub decimal_places_written: usize,
}

impl MotionProfileReport {
    pub fn of(
        stages: &[PreparedStage],
        motions: &[PlannedMotion],
        decimal_places_requested: usize,
        decimal_places_written: usize,
    ) -> Self {
        let stages = stages
            .iter()
            .map(|stage| {
                let (first, end) = stage.stage.motion_range;
                StageMotionProfile {
                    stage_id: stage.stage.stage_id.clone(),
                    operation_id: stage.stage.operation_id.clone(),
                    role: stage.stage.role,
                    tool_number: stage.tool_number,
                    motion_range: (first, end),
                    path_control: stage.process.path_control,
                    profile: MotionLengthProfile::of(motions.get(first..end).unwrap_or_default()),
                }
            })
            .collect();
        Self {
            stages,
            total: MotionLengthProfile::of(motions),
            decimal_places_requested,
            decimal_places_written,
        }
    }

    /// Informational observations, in report order. These never fail a check;
    /// they name the stages whose programmed motion is finer than the machine
    /// can execute, and any precision the writer had to add to keep it.
    pub fn observations(&self) -> Vec<CheckFinding> {
        let mut observations = vec![];
        if self.decimal_places_written > self.decimal_places_requested {
            observations.push(observation(
                "EXPORT_PRECISION_ESCALATED",
                format!(
                    "the machine profile asks for {} decimal places but the program needed {} \
                     to preserve moves this plan contains; raise the profile precision or \
                     simplify the path so the output grid is the machine's own",
                    self.decimal_places_requested, self.decimal_places_written
                ),
                None,
            ));
        }
        for stage in &self.stages {
            let profile = &stage.profile;
            // A knife stage that blends is a deliberate, verified relaxation
            // of exact path: say so, with the deviation it was allowed.
            if stage.role == StageRole::Knife
                && let PathControl::Blend { tolerance_mm, .. } = stage.path_control
            {
                observations.push(observation(
                    "EXPORT_KNIFE_BLENDED_PATH",
                    format!(
                        "stage '{}' runs the knife tip under a blend of {tolerance_mm} mm \
                         instead of exact path, bounded by the deviation the planner's own \
                         tip replay left unused; the machine may round corners by up to that much",
                        stage.stage_id
                    ),
                    Some(stage.stage_id.clone()),
                ));
            }
            if !profile.is_micro_move_bound() {
                continue;
            }
            let exact = stage.path_control == PathControl::ExactPath;
            observations.push(observation(
                if exact {
                    "EXPORT_EXACT_PATH_MICRO_MOVES"
                } else {
                    "EXPORT_MICRO_MOVE_DENSITY"
                },
                format!(
                    "stage '{}' programs {} feed moves over {:.1} mm of path ({:.1} per mm); \
                     {:.0}% of them are shorter than {} mm and the median is {:.4} mm{}",
                    stage.stage_id,
                    profile.xy_feed_moves,
                    profile.feed_length_mm,
                    profile.feed_moves_per_mm,
                    profile.share_feed_under_0p05_mm * 100.,
                    MICRO_MOVE_MM,
                    profile.median_feed_mm,
                    if exact {
                        "; exact path mode stops the machine at every one of them"
                    } else {
                        ""
                    }
                ),
                Some(stage.stage_id.clone()),
            ));
        }
        observations
    }
}

fn observation(code: &str, message: String, stage_id: Option<String>) -> CheckFinding {
    CheckFinding {
        code: code.into(),
        message,
        operation_id: None,
        stage_id,
    }
}
