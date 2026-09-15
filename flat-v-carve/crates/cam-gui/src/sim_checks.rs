//! Two things the simulation can say about the *machine* rather than the cut.
//!
//! Both run on a coarse display raster and both are **display estimates**: they
//! name the cell size they were computed on, they never gate export, and the
//! authoritative checks stay in `cam-core` (machine-simulation plan §7, D9,
//! D10). What they add is the two questions a 2.5D field can answer honestly:
//!
//! 1. would the *assembly* — the shaft and the holder, not the cutter — run
//!    into material that is still there?
//! 2. would a *rapid* move pass through material?
//!
//! The pass walks the program once over its own raster, in the worker, at
//! generation time, so nothing is recomputed per displayed frame.
use crate::sim::{Motion, Stock, TILE, Tool as NormalizedTool, ToolSpec};
use cam_core::post::HolderSegment;
use cam_core::project::ToolAssembly;
use serde::{Deserialize, Serialize};

/// Raster the checks run on. Deliberately coarser than the display raster: a
/// holder collision is millimetres deep, and a coarse field keeps the one extra
/// pass cheap on a job with a hundred thousand motions.
pub const CHECK_CELL_MM: f64 = 0.4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WarningKind {
    /// The shaft or the holder starts below the material's surface where the
    /// cutter is, so the part that cannot cut would be inside the job.
    AssemblyBelowSurface,
    /// A rapid move's path passes below the surface, so it would cut or crash.
    RapidThroughMaterial,
}

impl WarningKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::AssemblyBelowSurface => "assembly below the surface",
            Self::RapidThroughMaterial => "rapid through material",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Warning {
    pub kind: WarningKind,
    /// First motion that shows the problem, so the display can seek to it.
    pub motion: usize,
    /// How far the part or the rapid is below the material surface at that
    /// motion, in mm.
    pub depth_mm: f64,
    /// The worst intrusion anywhere in the program. A holder inside the material
    /// on forty consecutive passes is one problem, not forty: the row names the
    /// motion where it starts and how deep it gets.
    pub max_depth_mm: f64,
    /// The tool index the motion uses, so a panel can name the tool.
    pub tool: usize,
}

/// Everything the checks need. `assemblies` is parallel to `tools`; a default
/// entry means that tool states nothing about how it is held.
pub struct CheckInput<'a> {
    pub motions: &'a [Motion],
    pub tools: &'a [ToolSpec],
    pub assemblies: &'a [ToolAssembly],
    pub holder: &'a [HolderSegment],
    pub stock: Stock,
}

/// One body to keep clear of the material: a radius and the z its own bottom
/// face sits at, in setup coordinates.
struct Body {
    radius_mm: f64,
    bottom_z_mm: f64,
}

/// The bodies of one tool at one tip position. A tool that states no stickout
/// contributes nothing above the cutter: nothing says where a holder would be.
fn bodies(
    tool: ToolSpec,
    assembly: ToolAssembly,
    holder: &[HolderSegment],
    tip_z: f64,
) -> Vec<Body> {
    let mut bodies = Vec::new();
    let cutting_top = match tool.normalize() {
        Ok(NormalizedTool::Knife) => match tool {
            ToolSpec::Knife { max_cut_depth, .. } => tip_z + max_cut_depth,
            _ => return bodies,
        },
        Ok(_) => match tool {
            ToolSpec::Endmill { cutting_length, .. } => tip_z + cutting_length,
            ToolSpec::Vbit { height, .. } => tip_z + height,
            ToolSpec::Knife { .. } => return bodies,
        },
        Err(_) => return bodies,
    };
    let Some(stickout) = assembly.stickout_mm else {
        return bodies;
    };
    let holder_base = tip_z + stickout;
    if let Some(diameter) = assembly.shaft_diameter_mm
        && holder_base > cutting_top
    {
        // The shaft runs between the flutes and the holder, so the lowest point
        // it could touch material with is the top of the cutting portion, not
        // the holder's underside.
        bodies.push(Body {
            radius_mm: diameter / 2.,
            bottom_z_mm: cutting_top,
        });
    }
    let mut z = holder_base;
    for segment in holder {
        let radius = segment.lower_diameter_mm.max(segment.upper_diameter_mm) / 2.;
        bodies.push(Body {
            radius_mm: radius,
            bottom_z_mm: z,
        });
        z += segment.height_mm;
    }
    bodies
}

struct Raster {
    field: crate::sim::Field,
    stock: Stock,
}

impl Raster {
    /// Height of the material's remaining top surface at a point, in setup
    /// coordinates (zero is the stock top, cutting is negative). Cells nobody
    /// has cut are still at the top.
    fn surface_z(&self, x: f64, y: f64) -> Option<f64> {
        let col = ((x - self.stock.x0) / self.field.cell).floor();
        let row = ((y - self.stock.y0) / self.field.cell).floor();
        if col < 0. || row < 0. {
            return None;
        }
        let (col, row) = (col as usize, row as usize);
        if col >= self.field.cols || row >= self.field.rows {
            return None;
        }
        let (level, _, _) = self.field.cell_at(col, row);
        Some(-(level as f64) * self.field.quantum)
    }

    /// The deepest intrusion of one body at one tip position: how far material
    /// stands above the body's own bottom face, over the body's footprint. Zero
    /// when the body clears the material everywhere.
    fn intrusion(&self, body: &Body, centre: [f64; 2]) -> f64 {
        let mut worst: f64 = 0.;
        // The centre and a ring at the body's radius: a wall beside the tool is
        // exactly the case a centre sample would miss.
        let samples = std::iter::once([0., 0.]).chain((0..8).map(|i| {
            let angle = i as f64 / 8. * std::f64::consts::TAU;
            [angle.cos() * body.radius_mm, angle.sin() * body.radius_mm]
        }));
        for offset in samples {
            let Some(surface) = self.surface_z(centre[0] + offset[0], centre[1] + offset[1]) else {
                // Outside the raster is outside the stock: nothing to hit.
                continue;
            };
            worst = worst.max(surface - body.bottom_z_mm);
        }
        worst
    }
}

/// Run both checks over the whole program. The raster is built as the pass goes,
/// so each motion is judged against the material that is actually there when the
/// machine starts it.
pub fn run(input: CheckInput<'_>) -> Vec<Warning> {
    if input.motions.is_empty() || input.tools.is_empty() {
        return Vec::new();
    }
    let cell = check_cell_mm(input.stock);
    let Ok(field) = crate::sim::Field::new(input.stock, input.tools, cell) else {
        return Vec::new();
    };
    let mut raster = Raster {
        field,
        stock: input.stock,
    };
    let mut warnings = Vec::new();
    for (index, motion) in input.motions.iter().enumerate() {
        let fallback = ToolSpec::Endmill {
            diameter: 1.,
            cutting_length: 1.,
        };
        let tool = input.tools.get(motion.tool).copied().unwrap_or(fallback);
        let assembly = input
            .assemblies
            .get(motion.tool)
            .copied()
            .unwrap_or_default();
        // Sampled positions along the move: the ends and the middle, which is
        // where a long pass or a ramp spends most of its length.
        let points: Vec<([f64; 3], [f64; 2])> = [0., 0.5, 1.]
            .iter()
            .map(|fraction| {
                let point = motion.point_at(*fraction);
                (point, [point[0], point[1]])
            })
            .collect();
        // A move the machine makes at feed is where the assembly has to clear
        // the material; a rapid is the other check. Gating on the interpolation
        // rather than the material effect is what lets a knife stage be checked
        // too: its cuts are feeds, and its lifted moves are rapids.
        if motion.interpolation == crate::sim::Interpolation::Feed {
            for (point, xy) in &points {
                let intrusion = bodies(tool, assembly, input.holder, point[2])
                    .iter()
                    .map(|body| raster.intrusion(body, *xy))
                    .fold(0., f64::max);
                if intrusion > 0. {
                    warnings.push(Warning {
                        kind: WarningKind::AssemblyBelowSurface,
                        motion: index,
                        depth_mm: intrusion,
                        max_depth_mm: intrusion,
                        tool: motion.tool,
                    });
                    break;
                }
            }
        } else {
            // A pure retract starts where the tool already is and leaves along
            // its own path: it cannot sweep through material, so it is not a
            // crash. Everything else — a lateral move at depth, or a descent —
            // is judged against the surface at every sample.
            let lateral = motion.x0 != motion.x1 || motion.y0 != motion.y1;
            let climbing = motion.z1 > motion.z0;
            let mut worst: f64 = 0.;
            if lateral || !climbing {
                for (point, xy) in &points {
                    if let Some(surface) = raster.surface_z(xy[0], xy[1]) {
                        worst = worst.max(surface - point[2]);
                    }
                }
            }
            if worst > 0. {
                warnings.push(Warning {
                    kind: WarningKind::RapidThroughMaterial,
                    motion: index,
                    depth_mm: worst,
                    max_depth_mm: worst,
                    tool: motion.tool,
                });
            }
        }
        // The motion joins the raster after it has been judged: a move is judged
        // against what is there when the machine starts it.
        if raster.field.apply(motion, 0., 1.).is_err() {
            break;
        }
    }
    // One row per problem: the first motion that shows it, carrying the worst
    // depth the program reaches. Forty consecutive passes with the holder inside
    // the material are one problem, not forty.
    let mut problems: Vec<Warning> = Vec::new();
    for warning in warnings {
        match problems
            .iter_mut()
            .find(|existing| existing.kind == warning.kind && existing.tool == warning.tool)
        {
            Some(existing) => {
                existing.max_depth_mm = existing.max_depth_mm.max(warning.depth_mm);
            }
            None => problems.push(warning),
        }
    }
    problems
}

/// The raster the checks ran on, so a panel can say what resolution its claim
/// rests on instead of letting a coarse estimate look exact (D10).
pub fn check_cell_mm(stock: Stock) -> f64 {
    CHECK_CELL_MM.max((stock.x1 - stock.x0).max(stock.y1 - stock.y0) / 1024.)
}

/// The raster's tile size, so the check's memory cost stays visible next to the
/// number it is derived from.
#[allow(dead_code)]
pub fn cell_limit() -> usize {
    TILE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::{Interpolation, ToolSpec};

    fn stock(thickness: f64) -> Stock {
        Stock {
            x0: -30.,
            y0: -30.,
            x1: 30.,
            y1: 30.,
            thickness_mm: thickness,
        }
    }

    fn cut(tool: usize, z: f64) -> Motion {
        Motion {
            kind: "cut".into(),
            tool,
            stage: 0,
            interpolation: Interpolation::Feed,
            feed_mm_min: Some(600.),
            x0: -5.,
            y0: 0.,
            z0: z,
            x1: 5.,
            y1: 0.,
            z1: z,
        }
    }

    fn rapid(tool: usize, z: f64) -> Motion {
        Motion {
            kind: "rapid_xy".into(),
            tool,
            stage: 0,
            interpolation: Interpolation::Rapid,
            feed_mm_min: None,
            x0: -5.,
            y0: 0.,
            z0: z,
            x1: 5.,
            y1: 0.,
            z1: z,
        }
    }

    fn endmill() -> ToolSpec {
        ToolSpec::Endmill {
            diameter: 6.,
            cutting_length: 12.,
        }
    }

    /// A short stickout puts the holder inside the job; a long one clears it.
    #[test]
    fn a_holder_that_would_run_into_the_job_is_reported() {
        let holder = cam_core::post::HolderSelection::catalogue("er20")
            .body()
            .unwrap();
        let tools = [endmill()];
        let clear = [ToolAssembly {
            shaft_diameter_mm: Some(6.),
            stickout_mm: Some(20.),
        }];
        let warnings = run(CheckInput {
            motions: &[cut(0, -6.)],
            tools: &tools,
            assemblies: &clear,
            holder: &holder,
            stock: stock(10.),
        });
        assert!(
            warnings.is_empty(),
            "a 20 mm stickout clears a 6 mm cut: {warnings:?}"
        );

        // A 4 mm stickout puts the nut's bottom face at z = -2 (tip -6 + 4),
        // which is 2 mm below the stock top. The cutter has cleared a 6 mm band
        // around the axis, but the nut is 25 mm across, and where it reaches past
        // that band the material is untouched at z = 0: 2 mm stands above it.
        let tight = [ToolAssembly {
            shaft_diameter_mm: Some(6.),
            stickout_mm: Some(4.),
        }];
        let warnings = run(CheckInput {
            motions: &[cut(0, -6.)],
            tools: &tools,
            assemblies: &tight,
            holder: &holder,
            stock: stock(10.),
        });
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::AssemblyBelowSurface);
        assert_eq!(warnings[0].motion, 0);
        assert!(
            (warnings[0].max_depth_mm - 2.).abs() < CHECK_CELL_MM,
            "the nut is {} mm inside the material",
            warnings[0].max_depth_mm
        );
    }

    /// A tool that states no stickout says nothing about its holder, so there is
    /// nothing to check and nothing is reported.
    #[test]
    fn an_unstated_assembly_is_not_checked() {
        let holder = cam_core::post::HolderSelection::catalogue("er20")
            .body()
            .unwrap();
        let tools = [endmill()];
        let warnings = run(CheckInput {
            motions: &[cut(0, -8.)],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &holder,
            stock: stock(10.),
        });
        assert!(warnings.is_empty(), "{warnings:?}");
    }

    /// A rapid at clearance height is fine; the same rapid at cutting depth is
    /// the classic crash, and it is reported with the motion it happens at.
    #[test]
    fn a_rapid_through_material_is_reported_where_it_happens() {
        let tools = [endmill()];
        let warnings = run(CheckInput {
            motions: &[rapid(0, 5.)],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert!(warnings.is_empty(), "clearance height: {warnings:?}");

        let warnings = run(CheckInput {
            motions: &[rapid(0, -2.)],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::RapidThroughMaterial);
        assert!((warnings[0].depth_mm - 2.).abs() < CHECK_CELL_MM);
    }

    /// A rapid inside material that an earlier move has already removed is not a
    /// crash: the raster is built as the pass goes, so each motion is judged
    /// against the material left at that point in the program.
    #[test]
    fn a_rapid_is_judged_against_the_material_left_at_that_point() {
        let tools = [endmill()];
        let warnings = run(CheckInput {
            motions: &[cut(0, -5.), rapid(0, -4.)],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert!(
            warnings.is_empty(),
            "the trench is already cut: {warnings:?}"
        );

        let warnings = run(CheckInput {
            motions: &[rapid(0, -4.)],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }

    /// A retract starts where the tool already is — usually at the bottom of its
    /// own cut — and leaves along its own path, so it is not a crash. A lateral
    /// rapid from the same place is.
    #[test]
    fn a_retract_is_not_a_rapid_through_material_but_a_lateral_move_is() {
        let tools = [endmill()];
        let retract = Motion {
            kind: "rapid_xy".into(),
            tool: 0,
            stage: 0,
            interpolation: Interpolation::Rapid,
            feed_mm_min: None,
            x0: 0.,
            y0: 0.,
            z0: -3.,
            x1: 0.,
            y1: 0.,
            z1: 5.,
        };
        let warnings = run(CheckInput {
            motions: std::slice::from_ref(&retract),
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert!(
            warnings.is_empty(),
            "a retract leaves the cut: {warnings:?}"
        );

        let lateral = Motion {
            z1: -3.,
            x1: 5.,
            ..retract.clone()
        };
        let warnings = run(CheckInput {
            motions: &[lateral],
            tools: &tools,
            assemblies: &[ToolAssembly::default()],
            holder: &[],
            stock: stock(10.),
        });
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert_eq!(warnings[0].kind, WarningKind::RapidThroughMaterial);
    }
}
