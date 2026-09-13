//! Same display setup policy as the retired TypeScript simulator
//! (the former `web/src/sim/setup.ts`), using released projections.
use cam_core::{
    job::{Job, ToolGeometry},
    vcarve::CombinedPlan,
};
use cam_gui_runtime::sim::{Input, Motion, Stock, ToolSpec, choose_resolution};
use cam_service::inspection::SliceInfo;

pub fn build(job: &Job, plan: &CombinedPlan, slices: &[SliceInfo]) -> Result<Input, String> {
    let mut ids = Vec::new();
    let mut tools = Vec::new();
    for id in [&job.operation.endmill_id, &job.operation.vbit_id] {
        if ids.contains(id) {
            continue;
        }
        let Some(geometry) = job
            .tools
            .iter()
            .find(|t| t.id == *id)
            .and_then(|t| t.geometry.as_ref())
        else {
            continue;
        };
        tools.push(match geometry {
            ToolGeometry::Endmill(t) => ToolSpec::Endmill {
                diameter: t.diameter_mm,
            },
            ToolGeometry::Vbit(t) => ToolSpec::Vbit {
                angle: t.included_angle_deg,
                tip: t.tip_diameter_mm,
                diameter: t.max_cutting_diameter_mm,
                height: t.cutting_height_mm,
            },
        });
        ids.push(id.clone());
    }
    if tools.is_empty() {
        return Err("No simulator tool geometry".into());
    }
    let motions = plan
        .endmill
        .motions
        .iter()
        .chain(&plan.vbit_motions)
        .map(|m| {
            let tool = ids
                .iter()
                .position(|id| *id == m.tool_id)
                .ok_or("Unknown simulation motion tool")?;
            Ok(Motion {
                kind: serde_json::to_value(m.kind)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .into(),
                tool,
                x0: m.start.x,
                y0: m.start.y,
                z0: m.start.z,
                x1: m.end.x,
                y1: m.end.y,
                z1: m.end.z,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for s in slices {
        if let Some(bounds) = s
            .regions
            .iter()
            .find(|r| r.key == "nominalTarget")
            .and_then(|r| r.bounds.as_ref())
        {
            b[0] = b[0].min(bounds.min.x);
            b[1] = b[1].min(bounds.min.y);
            b[2] = b[2].max(bounds.max.x);
            b[3] = b[3].max(bounds.max.y);
        }
    }
    if !b[0].is_finite() {
        for m in &motions {
            b[0] = b[0].min(m.x0).min(m.x1);
            b[1] = b[1].min(m.y0).min(m.y1);
            b[2] = b[2].max(m.x0).max(m.x1);
            b[3] = b[3].max(m.y0).max(m.y1);
        }
    }
    let mut radius = 0_f64;
    let mut detail = f64::INFINITY;
    for t in &tools {
        match *t {
            ToolSpec::Knife { .. } => {
                unreachable!("this fixture adapter only builds milling tools")
            }
            ToolSpec::Endmill { diameter } => {
                radius = radius.max(diameter / 2.);
                detail = detail.min(diameter);
            }
            ToolSpec::Vbit { tip, diameter, .. } => {
                radius = radius.max(tip.max(diameter) / 2.);
                detail = detail.min(tip);
            }
        }
    }
    let resolution = choose_resolution(b[2] - b[0], b[3] - b[1], detail, 8192., 64_000_000.)?;
    let cell = resolution.cell_mm;
    let margin = radius + 1.;
    let cols = ((b[2] - b[0] + 2. * margin) / cell).ceil();
    let rows = ((b[3] - b[1] + 2. * margin) / cell).ceil();
    let x0 = (b[0] + b[2]) / 2. - cols * cell / 2.;
    let y0 = (b[1] + b[3]) / 2. - rows * cell / 2.;
    Ok(Input {
        stock: Stock {
            x0,
            y0,
            x1: x0 + cols * cell,
            y1: y0 + rows * cell,
            thickness_mm: job.stock.thickness_mm.ok_or("Missing stock thickness")?,
        },
        tools,
        resolution,
        prefixes: vec![
            0,
            plan.endmill.motions.len(),
            motions.len(),
            plan.endmill.motions.len() / 2,
            motions.len(),
        ],
        motions,
    })
}
