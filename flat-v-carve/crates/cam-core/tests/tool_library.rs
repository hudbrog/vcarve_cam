use cam_core::{
    job::Job,
    pocket::{EndmillPlan, plan_endmill},
    tool_library::{
        CuttingPreset, LibraryChange, LibraryTool, MAX_LIBRARY_BYTES, MAX_LIBRARY_REVISION,
        MAX_PRESETS_PER_TOOL, MAX_TOOLS, ToolLibrary, ToolSlot,
    },
};
use serde_json::{Value, json};

fn job() -> Job {
    Job::from_json(include_str!("../../../fixtures/m4/island.json")).unwrap()
}
fn tool(index: usize) -> LibraryTool {
    let settings = &job().tools[index];
    let mut tool = LibraryTool::from_settings(
        format!("saved-{index}"),
        "Synthetic cutter".into(),
        settings,
    )
    .unwrap();
    tool.cutting_presets.push(
        CuttingPreset::from_settings("test".into(), "Synthetic test values".into(), settings)
            .unwrap(),
    );
    tool
}
fn library() -> ToolLibrary {
    ToolLibrary {
        tools: vec![tool(0), tool(1)],
        ..ToolLibrary::default()
    }
}
fn value(value: &impl serde::Serialize) -> Value {
    serde_json::to_value(value).unwrap()
}

#[test]
fn strict_schema_rejects_unknown_duplicate_missing_and_future_fields() {
    let saved = library().to_json().unwrap();
    assert_eq!(
        ToolLibrary::from_json(&saved).unwrap().to_json().unwrap(),
        saved
    );
    for invalid in [
        saved.replacen("\"schema_version\": 1", "\"schema_version\": 99", 1),
        saved.replacen("\"revision\": 0", "\"revision\": 0, \"revision\": 1", 1),
        saved.replacen("\"revision\": 0", "\"revision\": 0, \"surprise\": true", 1),
        saved.replacen(
            "\"diameter_mm\": 4.0",
            "\"diameter_mm\": 4.0, \"extra\": 2",
            1,
        ),
        "{}".into(),
    ] {
        assert!(
            ToolLibrary::from_json(&invalid).is_err(),
            "accepted: {invalid}"
        );
    }
    assert!(
        LibraryChange::from_json(r#"{"kind":"remove_tool","tool_id":"saved-0","extra":0}"#)
            .is_err()
    );
}

#[test]
fn cutter_validation_is_shared_with_jobs_and_nonfinite_values_never_serialize_as_null() {
    let mut library = library();
    if let cam_core::tool_library::LibraryGeometry::Vbit(spec) = &mut library.tools[1].geometry {
        spec.cutting_height_mm = 100.;
    }
    assert_eq!(library.validate().unwrap_err().code, "INCONSISTENT_VBIT");
    library.tools[1] = tool(1);
    library.tools[0].plunge_capable = Some(false);
    assert_eq!(library.validate().unwrap_err().code, "JOB_TOOL_CAPABILITY");
    library.tools[0] = tool(0);
    for invalid in [0., -1., f64::NAN, f64::INFINITY] {
        library.tools[0].cutting_presets[0].spindle_rpm = Some(invalid);
        assert_eq!(library.to_json().unwrap_err().code, "JOB_PARAMETER");
    }
}

#[test]
fn library_ids_labels_and_nested_presets_are_validated() {
    let mut library = library();
    for invalid in ["../outside", "", "with space"] {
        library.tools[0].id = invalid.into();
        assert_eq!(library.validate().unwrap_err().code, "LIBRARY_ID");
    }
    library.tools[0] = tool(0);
    library.tools[0].name = " \n".into();
    assert_eq!(library.validate().unwrap_err().code, "LIBRARY_LABEL");
    library.tools[0] = tool(0);
    library.tools[0].cutting_presets[0].material = Some(" ".into());
    assert_eq!(library.validate().unwrap_err().code, "LIBRARY_LABEL");
    library.tools[0] = tool(0);
    library.tools[0]
        .cutting_presets
        .push(tool(0).cutting_presets.remove(0));
    assert_eq!(library.validate().unwrap_err().code, "LIBRARY_DUPLICATE_ID");
}

#[test]
fn tool_and_preset_crud_are_immutable_transactions() {
    let original = ToolLibrary::default();
    let added = original
        .changed(0, LibraryChange::AddTool { tool: tool(0) })
        .unwrap();
    assert!(original.tools.is_empty());
    let duplicated = added
        .changed(
            1,
            LibraryChange::DuplicateTool {
                tool_id: "saved-0".into(),
                new_id: "copy".into(),
                name: "Copy".into(),
            },
        )
        .unwrap();
    let mut replacement = duplicated.tool("copy").unwrap().clone();
    replacement.name = "Edited copy".into();
    replacement.cutting_presets.clear();
    let replaced = duplicated
        .changed(2, LibraryChange::ReplaceTool { tool: replacement })
        .unwrap();
    assert_eq!(replaced.tool("saved-0").unwrap().cutting_presets.len(), 1);
    let preset =
        CuttingPreset::from_settings("another".into(), "Another test".into(), &job().tools[0])
            .unwrap();
    let with_preset = replaced
        .changed(
            3,
            LibraryChange::AddPreset {
                tool_id: "copy".into(),
                preset: preset.clone(),
            },
        )
        .unwrap();
    let mut replacement_preset = preset;
    replacement_preset.spindle_rpm = None;
    let edited = with_preset
        .changed(
            4,
            LibraryChange::ReplacePreset {
                tool_id: "copy".into(),
                preset: replacement_preset,
            },
        )
        .unwrap();
    let cloned = edited
        .changed(
            5,
            LibraryChange::DuplicatePreset {
                tool_id: "copy".into(),
                preset_id: "another".into(),
                new_id: "third".into(),
                name: "Third test".into(),
            },
        )
        .unwrap();
    assert_eq!(
        cloned
            .tool("copy")
            .unwrap()
            .preset("third")
            .unwrap()
            .spindle_rpm,
        None
    );
    let removed = cloned
        .changed(
            6,
            LibraryChange::RemovePreset {
                tool_id: "copy".into(),
                preset_id: "another".into(),
            },
        )
        .unwrap();
    assert_eq!(removed.tool("copy").unwrap().cutting_presets.len(), 1);
    let deleted = removed
        .changed(
            7,
            LibraryChange::RemoveTool {
                tool_id: "copy".into(),
            },
        )
        .unwrap();
    assert_eq!(deleted.revision, 8);
    assert_eq!(deleted.tools.len(), 1);
    assert_eq!(duplicated.tool("copy").unwrap().name, "Copy");
}

#[test]
fn conflicts_bad_references_and_import_collisions_leave_original_unchanged() {
    let library = library();
    let before = library.to_json().unwrap();
    assert_eq!(
        library
            .changed(
                1,
                LibraryChange::RemoveTool {
                    tool_id: "saved-0".into()
                }
            )
            .unwrap_err()
            .code,
        "LIBRARY_CONFLICT"
    );
    for change in [
        LibraryChange::ReplaceTool {
            tool: LibraryTool {
                id: "absent".into(),
                ..tool(0)
            },
        },
        LibraryChange::RemoveTool {
            tool_id: "absent".into(),
        },
        LibraryChange::RemovePreset {
            tool_id: "saved-0".into(),
            preset_id: "absent".into(),
        },
        LibraryChange::ReplacePreset {
            tool_id: "absent".into(),
            preset: tool(0).cutting_presets.remove(0),
        },
    ] {
        assert_eq!(
            library.changed(0, change).unwrap_err().code,
            "LIBRARY_NOT_FOUND"
        );
    }
    let mut imported = ToolLibrary {
        tools: vec![LibraryTool {
            id: "new".into(),
            ..tool(0)
        }],
        revision: 98,
        ..ToolLibrary::default()
    };
    assert_eq!(
        library
            .changed(
                0,
                LibraryChange::Import {
                    library: imported.clone()
                }
            )
            .unwrap()
            .revision,
        1
    );
    imported.tools.push(tool(0));
    assert_eq!(
        library
            .changed(0, LibraryChange::Import { library: imported })
            .unwrap_err()
            .code,
        "LIBRARY_DUPLICATE_ID"
    );
    assert_eq!(library.to_json().unwrap(), before);
}

#[test]
fn applying_copies_snapshots_and_preserves_job_ids_machine_and_other_settings() {
    let mut original = job();
    original.tools[0].id = "job-local-mill".into();
    original.operation.endmill_id = "job-local-mill".into();
    original.machine_profile = Some(serde_json::from_value(json!({"id":"machine", "work_offset":"G54", "endmill_tool_number":7, "vbit_tool_number":8})).unwrap());
    original.tools.reverse(); // Role lookup must not depend on array order.
    let mut library = library();
    library.tools[0].cutting_presets[0].cutting_feed_mm_min = Some(444.);
    library.tools[0].cutting_presets[0].material = Some("Test material".into());
    let result = library
        .apply_to_job(&original, ToolSlot::Endmill, "saved-0", Some("test"))
        .unwrap();
    let mut expected = value(&original);
    expected["tools"][1]["cutting_feed_mm_min"] = json!(444.);
    assert_eq!(value(&result), expected);
    let saved = result.to_json().unwrap();
    library.tools.clear();
    assert_eq!(Job::from_json(&saved).unwrap().to_json().unwrap(), saved);
    assert_eq!(original.tools[1].cutting_feed_mm_min, Some(300.));
}

#[test]
fn no_preset_and_partial_presets_clear_previous_cutting_values_without_inference() {
    let mut library = library();
    let original = job();
    let blank = library
        .apply_to_job(&original, ToolSlot::Endmill, "saved-0", None)
        .unwrap();
    for field in [
        "spindle_rpm",
        "cutting_feed_mm_min",
        "plunge_feed_mm_min",
        "max_stepdown_mm",
        "stepover_mm",
    ] {
        assert!(value(&blank)["tools"][0][field].is_null());
    }
    assert_eq!(blank.tools[0].plunge_capable, None); // Do not invent a capability from geometry.
    library.tools[1].cutting_presets[0].spindle_rpm = None;
    let partial = library
        .apply_to_job(&original, ToolSlot::Vbit, "saved-1", Some("test"))
        .unwrap();
    assert_eq!(partial.tools[1].spindle_rpm, None);
    assert_eq!(partial.tools[1].cutting_feed_mm_min, Some(250.));
    assert_eq!(partial.tools[1].plunge_capable, Some(true));
}

#[test]
fn apply_rejects_wrong_kind_missing_preset_and_insufficient_cutting_length_without_mutation() {
    let mut library = library();
    let original = job();
    let before = original.to_json().unwrap();
    assert_eq!(
        library
            .apply_to_job(&original, ToolSlot::Vbit, "saved-0", None)
            .unwrap_err()
            .code,
        "LIBRARY_TOOL_KIND"
    );
    assert_eq!(
        library
            .apply_to_job(&original, ToolSlot::Endmill, "saved-0", Some("absent"))
            .unwrap_err()
            .code,
        "LIBRARY_NOT_FOUND"
    );
    if let cam_core::tool_library::LibraryGeometry::Endmill(spec) = &mut library.tools[0].geometry {
        spec.cutting_length_mm = 1.;
    }
    library.validate().unwrap();
    assert_eq!(
        library
            .apply_to_job(&original, ToolSlot::Endmill, "saved-0", None)
            .unwrap_err()
            .code,
        "ENDMILL_CUTTING_LENGTH"
    );
    assert_eq!(original.to_json().unwrap(), before);
}

#[test]
fn capture_requires_geometry_and_does_not_copy_cutting_values_implicitly() {
    let mut settings = job().tools.remove(0);
    assert!(
        LibraryTool::from_settings("new".into(), "Cutter".into(), &settings)
            .unwrap()
            .cutting_presets
            .is_empty()
    );
    settings.geometry = None;
    assert_eq!(
        LibraryTool::from_settings("new".into(), "Cutter".into(), &settings)
            .unwrap_err()
            .code,
        "LIBRARY_GEOMETRY"
    );
}

#[test]
fn resource_and_revision_limits_are_checked_on_reads_and_changes() {
    assert_eq!(
        ToolLibrary::from_json(&" ".repeat(MAX_LIBRARY_BYTES + 1))
            .unwrap_err()
            .code,
        "LIBRARY_RESOURCE_LIMIT"
    );
    let at_limit = ToolLibrary {
        revision: MAX_LIBRARY_REVISION,
        ..ToolLibrary::default()
    };
    assert_eq!(
        at_limit
            .changed(
                MAX_LIBRARY_REVISION,
                LibraryChange::AddTool { tool: tool(0) }
            )
            .unwrap_err()
            .code,
        "LIBRARY_RESOURCE_LIMIT"
    );
    let mut too_many = library();
    too_many.tools = vec![tool(0); MAX_TOOLS + 1];
    assert_eq!(
        too_many.validate().unwrap_err().code,
        "LIBRARY_RESOURCE_LIMIT"
    );
    too_many.tools = vec![tool(0)];
    too_many.tools[0].cutting_presets =
        vec![tool(0).cutting_presets.remove(0); MAX_PRESETS_PER_TOOL + 1];
    assert_eq!(
        too_many.validate().unwrap_err().code,
        "LIBRARY_RESOURCE_LIMIT"
    );
}

#[test]
fn applying_a_changed_preset_invalidates_existing_plan_identity() {
    let original = Job::from_json(include_str!("../../../fixtures/m3/no-access.json")).unwrap();
    let mut plan = plan_endmill(&original).unwrap();
    let mut tool =
        LibraryTool::from_settings("new".into(), "Synthetic".into(), &original.tools[0]).unwrap();
    let mut preset =
        CuttingPreset::from_settings("new".into(), "Synthetic".into(), &original.tools[0]).unwrap();
    preset.cutting_feed_mm_min = Some(321.);
    tool.cutting_presets.push(preset);
    let library = ToolLibrary {
        tools: vec![tool],
        ..ToolLibrary::default()
    };
    plan.job = library
        .apply_to_job(&original, ToolSlot::Endmill, "new", Some("new"))
        .unwrap();
    assert_eq!(
        EndmillPlan::from_json(&plan.to_json().unwrap())
            .unwrap_err()
            .code,
        "STALE_PLAN"
    );
}

// --- F1: knife tools and typed knife presets (plan section 5.3) ---

use cam_core::project::{
    CamJob, DragKnifeSettings, DragKnifeSpec, HeightRef, JobTool as JobToolSnapshot,
    KnifeAlignment, KnifeAssignment, Operation, OperationSettings, ToolGeometry as JobToolGeometry,
};
use cam_core::tool_library::{KnifeCuttingPreset, LibraryGeometry};

fn knife_tool() -> LibraryTool {
    LibraryTool {
        id: "blade".into(),
        name: "Drag knife".into(),
        geometry: LibraryGeometry::DragKnife(DragKnifeSpec {
            blade_offset_mm: 1.2,
            max_cut_depth_mm: 3.,
        }),
        ramp_capable: None,
        plunge_capable: None,
        cutting_presets: vec![],
        knife_cutting_presets: vec![KnifeCuttingPreset {
            id: "cardboard".into(),
            name: "Cardboard".into(),
            material: Some("cardboard".into()),
            machine: None,
            cutting_feed_mm_min: Some(150.),
            plunge_feed_mm_min: Some(60.),
            swivel_feed_mm_min: Some(50.),
            max_stepdown_mm: Some(1.),
        }],
    }
}

fn knife_job() -> CamJob {
    let assignment = |tool: &str| KnifeAssignment {
        tool_id: tool.into(),
        cutting_feed_mm_min: None,
        plunge_feed_mm_min: None,
        swivel_feed_mm_min: None,
        max_stepdown_mm: None,
    };
    let settings = |tool: &str| {
        OperationSettings::DragKnife(DragKnifeSettings {
            chains: vec![],
            assignment: assignment(tool),
            top: HeightRef {
                reference: Default::default(),
                offset_mm: 0.,
            },
            bottom: HeightRef {
                reference: cam_core::project::HeightReference::OperationTop,
                offset_mm: -1.,
            },
            stepdown_mm: None,
            swivel_depth_mm: None,
            corner_threshold_deg: None,
            through_cut_allowance_mm: None,
            start: Default::default(),
            closure_overlap_mm: None,
            alignment: KnifeAlignment::default(),
        })
    };
    let job = CamJob {
        schema_version: 4,
        name: "knife-library".into(),
        source: None,
        import: Default::default(),
        setup: Default::default(),
        tools: vec![JobToolSnapshot {
            id: "blade".into(),
            name: "old geometry".into(),
            geometry: Some(JobToolGeometry::DragKnife(DragKnifeSpec {
                blade_offset_mm: 0.8,
                max_cut_depth_mm: 2.,
            })),
            capabilities: Default::default(),
        }],
        operations: vec![
            Operation {
                id: "knife-a".into(),
                name: "knife-a".into(),
                enabled: true,
                settings: settings("blade"),
            },
            Operation {
                id: "knife-b".into(),
                name: "knife-b".into(),
                enabled: true,
                settings: settings("blade"),
            },
        ],
        tolerances: Default::default(),
        legacy_machine_profile: None,
    };
    job.validate().unwrap();
    job
}

#[test]
fn knife_tools_and_presets_validate_and_round_trip_strictly() {
    let library = ToolLibrary {
        tools: vec![knife_tool()],
        ..ToolLibrary::default()
    };
    let saved = library.to_json().unwrap();
    assert_eq!(
        ToolLibrary::from_json(&saved).unwrap().to_json().unwrap(),
        saved
    );
    // Knife presets carry no spindle speed or stepover: strict parsing
    // rejects them rather than guessing a meaning.
    for invalid in [
        saved.replacen(
            "\"swivel_feed_mm_min\"",
            "\"spindle_rpm\": 1000.0, \"swivel_feed_mm_min\"",
            1,
        ),
        saved.replacen(
            "\"swivel_feed_mm_min\"",
            "\"stepover_mm\": 1.0, \"swivel_feed_mm_min\"",
            1,
        ),
    ] {
        assert!(
            ToolLibrary::from_json(&invalid).is_err(),
            "accepted: {invalid}"
        );
    }
    // A milling preset on a knife tool, a knife preset on a milling tool and
    // milling entry capabilities on a knife are all kind errors.
    let mut wrong_preset = knife_tool();
    wrong_preset
        .cutting_presets
        .push(CuttingPreset::from_settings("m".into(), "Milling".into(), &job().tools[0]).unwrap());
    assert_eq!(
        wrong_preset.validate().unwrap_err().code,
        "LIBRARY_TOOL_KIND"
    );
    let mut wrong_tool = tool(0);
    wrong_tool.knife_cutting_presets = knife_tool().knife_cutting_presets;
    assert_eq!(wrong_tool.validate().unwrap_err().code, "LIBRARY_TOOL_KIND");
    let mut capabilities = knife_tool();
    capabilities.plunge_capable = Some(true);
    assert_eq!(
        capabilities.validate().unwrap_err().code,
        "LIBRARY_TOOL_KIND"
    );
    // Preset values stay finite and nonnegative.
    let mut negative = knife_tool();
    negative.knife_cutting_presets[0].swivel_feed_mm_min = Some(-1.);
    assert_eq!(negative.validate().unwrap_err().code, "JOB_PARAMETER");
}

#[test]
fn knife_preset_crud_is_an_immutable_transaction() {
    // The library holds a milling tool beside the knife so kind mismatches
    // are distinguishable from unknown IDs.
    let library = ToolLibrary {
        tools: vec![tool(0), knife_tool()],
        ..ToolLibrary::default()
    };
    let mut preset = KnifeCuttingPreset {
        id: "felt".into(),
        name: "Felt".into(),
        material: None,
        machine: None,
        cutting_feed_mm_min: Some(200.),
        plunge_feed_mm_min: Some(80.),
        swivel_feed_mm_min: Some(60.),
        max_stepdown_mm: None,
    };
    let added = library
        .changed(
            0,
            LibraryChange::AddKnifePreset {
                tool_id: "blade".into(),
                preset: preset.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        library.tools[1].knife_cutting_presets.len(),
        1,
        "original untouched"
    );
    assert_eq!(added.tools[1].knife_cutting_presets.len(), 2);
    preset.cutting_feed_mm_min = Some(220.);
    let replaced = added
        .changed(
            1,
            LibraryChange::ReplaceKnifePreset {
                tool_id: "blade".into(),
                preset,
            },
        )
        .unwrap();
    assert_eq!(
        replaced.tools[1].knife_cutting_presets[1].cutting_feed_mm_min,
        Some(220.)
    );
    let duplicated = replaced
        .changed(
            2,
            LibraryChange::DuplicateKnifePreset {
                tool_id: "blade".into(),
                preset_id: "felt".into(),
                new_id: "felt-copy".into(),
                name: "Felt copy".into(),
            },
        )
        .unwrap();
    assert_eq!(duplicated.tools[1].knife_cutting_presets.len(), 3);
    let removed = duplicated
        .changed(
            3,
            LibraryChange::RemoveKnifePreset {
                tool_id: "blade".into(),
                preset_id: "felt-copy".into(),
            },
        )
        .unwrap();
    assert_eq!(removed.tools[1].knife_cutting_presets.len(), 2);
    // A knife preset edit on a milling tool is rejected; so is an unknown ID.
    assert_eq!(
        removed
            .changed(
                4,
                LibraryChange::AddKnifePreset {
                    tool_id: "saved-0".into(),
                    preset: KnifeCuttingPreset {
                        id: "x".into(),
                        name: "x".into(),
                        material: None,
                        machine: None,
                        cutting_feed_mm_min: None,
                        plunge_feed_mm_min: None,
                        swivel_feed_mm_min: None,
                        max_stepdown_mm: None,
                    },
                },
            )
            .unwrap_err()
            .code,
        "LIBRARY_TOOL_KIND"
    );
    assert_eq!(
        removed
            .changed(
                4,
                LibraryChange::RemoveKnifePreset {
                    tool_id: "blade".into(),
                    preset_id: "absent".into()
                }
            )
            .unwrap_err()
            .code,
        "LIBRARY_NOT_FOUND"
    );
}

#[test]
fn knife_presets_apply_to_exactly_one_canonical_operation() {
    let library = ToolLibrary {
        tools: vec![tool(0), knife_tool()],
        ..ToolLibrary::default()
    };
    let original = knife_job();
    let applied = library
        .apply_knife_to_job(&original, "knife-a", "blade", Some("cardboard"))
        .unwrap();
    // The job tool snapshot gains the library geometry under the tool ID.
    let snapshot = applied.tools.iter().find(|t| t.id == "blade").unwrap();
    assert_eq!(
        snapshot.geometry,
        Some(JobToolGeometry::DragKnife(DragKnifeSpec {
            blade_offset_mm: 1.2,
            max_cut_depth_mm: 3.,
        }))
    );
    // Exactly the selected operation's assignment receives the preset; the
    // sibling operation using the same tool keeps its values (plan 16.3).
    let assignment = |job: &CamJob, id: &str| match &job
        .operations
        .iter()
        .find(|o| o.id == id)
        .unwrap()
        .settings
    {
        OperationSettings::DragKnife(s) => s.assignment.clone(),
        _ => unreachable!(),
    };
    let a = assignment(&applied, "knife-a");
    assert_eq!(a.tool_id, "blade");
    assert_eq!(a.cutting_feed_mm_min, Some(150.));
    assert_eq!(a.swivel_feed_mm_min, Some(50.));
    assert_eq!(a.max_stepdown_mm, Some(1.));
    let b = assignment(&applied, "knife-b");
    assert_eq!(
        b.cutting_feed_mm_min, None,
        "the sibling operation is untouched"
    );
    // The source job is unchanged.
    assert_eq!(assignment(&original, "knife-a").cutting_feed_mm_min, None);
    // A milling tool or a non-knife operation is a kind error, and without a
    // preset only the tool binding is applied.
    assert_eq!(
        library
            .apply_knife_to_job(&original, "knife-a", "saved-0", None)
            .unwrap_err()
            .code,
        "LIBRARY_TOOL_KIND"
    );
    let bound_only = library
        .apply_knife_to_job(&original, "knife-b", "blade", None)
        .unwrap();
    assert_eq!(assignment(&bound_only, "knife-b").cutting_feed_mm_min, None);
    assert_eq!(assignment(&bound_only, "knife-b").tool_id, "blade");
    // Knife tools never enter a legacy job slot.
    assert_eq!(
        library
            .apply_to_job(
                &Job::from_json(include_str!("../../../fixtures/m3/no-access.json")).unwrap(),
                ToolSlot::Endmill,
                "blade",
                None
            )
            .unwrap_err()
            .code,
        "LIBRARY_TOOL_KIND"
    );
}
