//! Independent kinematic replay of emitted knife holder polylines (plan
//! section 12.5). This is an engineering check of the no-slip blade model —
//! deliberately separate from the planner's modeled headings — not a
//! prediction of blade flex or material behaviour.
//!
//! For holder trajectory `q(t)` with blade heading unit vector `u(theta)`
//! (pivot toward tip, the display convention shared with
//! [`crate::toolpath::knife_tip`]) and its perpendicular `n(theta)`, the
//! zero-lateral-velocity condition at the tip gives
//! `theta_dot = -dot(q_dot, n(theta)) / blade_offset` while contact is
//! established. On a straight holder segment of direction `beta` this is
//! `dphi/ds = -sin(beta - phi)/d`. Fully lifted moves retain the heading
//! instead of applying the contact equation in air.
use crate::{
    geometry::Point,
    toolpath::{MotionEffect, PlannedMotion},
};

/// The tip the planner intends a motion to produce; the replay measures its
/// own integrated tip against this.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IntendedTip {
    /// Lifted travel: no contact, nothing to compare.
    Lifted,
    /// The tip stays planted at this point (entries, swivels, verticals).
    Planted(Point),
    /// The tip travels this segment (compensated cutting lines).
    Segment(Point, Point),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayStatus {
    /// Every sampled deviation and heading error stayed within budget.
    Within,
    /// A deviation or heading error exceeded its budget.
    Exceeded,
    /// The adaptive integration exhausted its step budget: inconclusive.
    BudgetExhausted,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReplayOutcome {
    pub status: ReplayStatus,
    pub max_tip_deviation_mm: f64,
    pub max_heading_error_deg: f64,
}

/// Local per-step heading tolerance of the adaptive integrator, radians.
/// Far below any mm-scale tip budget, so integration error never competes
/// with the geometry being checked.
const LOCAL_TOLERANCE_RAD: f64 = 1e-10;
/// Smallest step the integrator will halve to; below this the step is
/// accepted anyway so termination is guaranteed.
const MIN_STEP: f64 = 1e-12;

fn point_segment_distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = Point::new(b.x - a.x, b.y - a.y);
    let len_sq = ab.x * ab.x + ab.y * ab.y;
    if len_sq <= 1e-24 {
        return p.distance(a);
    }
    let t = (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len_sq).clamp(0., 1.);
    p.distance(Point::new(a.x + ab.x * t, a.y + ab.y * t))
}

fn intended_distance(tip: Point, intended: IntendedTip) -> Option<f64> {
    match intended {
        IntendedTip::Lifted => None,
        IntendedTip::Planted(p) => Some(tip.distance(p)),
        IntendedTip::Segment(a, b) => Some(point_segment_distance(tip, a, b)),
    }
}

/// Heading difference in degrees normalized to [0, 180].
fn heading_error_deg(a: f64, b: f64) -> f64 {
    let mut delta = (a - b).abs().rem_euclid(360.);
    if delta > 180. {
        delta = 360. - delta;
    }
    delta
}

/// One RK4 step of `dphi/ds = -sin(beta - phi)/d`.
fn rk4_step(phi: f64, h: f64, beta: f64, d: f64) -> f64 {
    let f = |phi: f64| -(beta - phi).sin() / d;
    let k1 = f(phi);
    let k2 = f(phi + h / 2. * k1);
    let k3 = f(phi + h / 2. * k2);
    let k4 = f(phi + h * k3);
    phi + h / 6. * (k1 + 2. * k2 + 2. * k3 + k4)
}

struct ReplayRun {
    outcome: ReplayOutcome,
    tip_budget_mm: f64,
    heading_tolerance_deg: f64,
    blade_offset_mm: f64,
}

impl ReplayRun {
    fn sample(&mut self, pivot: Point, phi: f64, intended: IntendedTip) {
        let tip = Point::new(
            pivot.x + self.blade_offset_mm * phi.cos(),
            pivot.y + self.blade_offset_mm * phi.sin(),
        );
        match intended_distance(tip, intended) {
            Some(deviation) => {
                self.outcome.max_tip_deviation_mm =
                    self.outcome.max_tip_deviation_mm.max(deviation);
                if deviation > self.tip_budget_mm {
                    self.outcome.status = ReplayStatus::Exceeded;
                }
            }
            // A contact motion with no intended tip is a construction bug;
            // the replay refuses to bless it.
            None => self.outcome.status = ReplayStatus::Exceeded,
        }
    }

    fn check_heading(&mut self, integrated_deg: f64, modeled_deg: f64) {
        let error = heading_error_deg(integrated_deg, modeled_deg);
        self.outcome.max_heading_error_deg = self.outcome.max_heading_error_deg.max(error);
        if error > self.heading_tolerance_deg {
            self.outcome.status = ReplayStatus::Exceeded;
        }
    }
}

/// Replay the emitted motions under the ideal no-slip model. The initial
/// heading is the alignment assumption carried by the first contact
/// motion's modeled start heading; from there the replay integrates
/// independently and compares against the modeled headings at every
/// contact-motion boundary.
pub fn replay(
    motions: &[PlannedMotion],
    blade_offset_mm: f64,
    intended: &[IntendedTip],
    tip_budget_mm: f64,
    step_budget: usize,
) -> ReplayOutcome {
    let mut run = ReplayRun {
        outcome: ReplayOutcome {
            status: ReplayStatus::Within,
            max_tip_deviation_mm: 0.,
            max_heading_error_deg: 0.,
        },
        tip_budget_mm,
        heading_tolerance_deg: ((2. * tip_budget_mm / blade_offset_mm).to_degrees()).max(0.01),
        blade_offset_mm,
    };
    let mut heading: Option<f64> = None;
    let mut steps = 0usize;
    for (motion, intent) in motions.iter().zip(intended) {
        if motion.effect != MotionEffect::KnifeTrace {
            // Lifted: the holder moves, the heading is retained.
            continue;
        }
        let (modeled_start, modeled_end) = motion.blade_heading_deg.unwrap_or((0., 0.));
        let phi = *heading.get_or_insert(modeled_start.to_radians());
        let a = motion.start.xy();
        let b = motion.end.xy();
        run.sample(a, phi, *intent);
        let length = a.distance(b);
        if length <= 1e-12 {
            // Vertical contact move: the tip stays planted; the modeled
            // headings must be constant through it.
            run.check_heading(modeled_start, modeled_end);
            continue;
        }
        let beta = (b.y - a.y).atan2(b.x - a.x);
        let mut s = 0f64;
        let mut h = length.min(blade_offset_mm);
        let mut phi = phi;
        while s < length - 1e-12 {
            if s + h > length {
                h = length - s;
            }
            let full = rk4_step(phi, h, beta, blade_offset_mm);
            let half = rk4_step(
                rk4_step(phi, h / 2., beta, blade_offset_mm),
                h / 2.,
                beta,
                blade_offset_mm,
            );
            steps += 8;
            if steps > step_budget {
                run.outcome.status = ReplayStatus::BudgetExhausted;
                return run.outcome;
            }
            let error = (half - full).abs();
            if error > LOCAL_TOLERANCE_RAD && h / 2. > MIN_STEP {
                h /= 2.;
                continue;
            }
            phi = half;
            s += h;
            run.sample(
                Point::new(
                    a.x + (b.x - a.x) * s / length,
                    a.y + (b.y - a.y) * s / length,
                ),
                phi,
                *intent,
            );
            if error < LOCAL_TOLERANCE_RAD / 64. {
                h *= 2.;
            }
        }
        heading = Some(phi);
        run.check_heading(phi.to_degrees(), modeled_end);
    }
    if run.outcome.max_tip_deviation_mm > tip_budget_mm {
        run.outcome.status = ReplayStatus::Exceeded;
    }
    run.outcome
}
