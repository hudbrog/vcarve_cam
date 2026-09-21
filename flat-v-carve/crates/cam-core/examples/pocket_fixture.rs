//! Reproduce the portable Pocket demonstration (no machine profile is applied).
use cam_core::project::{
    self,
    v5::{
        self, OperationSettingsV5,
        commands::{self, NewOperationKind},
    },
};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    let measured_svg = args
        .windows(2)
        .find(|a| a[0] == "--measure-svg")
        .map(|a| a[1].clone());
    let svg = measured_svg.as_ref().map(|p| std::fs::read_to_string(p).unwrap()).unwrap_or_else(|| r#"<svg xmlns="http://www.w3.org/2000/svg" width="80mm" height="40mm" viewBox="0 0 80 40"><path id="island-pocket" fill-rule="evenodd" d="M3 3H37V37H3Z M14 14H26V26H14Z"/><rect id="second-pocket" x="47" y="6" width="25" height="28"/></svg>"#.into());
    let mut job = commands::add_operation(
        &v5::authoring::from_svg("pocket-demo.svg".into(), svg, 0.001).unwrap(),
        NewOperationKind::Pocket,
        "pocket",
        "Two pockets with an island",
    )
    .unwrap()
    .job;
    let catalogue = v5::inspect_artwork(&job).unwrap();
    let picks: Vec<_> = catalogue.items[0]
        .entries
        .iter()
        .filter(|e| e.kind == v5::GeometryRefKind::FilledComponent)
        .map(|e| v5::GeometryPick {
            artwork_item_id: e.reference.artwork_item_id.clone(),
            kind: e.kind,
            local_geometry_id: e.reference.local_geometry_id.clone(),
        })
        .collect();
    job = commands::set_component_selection(&job, "pocket", &picks)
        .unwrap()
        .job;
    job.name = "Pocket demonstration".into();
    job.tools[0].name = "4 mm flat endmill".into();
    job.tools[0].geometry = Some(project::ToolGeometry::Endmill(project::EndmillGeometry {
        diameter_mm: 4.,
        cutting_length_mm: 12.,
    }));
    job.tools[0].capabilities = project::ToolCapabilities {
        plunge_capable: Some(true),
        ramp_capable: Some(true),
    };
    let OperationSettingsV5::Pocket(s) = &mut job.operations[0].settings else {
        unreachable!()
    };
    s.bottom.offset_mm = -2.3;
    s.assignment.spindle_direction = Some(project::SpindleDirection::Clockwise);
    s.assignment.spindle_rpm = Some(10000.);
    s.assignment.cutting_feed_mm_min = Some(600.);
    s.assignment.plunge_feed_mm_min = Some(150.);
    s.assignment.max_stepdown_mm = Some(1.);
    s.assignment.stepover_mm = Some(2.);
    s.direction = Some(project::CutDirection::Climb);
    s.wall_allowance_mm = Some(0.2);
    s.finish_walls = true;
    s.finish_feed_mm_min = Some(400.);
    s.entry = v5::PocketEntry::Helix {
        radius_mm: Some(1.),
        max_angle_deg: Some(10.),
        feed_mm_min: Some(200.),
    };
    s.lead_in = project::LeadSpec::TangentArc {
        radius_mm: Some(0.2),
        sweep_deg: Some(90.),
        feed_mm_min: Some(250.),
    };
    s.lead_out = project::LeadSpec::TangentLine {
        length_mm: Some(0.2),
        feed_mm_min: Some(250.),
    };
    if measured_svg.is_some() {
        // Comparable constant-section clearing workload for a real drawing.
        s.bottom.offset_mm = -1.;
        s.assignment.stepover_mm = Some(1.25);
        s.wall_allowance_mm = Some(0.);
        s.finish_walls = false;
        s.entry = v5::PocketEntry::Plunge;
        s.lead_in = project::LeadSpec::None;
        s.lead_out = project::LeadSpec::None;
        job.tools[0].geometry = Some(project::ToolGeometry::Endmill(project::EndmillGeometry {
            diameter_mm: 2.5,
            cutting_length_mm: 12.,
        }));
    }
    if measured_svg.is_some() || args.iter().any(|a| a == "--measure") {
        let started = std::time::Instant::now();
        let plan = cam_core::sequence::OperationPlanV5::plan_job_v5(
            &job,
            &v5::ReadinessScope::AllEnabled,
            &Default::default(),
        )
        .unwrap();
        let planned_ms = started.elapsed().as_secs_f64() * 1000.;
        let checks = cam_core::checks::check_plan_v5(&plan).unwrap();
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({
            "drawing": measured_svg.unwrap_or_else(|| "two-pockets.job.json".into()),
            "planningMs": planned_ms, "planningAndChecksMs": started.elapsed().as_secs_f64() * 1000.,
            "motions": plan.motions.len(), "stages": plan.stages.len(),
            "results": plan.operation_results, "diagnostics": plan.generation_diagnostics,
            "checks": checks,
        })).unwrap());
        return;
    }
    println!("{}", job.to_json().unwrap());
}
