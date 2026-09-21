//! Constant-section flat-bottom pocket planning.
mod entries;
mod planner;
mod verify;
use crate::{
    operations::{LocatedDiagnostic, PlanContext},
    project::{
        LeadSpec,
        v5::{PocketEntry, PocketSettingsV5},
    },
};
pub(crate) use planner::plan;
pub use verify::verify_recorded_motions;

pub(crate) fn missing_fields(
    ctx: &PlanContext,
    id: &str,
    s: &PocketSettingsV5,
) -> Vec<LocatedDiagnostic> {
    let mut missing = vec![];
    let mut need = |present: bool, field: &str| {
        if !present {
            missing.push(LocatedDiagnostic::missing(
                id,
                field,
                format!("set {field} before pocket planning"),
            ));
        }
    };
    need(
        ctx.setup.stock.thickness_mm.is_some(),
        "setup.stock.thickness_mm",
    );
    need(ctx.setup.stock.xy.is_some(), "setup.stock.xy");
    need(
        ctx.setup.clearance_above_stock_mm.is_some(),
        "setup.clearance_above_stock_mm",
    );
    need(ctx.setup.start_xy_mm.is_some(), "setup.start_xy_mm");
    need(
        ctx.tolerances.motion_tolerance_mm.is_some(),
        "tolerances.motion_tolerance_mm",
    );
    need(
        ctx.tolerances.verification_tolerance_mm.is_some(),
        "tolerances.verification_tolerance_mm",
    );
    need(!s.components.is_empty(), "components");
    need(s.direction.is_some(), "direction");
    need(
        s.assignment.spindle_direction.is_some(),
        "assignment.spindle_direction",
    );
    need(s.wall_allowance_mm.is_some(), "wall_allowance_mm");
    need(
        ctx.tool(&s.assignment.tool_id)
            .is_some_and(|t| t.geometry.is_some()),
        "assignment.tool_id.geometry",
    );
    for (v, name) in [
        (s.assignment.spindle_rpm, "spindle_rpm"),
        (s.assignment.cutting_feed_mm_min, "cutting_feed_mm_min"),
        (s.assignment.max_stepdown_mm, "max_stepdown_mm"),
        (s.assignment.stepover_mm, "stepover_mm"),
    ] {
        need(v.is_some(), &format!("assignment.{name}"));
    }
    match s.entry {
        PocketEntry::Plunge => need(
            s.assignment.plunge_feed_mm_min.is_some(),
            "assignment.plunge_feed_mm_min",
        ),
        PocketEntry::Ramp {
            max_angle_deg,
            feed_mm_min,
        }
        | PocketEntry::Helix {
            max_angle_deg,
            feed_mm_min,
            ..
        } => {
            need(max_angle_deg.is_some(), "entry.max_angle_deg");
            need(feed_mm_min.is_some(), "entry.feed_mm_min");
        }
    }
    if let PocketEntry::Helix { radius_mm, .. } = s.entry {
        need(radius_mm.is_some(), "entry.radius_mm");
    }
    for (lead, prefix) in [(&s.lead_in, "lead_in"), (&s.lead_out, "lead_out")] {
        match lead {
            LeadSpec::None => {}
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                need(length_mm.is_some(), &format!("{prefix}.length_mm"));
                need(feed_mm_min.is_some(), &format!("{prefix}.feed_mm_min"));
            }
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                feed_mm_min,
            } => {
                need(radius_mm.is_some(), &format!("{prefix}.radius_mm"));
                need(sweep_deg.is_some(), &format!("{prefix}.sweep_deg"));
                need(feed_mm_min.is_some(), &format!("{prefix}.feed_mm_min"));
            }
        }
    }
    missing
}
