use cam_core::project::{
    FlatVcarveMode,
    v5::{self, CamJobV5},
};
use cam_gui_runtime::{
    app::Document,
    authoring, compute,
    session::{self, ArtworkCommand, Command},
};
use cam_service::retained::Retained;

const JOB: &str = include_str!("../../../fixtures/gui4/lettering.job.json");
const SVG: &str = include_str!("../../../fixtures/gui4/second.svg");
const REPLACEMENT: &str = include_str!("../../../fixtures/gui4/replacement.svg");
#[test]
fn batch_import_duplicate_and_reorder_preserve_assignments() {
    let job = session::open(JOB).unwrap();
    let (added, report) = edit(
        &job,
        ArtworkCommand::AddMany {
            files: vec![
                cam_gui_runtime::platform::SvgFile {
                    filename: "second.svg".into(),
                    content: Ok(SVG.into()),
                },
                cam_gui_runtime::platform::SvgFile {
                    filename: "broken.svg".into(),
                    content: Ok("<svg>broken".into()),
                },
                cam_gui_runtime::platform::SvgFile {
                    filename: "unreadable.svg".into(),
                    content: Err("Access denied".into()),
                },
            ],
        },
    );
    assert_eq!(added.artwork.len(), 2);
    assert_eq!(report["rejectedFiles"].as_array().unwrap().len(), 2);
    assert_eq!(
        session::settings(&added).components,
        session::settings(&job).components
    );
    let (duplicated, _) = edit(
        &added,
        ArtworkCommand::Duplicate {
            item: added.artwork[1].id.clone(),
        },
    );
    assert_eq!(duplicated.artwork.len(), 3);
    assert_ne!(duplicated.artwork[1].id, duplicated.artwork[2].id);
    assert_eq!(duplicated.artwork[1].content, duplicated.artwork[2].content);
    assert_eq!(
        session::settings(&duplicated).components,
        session::settings(&job).components
    );
    let ids = duplicated
        .artwork
        .iter()
        .rev()
        .map(|i| i.id.clone())
        .collect();
    let (reordered, _) = edit(&duplicated, ArtworkCommand::Reorder { items: ids });
    assert_eq!(
        session::settings(&reordered).components,
        session::settings(&job).components
    );
    assert_eq!(reordered.artwork[2], job.artwork[0]);
}
#[test]
fn explicit_migration_and_applied_profile_produce_one_portable_project() {
    let legacy = include_str!("../../../../real_data/flower_box-svg.job-real.json");
    let expected = v5::migrate::migrate_json(legacy).unwrap();
    let (migrated, _) = session::execute(
        &mut Retained::new(),
        Command::Migrate {
            json: legacy.into(),
        },
    )
    .unwrap();
    assert_eq!(session::open(&migrated.job).unwrap(), expected);
    let (applied, _) = session::execute(
        &mut Retained::new(),
        Command::ApplyProfile {
            job: migrated.job,
            json: session::PROFILE.into(),
        },
    )
    .unwrap();
    let portable = session::open(&applied.job).unwrap();
    assert_eq!(portable.artwork, expected.artwork);
    assert_eq!(
        session::settings(&portable).components,
        session::settings(&expected).components
    );
    assert!(portable.machine_configuration.is_some());
    assert_eq!(
        session::open(&portable.to_json().unwrap()).unwrap(),
        portable
    );
    let mut newer = serde_json::to_value(&portable).unwrap();
    newer["schema_version"] = serde_json::json!(6);
    assert!(
        session::execute(
            &mut Retained::new(),
            Command::Migrate {
                json: newer.to_string()
            }
        )
        .is_err()
    );
}
fn edit(job: &CamJobV5, action: ArtworkCommand) -> (CamJobV5, serde_json::Value) {
    let (scene, _) = session::execute(
        &mut Retained::new(),
        Command::Artwork {
            job: job.to_json().unwrap(),
            action,
        },
    )
    .unwrap();
    (
        session::open(&scene.job).unwrap(),
        scene.report["gui2"].clone(),
    )
}
fn added() -> CamJobV5 {
    edit(
        &session::open(JOB).unwrap(),
        ArtworkCommand::Add {
            filename: "second.svg".into(),
            svg: SVG.into(),
        },
    )
    .0
}
fn components(job: &CamJobV5) -> Vec<authoring::Component> {
    authoring::catalogue_components(&v5::artwork::inspect_artwork(job).unwrap())
}
#[test]
fn collection_placement_drafts_recovery_are_bound_to_ids() {
    let mut doc = Document::new(added());
    let first = doc.job.artwork[0].id.0.clone();
    let second = doc.job.artwork[1].id.0.clone();
    let before = doc.job.artwork[0].clone();
    assert!(doc.select_artwork(&second));
    doc.edit(26, "-25".into()).unwrap();
    assert_eq!(doc.job.artwork[0], before);
    assert!(doc.edit(27, "-".into()).is_err());
    assert!(doc.select_artwork(&first));
    assert_eq!(doc.text(26), "-5");
    assert!(
        doc.pending(),
        "inactive artwork still has a partial placement draft"
    );
    doc.edit(27, "-4".into()).unwrap();
    doc.job.artwork.reverse();
    assert_eq!(doc.text(27), "-4");
    assert!(doc.select_artwork(&second));
    assert_eq!(doc.text(27), "-");
    assert_eq!(doc.text(26), "-25");
    doc.snapshot().validate().unwrap();
    let restored: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
    restored.validate().unwrap();
    assert_eq!(restored, doc);
    assert!(!doc.select_artwork("missing"));
    doc.edit(27, "-3".into()).unwrap();
    assert!(!doc.pending());
}
#[test]
fn replaced_and_deleted_sources_stay_unresolved_until_explicit_repair() {
    let mut job = added();
    let refs: Vec<_> = components(&job)
        .into_iter()
        .filter(|c| c.reference.local_geometry_id == "letter-l::0")
        .map(|c| c.reference)
        .collect();
    authoring::settings_mut(&mut job).components = refs.clone();
    let first = job.artwork[0].id.clone();
    let (replaced, report) = edit(
        &job,
        ArtworkCommand::Replace {
            item: first.clone(),
            filename: "same-name.svg".into(),
            svg: REPLACEMENT.into(),
        },
    );
    assert!(!report["issues"].as_array().unwrap().is_empty());
    assert_eq!(session::settings(&replaced).components, refs);
    let reopened = session::open(&replaced.to_json().unwrap()).unwrap();
    let (blocked, _) = session::execute(
        &mut Retained::new(),
        Command::Generate {
            job: reopened.to_json().unwrap(),
        },
    )
    .unwrap();
    assert_eq!(blocked.report["gui2"]["kind"], "issues");
    let new_ref = components(&reopened)
        .into_iter()
        .find(|c| {
            c.reference.artwork_item_id == first && c.reference.local_geometry_id == "letter-l::0"
        })
        .unwrap()
        .reference;
    assert_ne!(refs[0].source_revision, new_ref.source_revision);
    let (repaired, report) = edit(
        &reopened,
        ArtworkCommand::Repair {
            expected: refs[0].clone(),
            replacement: new_ref.clone(),
        },
    );
    assert!(report["issues"].as_array().unwrap().is_empty());
    assert_eq!(
        session::settings(&repaired).components,
        vec![new_ref, refs[1].clone()]
    );
    assert!(
        session::execute(
            &mut Retained::new(),
            Command::Artwork {
                job: repaired.to_json().unwrap(),
                action: ArtworkCommand::Repair {
                    expected: refs[0].clone(),
                    replacement: refs[1].clone()
                }
            }
        )
        .is_err()
    );
    let (deleted, report) = edit(&repaired, ArtworkCommand::Delete { item: first });
    assert_eq!(deleted.artwork.len(), 1);
    assert!(!report["issues"].as_array().unwrap().is_empty());
    assert_eq!(session::open(&deleted.to_json().unwrap()).unwrap(), deleted);
    let last = deleted.artwork[0].id.clone();
    let (empty, _) = edit(&deleted, ArtworkCommand::Delete { item: last });
    let mut doc = Document::new(empty);
    doc.sync_artwork();
    doc.snapshot().validate().unwrap();
    assert!(doc.active_artwork().is_none());
    let (restored, _) = edit(
        &doc.job,
        ArtworkCommand::Add {
            filename: "second.svg".into(),
            svg: SVG.into(),
        },
    );
    assert_eq!(
        session::settings(&restored).components.len(),
        2,
        "adding does not infer repairs or delete dangling refs"
    );
    assert!(
        !session::settings(&restored)
            .components
            .iter()
            .any(|r| r.artwork_item_id == restored.artwork[0].id),
        "adding a same-named file must not reuse a dangling owner ID"
    );
    let before = job.to_json().unwrap();
    assert!(
        session::execute(
            &mut Retained::new(),
            Command::Artwork {
                job: before.clone(),
                action: ArtworkCommand::Replace {
                    item: job.artwork[0].id.clone(),
                    filename: "bad.svg".into(),
                    svg: "<svg>broken".into()
                }
            }
        )
        .is_err()
    );
    assert_eq!(job.to_json().unwrap(), before);
}
#[test]
fn cross_source_retained_simulation_checked_output_and_portable_reopen() {
    let mut job = added();
    job.artwork[1].placement.origin_mm = cam_core::geometry::Point::new(-25., -3.);
    let assigned: Vec<_> = components(&job)
        .into_iter()
        .filter(|c| c.reference.local_geometry_id == "letter-l::0")
        .map(|c| c.reference)
        .collect();
    authoring::settings_mut(&mut job).components = assigned;
    authoring::set_mode(&mut job, FlatVcarveMode::Combined);
    let portable = job.to_json().unwrap();
    assert_eq!(session::open(&portable).unwrap(), job);
    let mut worker = Retained::new();
    let (scene, payload) = session::execute(
        &mut worker,
        Command::Generate {
            job: portable.clone(),
        },
    )
    .unwrap();
    assert_eq!(scene.report["gui2"]["checks"]["exportReady"], true);
    assert_eq!(
        scene.report["gui2"]["components"].as_array().unwrap().len(),
        4
    );
    assert!(scene.motions > 0 && scene.motions > scene.rough_vertices / 2);
    let handle = scene.report["gui2"]["handle"].as_str().unwrap().to_string();
    let input = compute::Scene {
        meta: scene.clone(),
        payload: std::sync::Arc::new(payload),
    }
    .sim_input()
    .unwrap()
    .unwrap();
    assert!(input.motions.iter().any(|m| m.x1 > 27.));
    for prefix in [scene.rough_vertices / 2, scene.motions, 0, scene.motions] {
        let (seek, _) = session::execute(
            &mut worker,
            Command::Seek {
                handle: handle.clone(),
                prefix,
            },
        )
        .unwrap();
        assert_eq!(seek.report["gui2"]["prefix"], prefix);
    }
    let (output, _) = session::execute(
        &mut worker,
        Command::Prepare {
            job: portable,
            handle,
        },
    )
    .unwrap();
    let file = &output.report["gui2"]["file"];
    assert_eq!(
        compute::hash(file["gcode"].as_str().unwrap().as_bytes()),
        file["sha256"]
    );
    assert_eq!(output.report["gui2"]["retained"]["plansRun"], 1);
}

#[test]
fn retained_plan_reuse_uses_core_machining_identity_not_document_revision() {
    let job = session::open(JOB).unwrap();
    let mut worker = Retained::new();
    let (generated, _) = session::execute(
        &mut worker,
        Command::Generate {
            job: job.to_json().unwrap(),
        },
    )
    .unwrap();
    let handle = generated.report["gui2"]["handle"]
        .as_str()
        .unwrap()
        .to_string();
    let (added, _) = session::execute(
        &mut worker,
        Command::Artwork {
            job: job.to_json().unwrap(),
            action: ArtworkCommand::Add {
                filename: "unused.svg".into(),
                svg: SVG.into(),
            },
        },
    )
    .unwrap();
    let mut candidate = session::open(&added.job).unwrap();
    candidate.artwork[1].placement.origin_mm.x = -200.;
    candidate.name = "Renamed portable project".into();
    candidate.artwork.reverse();
    let (reused, _) = session::execute(
        &mut worker,
        Command::ValidatePlan {
            job: candidate.to_json().unwrap(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(reused.report["gui2"]["kind"], "revalidated");
    assert_eq!(reused.motions, generated.motions);
    assert_eq!(
        reused.report["gui2"]["executionFingerprint"],
        generated.report["gui2"]["executionFingerprint"]
    );
    let (prepared, _) = session::execute(
        &mut worker,
        Command::Prepare {
            job: candidate.to_json().unwrap(),
            handle: handle.clone(),
        },
    )
    .unwrap();
    assert_eq!(prepared.report["gui2"]["retained"]["plansRun"], 1);
    candidate.artwork[1].placement.origin_mm.x -= 1.;
    let (stale, _) = session::execute(
        &mut worker,
        Command::ValidatePlan {
            job: candidate.to_json().unwrap(),
            handle,
        },
    )
    .unwrap();
    assert_eq!(stale.report["gui2"]["kind"], "stale");
}
