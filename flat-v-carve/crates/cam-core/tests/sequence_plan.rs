//! A2 sequence contract: adapted Flat V-carve plans are equivalent to the
//! legacy planners' motion geometry, order and completeness, and independent
//! operations keep distinct identities.
use cam_core::{
    job::Job as LegacyJob,
    motion::{Motion, MotionKind},
    pocket::plan_endmill,
    project::CamJob,
    project::FlatVcarveSettings,
    project::OperationSettings,
    project::migrate::{migrate_job, migrate_legacy_json},
    sequence::{
        ExecutionItem, GenerationStatus, OperationPlan, PlanLimits, ProcessSpindle, StageRole,
    },
    toolpath::{Interpolation, MotionEffect, PlannedMotion},
    vcarve::plan_combined,
};

const M3_RECTANGLE: &str = include_str!("../../../fixtures/m3/rectangle.json");
const M4_CONTACT_LINE: &str = include_str!("../../../fixtures/m4/contact-line.json");
const M4_RESOURCE_LIMIT: &str = include_str!("../../../fixtures/m4/resource-limit.json");
const FLOWER_COMBINED: &str = include_str!("../../../../real_data/flower_box-svg.job-real.json");

fn flat_vcarve(job: &CamJob) -> &FlatVcarveSettings {
    job.operations
        .iter()
        .find_map(|op| match &op.settings {
            OperationSettings::FlatVcarve(settings) => Some(settings),
            _ => None,
        })
        .unwrap()
}

fn operation_id(job: &CamJob) -> &str {
    &job.operations[0].id
}

/// The legacy endmill slot-level plunge flag is redundant with the spec and
/// jobs spell it either way (`null` or the spec value). Drop it from both
/// sides before comparing reconstructed and original legacy jobs.
fn canonical_legacy_json(job: &serde_json::Value) -> serde_json::Value {
    let mut value = job.clone();
    if let Some(tools) = value.get_mut("tools").and_then(|t| t.as_array_mut()) {
        for tool in tools {
            let is_endmill = tool
                .get("geometry")
                .and_then(|g| g.get("kind"))
                .and_then(|k| k.as_str())
                == Some("endmill");
            if is_endmill && let Some(object) = tool.as_object_mut() {
                object.remove("plunge_capable");
            }
        }
    }
    value
}

/// The adapter must reconstruct a legacy job equivalent to the one that was
/// migrated: same selection, controls and tool snapshots.
#[test]
fn adapter_reconstructs_the_migrated_legacy_job() {
    for json in [M3_RECTANGLE, M4_CONTACT_LINE, FLOWER_COMBINED] {
        let legacy = LegacyJob::from_json(json).unwrap();
        let cam = migrate_job(&legacy).unwrap();
        let reconstructed = cam_core::operations::flat_vcarve::to_legacy_job(
            &cam,
            operation_id(&cam),
            flat_vcarve(&cam),
        )
        .unwrap();
        assert_eq!(
            canonical_legacy_json(&serde_json::to_value(&reconstructed).unwrap()),
            canonical_legacy_json(&serde_json::to_value(&legacy).unwrap()),
            "adapter legacy job must equal the migrated source"
        );
    }
}

/// One legacy planning pass; returns the concatenated motion stream, the
/// index where the V-bit stage starts (equal to length for endmill-only)
/// and whether expected finish paths were not all executed.
fn legacy_reference(legacy: &LegacyJob) -> (Vec<Motion>, usize, bool) {
    if legacy.vbit_planning.is_some() {
        let plan = plan_combined(legacy).unwrap();
        let finish_start = plan.endmill.motions.len();
        let shortfall = plan.analysis.finish_paths_expected != plan.analysis.finish_paths_executed;
        let mut motions = plan.endmill.motions.clone();
        motions.extend(plan.vbit_motions.clone());
        (motions, finish_start, shortfall)
    } else {
        (plan_endmill(legacy).unwrap().motions, usize::MAX, false)
    }
}

fn expected_interpolation(kind: MotionKind) -> Interpolation {
    if kind.rapid() {
        Interpolation::Rapid
    } else {
        Interpolation::LinearFeed
    }
}

fn expected_effect(kind: MotionKind) -> MotionEffect {
    if kind.cutting() {
        MotionEffect::MillingSweep
    } else {
        MotionEffect::None
    }
}

fn check_equivalence(json: &str) -> (OperationPlan, GenerationStatus, usize) {
    let legacy = LegacyJob::from_json(json).unwrap();
    let cam = migrate_job(&legacy).unwrap();
    let plan = OperationPlan::plan_job(&cam, &PlanLimits::default()).unwrap();
    let (legacy_motions, finish_start, shortfall) = legacy_reference(&legacy);
    assert_eq!(
        plan.motions.len(),
        legacy_motions.len(),
        "motion count must match the legacy engine"
    );
    let pass_count = plan.operation_results[0].legacy_pass_evidence.len().max(1);
    for (planned, legacy) in plan.motions.iter().zip(legacy_motions.iter()) {
        assert_eq!(planned.start, legacy.start, "start must match in order");
        assert_eq!(planned.end, legacy.end, "end must match in order");
        assert_eq!(planned.layer, legacy.layer, "layer must match");
        assert_eq!(planned.feed_mm_min, legacy.feed_mm_min);
        assert_eq!(planned.interpolation, expected_interpolation(legacy.kind));
        assert_eq!(planned.effect, expected_effect(legacy.kind));
        // Feed moves carry an explicit feed under the new contract.
        if planned.interpolation == Interpolation::LinearFeed {
            assert!(
                planned.feed_mm_min.is_some(),
                "linear feed motion {} must carry a feed",
                planned.id
            );
        }
        let expected_stage = if planned.id < finish_start {
            format!("{}-vcarve-rough", planned.operation_id)
        } else {
            format!("{}-vcarve-finish", planned.operation_id)
        };
        assert_eq!(planned.stage_id, expected_stage);
        assert!(
            planned.pass_id < pass_count,
            "pass ids stay in evidence range"
        );
        if planned.id < finish_start {
            assert_eq!(planned.pass_id, 0, "endmill motions use pass 0");
        }
    }
    let _ = shortfall;
    let status = plan.operation_results[0].generation_status;
    (plan, status, finish_start)
}

#[test]
fn migrated_m4_and_flower_plans_match_legacy_geometry_order_and_completeness() {
    for json in [M3_RECTANGLE, M4_CONTACT_LINE, FLOWER_COMBINED] {
        let (plan, status, finish_start) = check_equivalence(json);
        assert_eq!(
            status,
            GenerationStatus::Complete,
            "clean fixtures plan completely"
        );
        // Stage and execution assembly: nonempty stages only, in order, each
        // preceded by a tool change (when the tool changes) and process intent.
        let legacy = LegacyJob::from_json(json).unwrap();
        let rough_present = legacy.vbit_planning.is_none() || finish_start > 0;
        let finish_present = legacy.vbit_planning.is_some() && plan.motions.len() > finish_start;
        let expected_stages = rough_present as usize + finish_present as usize;
        assert_eq!(plan.stages.len(), expected_stages);
        assert_eq!(plan.execution.len(), 3 * expected_stages);
        if rough_present && finish_present {
            assert_eq!(plan.stages[0].role, StageRole::VcarveRough);
            assert_eq!(plan.stages[1].role, StageRole::VcarveFinish);
            assert_eq!(
                plan.stages[0].motion_range.1, plan.stages[1].motion_range.0,
                "finish stage continues exactly after rough"
            );
        }
        if let Some(last) = plan.stages.last() {
            assert_eq!(last.motion_range.1, plan.motions.len());
        }
        let mut expected_items = vec![];
        let mut tool: Option<&String> = None;
        for stage in &plan.stages {
            if tool != Some(&stage.tool_id) {
                expected_items.push("tool_change");
            }
            tool = Some(&stage.tool_id);
            expected_items.push("intent");
            expected_items.push("run_stage");
        }
        let actual: Vec<&str> = plan
            .execution
            .iter()
            .map(|item| match item {
                ExecutionItem::ToolChange { .. } => "tool_change",
                ExecutionItem::SetProcessIntent { .. } => "intent",
                ExecutionItem::RunStage { .. } => "run_stage",
            })
            .collect();
        assert_eq!(actual, expected_items);
        // Every motion belongs to exactly one stage with its stage's tool.
        for motion in &plan.motions {
            let owning: Vec<&cam_core::sequence::ExecutionStage> = plan
                .stages
                .iter()
                .filter(|s| motion.id >= s.motion_range.0 && motion.id < s.motion_range.1)
                .collect();
            assert_eq!(
                owning.len(),
                1,
                "motion {} has exactly one stage",
                motion.id
            );
            assert_eq!(&motion.tool_id, &owning[0].tool_id);
        }
        // Migrated assignments keep spindle direction unresolved; the plan
        // records that as an export preparation requirement, not state.
        assert!(
            plan.preparation_requirements
                .iter()
                .all(|r| r.code == "PROCESS_SPINDLE_DIRECTION")
        );
        assert!(!plan.preparation_requirements.is_empty());
        assert!(plan.execution.iter().all(
            |item| !matches!(item, ExecutionItem::SetProcessIntent { intent } if matches!(
                intent.spindle,
                ProcessSpindle::Off
            ))
        ));
    }
}

#[test]
fn vbit_finish_motions_carry_execution_pass_ids() {
    let (plan, _, finish_start) = check_equivalence(M4_CONTACT_LINE);
    let result = &plan.operation_results[0];
    assert!(!result.legacy_pass_evidence.is_empty());
    for evidence in &result.legacy_pass_evidence {
        assert!(evidence.end_motion_id > evidence.first_motion_id);
    }
    let finish_stage = plan
        .stages
        .iter()
        .find(|s| s.role == StageRole::VcarveFinish)
        .expect("combined plan has a finish stage");
    let finish_motions: Vec<&PlannedMotion> = plan
        .motions
        .iter()
        .filter(|m| m.id >= finish_stage.motion_range.0)
        .collect();
    assert_eq!(finish_motions.len(), plan.motions.len() - finish_start);
    let max_pass = finish_motions.iter().map(|m| m.pass_id).max().unwrap();
    assert!(
        max_pass < result.legacy_pass_evidence.len().max(1),
        "finish pass ids stay within recorded executions"
    );
}

#[test]
fn resource_limit_fixture_is_inconclusive_not_truncated_success() {
    let legacy = LegacyJob::from_json(M4_RESOURCE_LIMIT).unwrap();
    let cam = migrate_job(&legacy).unwrap();
    let plan = OperationPlan::plan_job(&cam, &PlanLimits::default()).unwrap();
    let status = plan.operation_results[0].generation_status;
    assert!(
        status == GenerationStatus::Inconclusive || status == GenerationStatus::Incomplete,
        "budget exhaustion must not read as complete, got {status:?}"
    );
    assert!(!plan.generation_diagnostics.is_empty());
}

#[test]
fn two_independent_operations_keep_distinct_identities() {
    let cam = migrate_legacy_json(M3_RECTANGLE).unwrap();
    let mut two = cam.clone();
    let first = two.operations[0].clone();
    let mut second = first.clone();
    second.id = "carve-2".into();
    second.name = "Second carve".into();
    // Duplicating copies settings under a new ID; the two selections may even
    // overlap because A2 conservatively plans each against original stock.
    two.operations.push(second);

    let plan = OperationPlan::plan_job(&two, &PlanLimits::default()).unwrap();
    assert_eq!(plan.operation_results.len(), 2);
    let ids: Vec<&String> = plan
        .operation_results
        .iter()
        .map(|r| &r.operation_id)
        .collect();
    assert_eq!(ids, [&first.id, &"carve-2".to_string()]);

    // Stage IDs are unique and namespaced by operation.
    let stage_ids: Vec<&String> = plan.stages.iter().map(|s| &s.stage_id).collect();
    let mut unique = stage_ids.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(stage_ids.len(), unique.len(), "stage ids must be unique");
    assert!(
        plan.stages
            .iter()
            .all(|s| s.stage_id.starts_with(&s.operation_id))
    );

    // Global motion IDs are unique and dense; each operation owns one
    // contiguous global range, offset by everything planned before it.
    let mut motion_ids: Vec<usize> = plan.motions.iter().map(|m| m.id).collect();
    let total = motion_ids.len();
    motion_ids.sort();
    motion_ids.dedup();
    assert_eq!(motion_ids.len(), total);
    let first_len = plan
        .motions
        .iter()
        .filter(|m| m.operation_id == first.id)
        .count();
    for (index, operation_id) in [&first.id, "carve-2"].iter().enumerate() {
        let base = if index == 0 { 0 } else { first_len };
        let owned: Vec<usize> = plan
            .motions
            .iter()
            .filter(|m| m.operation_id == *operation_id)
            .map(|m| m.id)
            .collect();
        assert!(!owned.is_empty());
        let expected: Vec<usize> = (base..base + owned.len()).collect();
        assert_eq!(
            owned, expected,
            "operation owns a contiguous global id range"
        );
    }

    // A recurring tool is re-selected in execution order; motions are not
    // regrouped by tool.
    let tools_in_order: Vec<&str> = plan.stages.iter().map(|s| s.tool_id.as_str()).collect();
    assert_eq!(tools_in_order, ["endmill", "endmill"]);
    let changes = plan
        .execution
        .iter()
        .filter(|i| matches!(i, ExecutionItem::ToolChange { .. }))
        .count();
    assert_eq!(
        changes, 1,
        "consecutive same-tool stages share one tool change"
    );

    // Identity: a plan of one operation differs from the plan of two.
    let single = OperationPlan::plan_job(&cam, &PlanLimits::default()).unwrap();
    assert_ne!(single.execution_fingerprint, plan.execution_fingerprint);
    assert_ne!(single.input_fingerprint, plan.input_fingerprint);
}

#[test]
fn missing_machining_fields_yield_incomplete_result_not_silent_skip() {
    let cam = migrate_legacy_json(M4_CONTACT_LINE).unwrap();
    let mut incomplete = cam.clone();
    let OperationSettings::FlatVcarve(settings) = &mut incomplete.operations[0].settings else {
        panic!("flat vcarve settings expected");
    };
    settings.max_depth_mm = None;
    settings.endmill.spindle_rpm = None;
    let plan = OperationPlan::plan_job(&incomplete, &PlanLimits::default()).unwrap();
    let result = &plan.operation_results[0];
    assert_eq!(result.generation_status, GenerationStatus::Incomplete);
    assert!(result.stage_ids.is_empty());
    assert!(plan.motions.is_empty());
    let missing: Vec<&cam_core::sequence::PlanIssue> = plan
        .generation_diagnostics
        .iter()
        .filter(|d| d.code == "MISSING_MACHINING_SETTING")
        .collect();
    assert!(missing.len() >= 2, "both cleared fields are reported");
    assert!(
        missing
            .iter()
            .all(|d| d.message.contains("max_depth_mm") || d.message.contains("spindle_rpm"))
    );
}

#[test]
fn unsupported_enabled_operations_are_rejected_with_a_specific_diagnostic() {
    let mut cam = migrate_legacy_json(M4_CONTACT_LINE).unwrap();
    // Profile planning ships in a later slice; an enabled profile must be
    // rejected specifically, never silently skipped. (Face is planned since C1.)
    cam.operations.push(cam_core::project::Operation {
        id: "profile-1".into(),
        name: "Cutout".into(),
        enabled: true,
        settings: OperationSettings::Profile(cam_core::project::ProfileSettings {
            contours: vec![],
            assignment: cam_core::project::MillingAssignment {
                tool_id: "endmill".into(),
                spindle_rpm: None,
                spindle_direction: None,
                cutting_feed_mm_min: None,
                plunge_feed_mm_min: None,
                max_stepdown_mm: None,
                stepover_mm: None,
            },
            top: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: 0.,
            },
            bottom: cam_core::project::HeightRef {
                reference: cam_core::project::HeightReference::StockTop,
                offset_mm: -1.,
            },
            stepdown_mm: None,
            through_cut_allowance_mm: None,
            direction: None,
            order: Default::default(),
            start: Default::default(),
            finish: Default::default(),
            entry: Default::default(),
            lead_in: Default::default(),
            lead_out: Default::default(),
            tabs: None,
        }),
    });
    let err = OperationPlan::plan_job(&cam, &PlanLimits::default()).unwrap_err();
    assert_eq!(err.code, "OPERATION_PLANNER_UNAVAILABLE");
    assert!(err.message.contains("profile-1"), "{}", err.message);
    assert!(err.message.contains("profile"), "{}", err.message);

    // Disabled unsupported operations are excluded from planning readiness.
    cam.operations[1].enabled = false;
    OperationPlan::plan_job(&cam, &PlanLimits::default()).unwrap();
}
