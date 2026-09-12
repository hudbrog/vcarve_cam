//! Profile-operation editor adapter (GUI8a–GUI8d).
//!
//! The planner owns every machining decision. This module only binds the
//! editor's typed fields to the canonical [`ProfileSettingsV5`] values and
//! back, lists the closed contours of the displayed catalogue, and turns an
//! explicit row/gesture choice into the shared document commands. Empty text
//! stays unset, so a half-configured profile never looks ready.
use cam_core::project::{
    ContourSide, CutDirection, HeightReference, LeadSpec, ProfileEntry, TabShape,
    TraversalDirection,
    v5::{
        self, CamJobV5, ContourAnchorV5, OperationSettingsV5, ProfileContourV5, ProfileSettingsV5,
        StartSelectionV5, TabPlacementV5, TabSettingsV5, artwork,
    },
};
use serde::{Deserialize, Serialize};

/// The profile settings of one explicit operation.
pub fn settings_in<'a>(job: &'a CamJobV5, operation_id: &str) -> Option<&'a ProfileSettingsV5> {
    match &crate::session::operation(job, operation_id)?.settings {
        OperationSettingsV5::Profile(settings) => Some(settings),
        _ => None,
    }
}

pub fn settings_in_mut<'a>(
    job: &'a mut CamJobV5,
    operation_id: &str,
) -> Option<&'a mut ProfileSettingsV5> {
    match &mut crate::session::operation_mut(job, operation_id)?.settings {
        OperationSettingsV5::Profile(settings) => Some(settings),
        _ => None,
    }
}

/// The first profile operation. Ordered sequences address an explicit ID.
pub fn settings(job: &CamJobV5) -> Option<&ProfileSettingsV5> {
    job.operations
        .iter()
        .find_map(|operation| match &operation.settings {
            OperationSettingsV5::Profile(settings) => Some(settings),
            _ => None,
        })
}

/// Fields this editor binds for a Profile operation. IDs 2/8/10/11/12/13/70/88
/// are the operation's own assignment and pass values, exactly as they are for
/// a Face or Flat V-carve operation; 89…108 are the profile-only values.
pub fn active(field: usize) -> bool {
    matches!(
        field,
        2 | 6
            | 7
            | 8
            | 10
            | 11
            | 12
            | 13
            | 23
            | 25
            | 26
            | 27
            | 28
            | 29
            | 30
            | 31
            | 40..=45
            | 70
            | 88..=108
    )
}

/// Multi-field commit groups. Profile pass/height values are independent, so
/// only the placement and stock groups it reuses remain grouped.
pub fn group(_field: usize) -> &'static [usize] {
    &[]
}

pub fn value(job: &CamJobV5, operation_id: &str, field: usize) -> Option<f64> {
    let s = settings_in(job, operation_id);
    match field {
        2 => s?.assignment.cutting_feed_mm_min,
        8 => s?.stepdown_mm,
        10 => s?.assignment.plunge_feed_mm_min,
        11 => s?.assignment.spindle_rpm,
        70 => s?.through_cut_allowance_mm,
        88 => s?.assignment.max_stepdown_mm,
        89 => Some(s?.top.offset_mm),
        90 => Some(s?.bottom.offset_mm),
        91 => s?.finish.radial_allowance_mm,
        92 => s?.finish.feed_mm_min,
        93 => s?.tabs.as_ref()?.height_mm,
        94 => s?.tabs.as_ref()?.width_mm,
        95 => match &s?.tabs.as_ref()?.placement {
            TabPlacementV5::Automatic { count, .. } => count.map(f64::from),
            TabPlacementV5::Manual { .. } => None,
        },
        96 => match &s?.tabs.as_ref()?.placement {
            TabPlacementV5::Automatic { spacing_mm, .. } => *spacing_mm,
            TabPlacementV5::Manual { .. } => None,
        },
        98 => match &s?.entry {
            ProfileEntry::Ramp { max_angle_deg, .. } => *max_angle_deg,
            ProfileEntry::Plunge => None,
        },
        99 => match &s?.entry {
            ProfileEntry::Ramp { feed_mm_min, .. } => *feed_mm_min,
            ProfileEntry::Plunge => None,
        },
        100 | 101 => match &s?.lead_in {
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                if field == 100 {
                    *length_mm
                } else {
                    *feed_mm_min
                }
            }
            _ => None,
        },
        102 | 103 => match &s?.lead_in {
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                ..
            } => {
                if field == 102 {
                    *radius_mm
                } else {
                    *sweep_deg
                }
            }
            _ => None,
        },
        104 | 105 => match &s?.lead_out {
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                if field == 104 {
                    *length_mm
                } else {
                    *feed_mm_min
                }
            }
            _ => None,
        },
        106 | 107 => match &s?.lead_out {
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                ..
            } => {
                if field == 106 {
                    *radius_mm
                } else {
                    *sweep_deg
                }
            }
            _ => None,
        },
        // The anchor fractions are edited by their own row, never by the
        // job-scoped binder; the anchor's own scope supplies the value.
        97 | 108 => None,
        _ => crate::authoring::value_in(job, operation_id, field),
    }
}

pub fn set(
    job: &mut CamJobV5,
    operation_id: &str,
    field: usize,
    v: Option<f64>,
) -> Result<(), String> {
    if field == 6 {
        job.setup.stock.thickness_mm = v;
        return Ok(());
    }
    if matches!(field, 97 | 108) {
        return Err("Anchor positions are edited in their own tab or start row".into());
    }
    if !matches!(field, 2 | 8 | 10 | 11 | 70 | 88..=99 | 100..=107) {
        return crate::authoring::set_in(job, operation_id, field, v);
    }
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    match field {
        2 => s.assignment.cutting_feed_mm_min = v,
        8 => s.stepdown_mm = v,
        10 => s.assignment.plunge_feed_mm_min = v,
        11 => s.assignment.spindle_rpm = v,
        70 => s.through_cut_allowance_mm = v,
        88 => s.assignment.max_stepdown_mm = v,
        89 => s.top.offset_mm = v.ok_or("Profile top offset cannot be unset")?,
        90 => s.bottom.offset_mm = v.ok_or("Profile bottom offset cannot be unset")?,
        91 => s.finish.radial_allowance_mm = v,
        92 => s.finish.feed_mm_min = v,
        93 => tab_settings(s)?.height_mm = v,
        94 => tab_settings(s)?.width_mm = v,
        95 => {
            let TabPlacementV5::Automatic { count, .. } = &mut tab_settings(s)?.placement else {
                return Err("Choose automatic tab placement before setting a count".into());
            };
            *count = match v {
                None => None,
                Some(n) if n.fract() == 0. && (1. ..=256.).contains(&n) => Some(n as u32),
                _ => return Err("Tab count must be a whole number from 1 to 256".into()),
            };
        }
        96 => {
            let TabPlacementV5::Automatic { spacing_mm, .. } = &mut tab_settings(s)?.placement
            else {
                return Err("Choose automatic tab placement before setting a spacing".into());
            };
            *spacing_mm = v;
        }
        98 => match &mut s.entry {
            ProfileEntry::Ramp { max_angle_deg, .. } => *max_angle_deg = v,
            ProfileEntry::Plunge => {
                return Err("Choose a ramp entry before setting its angle".into());
            }
        },
        99 => match &mut s.entry {
            ProfileEntry::Ramp { feed_mm_min, .. } => *feed_mm_min = v,
            ProfileEntry::Plunge => {
                return Err("Choose a ramp entry before setting its feed".into());
            }
        },
        100 | 101 => match &mut s.lead_in {
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                if field == 100 {
                    *length_mm = v
                } else {
                    *feed_mm_min = v
                }
            }
            _ => return Err("Choose a tangent line lead-in before setting its values".into()),
        },
        102 | 103 => match &mut s.lead_in {
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                ..
            } => {
                if field == 102 {
                    *radius_mm = v
                } else {
                    *sweep_deg = v
                }
            }
            _ => return Err("Choose a tangent arc lead-in before setting its values".into()),
        },
        104 | 105 => match &mut s.lead_out {
            LeadSpec::TangentLine {
                length_mm,
                feed_mm_min,
            } => {
                if field == 104 {
                    *length_mm = v
                } else {
                    *feed_mm_min = v
                }
            }
            _ => return Err("Choose a tangent line lead-out before setting its values".into()),
        },
        _ => match &mut s.lead_out {
            LeadSpec::TangentArc {
                radius_mm,
                sweep_deg,
                ..
            } => {
                if field == 106 {
                    *radius_mm = v
                } else {
                    *sweep_deg = v
                }
            }
            _ => return Err("Choose a tangent arc lead-out before setting its values".into()),
        },
    }
    Ok(())
}

fn tab_settings(s: &mut ProfileSettingsV5) -> Result<&mut TabSettingsV5, String> {
    s.tabs
        .as_mut()
        .ok_or_else(|| "Turn tabs on before setting their values".into())
}

pub fn set_spindle_direction(
    job: &mut CamJobV5,
    operation_id: &str,
    direction: Option<cam_core::project::SpindleDirection>,
) -> Result<(), String> {
    settings_in_mut(job, operation_id)
        .ok_or("Expected a profile operation")?
        .assignment
        .spindle_direction = direction;
    Ok(())
}

/// Climb or conventional cutting. Required whenever a selection retains a side.
pub fn set_direction(
    job: &mut CamJobV5,
    operation_id: &str,
    direction: Option<CutDirection>,
) -> Result<(), String> {
    settings_in_mut(job, operation_id)
        .ok_or("Expected a profile operation")?
        .direction = direction;
    Ok(())
}

pub fn set_order(
    job: &mut CamJobV5,
    operation_id: &str,
    order: cam_core::project::ContourOrder,
) -> Result<(), String> {
    settings_in_mut(job, operation_id)
        .ok_or("Expected a profile operation")?
        .order = order;
    Ok(())
}

/// Height reference selection for the top or bottom of a profile operation.
pub fn set_height_reference(
    job: &mut CamJobV5,
    operation_id: &str,
    bottom: bool,
    reference: HeightReference,
) -> Result<(), String> {
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    if bottom {
        s.bottom.reference = reference;
    } else {
        s.top.reference = reference;
    }
    job.validate_structure().map_err(|e| e.to_string())?;
    Ok(())
}

/// Every face plane an earlier operation published, in document order, for the
/// height-reference choices of a later operation.
pub fn published_faces(job: &CamJobV5, before: &str) -> Vec<String> {
    crate::face::published_faces(job, before)
}

/// Radial finishing (GUI8c). Turning it on leaves the allowance and feed unset
/// so the planner reports exactly what is still missing.
pub fn set_finish_enabled(
    job: &mut CamJobV5,
    operation_id: &str,
    enabled: bool,
) -> Result<(), String> {
    settings_in_mut(job, operation_id)
        .ok_or("Expected a profile operation")?
        .finish
        .enabled = enabled;
    Ok(())
}

/// Entry mode: a plunge straight down, or a ramp that descends along the loop.
pub fn set_entry(job: &mut CamJobV5, operation_id: &str, ramp: bool) -> Result<(), String> {
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    s.entry = if ramp {
        match &s.entry {
            ProfileEntry::Ramp { .. } => s.entry.clone(),
            ProfileEntry::Plunge => ProfileEntry::Ramp {
                max_angle_deg: None,
                feed_mm_min: None,
            },
        }
    } else {
        ProfileEntry::Plunge
    };
    Ok(())
}

/// Lead-in/lead-out shape. `None` clears the lead; a chosen shape keeps its
/// values unset until edited (the mode buttons pass a shape without values,
/// while a caller that has resolved values supplies them).
pub fn set_lead(
    job: &mut CamJobV5,
    operation_id: &str,
    out: bool,
    spec: LeadSpec,
) -> Result<(), String> {
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    let slot = if out { &mut s.lead_out } else { &mut s.lead_in };
    *slot = spec;
    Ok(())
}

/// Turn tabs on or off. Turning them on starts from four automatically placed
/// rectangular tabs with their height and width unset: a visible, editable
/// starting point rather than a hidden machining assumption.
pub fn set_tabs_enabled(job: &mut CamJobV5, operation_id: &str, on: bool) -> Result<(), String> {
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    if on {
        if s.tabs.is_none() {
            s.tabs = Some(TabSettingsV5 {
                height_mm: None,
                width_mm: None,
                shape: TabShape::Rectangular,
                placement: TabPlacementV5::Automatic {
                    count: Some(4),
                    spacing_mm: None,
                },
            });
        }
    } else {
        s.tabs = None;
    }
    Ok(())
}

/// The tab shape. Ramped shoulders are diagnosed by the planner; the editor
/// only offers rectangular and shows a stored unsupported value read-only.
pub fn set_tab_shape(
    job: &mut CamJobV5,
    operation_id: &str,
    shape: TabShape,
) -> Result<(), String> {
    tab_settings(settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?)?
        .shape = shape;
    Ok(())
}

/// Switch between automatic (count/spacing) and manual (source anchors) tab
/// placement. Manual placement starts empty: no tab is invented.
pub fn set_tab_placement_mode(
    job: &mut CamJobV5,
    operation_id: &str,
    automatic: bool,
) -> Result<(), String> {
    let tabs =
        tab_settings(settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?)?;
    tabs.placement = match (&tabs.placement, automatic) {
        (placement, true) => match placement {
            TabPlacementV5::Automatic { .. } => placement.clone(),
            TabPlacementV5::Manual { .. } => TabPlacementV5::Automatic {
                count: Some(4),
                spacing_mm: None,
            },
        },
        (_, false) => match &tabs.placement {
            TabPlacementV5::Manual { anchors } => TabPlacementV5::Manual {
                anchors: anchors.clone(),
            },
            TabPlacementV5::Automatic { .. } => TabPlacementV5::Manual { anchors: vec![] },
        },
    };
    Ok(())
}

// ---------------------------------------------------------------------------
// Closed contours of the displayed catalogue
// ---------------------------------------------------------------------------

/// One closed contour a profile may cut, with the advisory side the importer
/// derived. Built off the frame thread by [`crate::scene`] and edited here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Contour {
    pub reference: v5::GeometryRef,
    pub wire_id: String,
    pub owner: String,
    pub role: String,
    pub suggested_side: ContourSide,
    pub bounds: [f64; 4],
    pub perimeter_mm: f64,
    pub vertices: Vec<[f64; 2]>,
    /// The ring in the vertex order anchor fractions address. Uniform
    /// placement scaling preserves arc-length fractions, so walking this ring
    /// reproduces the planner's own anchor resolution exactly.
    pub anchor_ring: Vec<[f64; 2]>,
}

/// Every closed contour of every artwork item, owner-qualified.
pub fn contours(job: &CamJobV5) -> Result<Vec<Contour>, String> {
    let catalogue = artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    Ok(catalogue
        .items
        .iter()
        .flat_map(|item| {
            item.entries
                .iter()
                .filter(|entry| entry.kind == v5::GeometryRefKind::ClosedContour)
                .map(|entry| Contour {
                    reference: entry.reference.clone(),
                    wire_id: entry.wire_id.clone(),
                    owner: item.name.clone(),
                    role: match entry.role {
                        cam_core::contours::ContourRole::Outer => "outer",
                        cam_core::contours::ContourRole::Hole => "hole",
                        cam_core::contours::ContourRole::Open => "chain",
                    }
                    .into(),
                    suggested_side: entry.suggested_side,
                    bounds: [
                        entry.bounds.min_x_mm,
                        entry.bounds.min_y_mm,
                        entry.bounds.max_x_mm,
                        entry.bounds.max_y_mm,
                    ],
                    perimeter_mm: entry.perimeter_mm,
                    vertices: item
                        .catalogue
                        .as_ref()
                        .and_then(|catalogue| catalogue.contour(&entry.reference.local_geometry_id))
                        .map(|contour| contour.vertices.iter().map(|p| [p.x, p.y]).collect())
                        .unwrap_or_default(),
                    anchor_ring: item
                        .catalogue
                        .as_ref()
                        .and_then(|catalogue| catalogue.contour(&entry.reference.local_geometry_id))
                        .map(|contour| contour.anchor_ring().iter().map(|p| [p.x, p.y]).collect())
                        .unwrap_or_default(),
                })
        })
        .collect())
}

/// Walk a closed ring by arc-length fraction. This is the numeric equivalent
/// of dragging an anchor: it uses the same ring order the planner resolves.
pub fn ring_point(ring: &[[f64; 2]], fraction: f64) -> Option<[f64; 2]> {
    if ring.len() < 2 {
        return None;
    }
    let lengths: Vec<f64> = ring_lengths(ring);
    let total: f64 = lengths.iter().sum();
    if total <= 0. {
        return Some(ring[0]);
    }
    let target = fraction.rem_euclid(1.) * total;
    let mut walked = 0.;
    for (index, length) in lengths.iter().enumerate() {
        if target <= walked + length || index + 1 == ring.len() {
            let t = if *length > 0. {
                ((target - walked) / length).clamp(0., 1.)
            } else {
                0.
            };
            let a = ring[index];
            let b = ring[(index + 1) % ring.len()];
            return Some([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
        walked += length;
    }
    Some(ring[0])
}

/// Project a setup-space point onto a closed ring: the nearest point on the
/// ring and its arc-length fraction. Used by anchor dragging.
pub fn project_ring(ring: &[[f64; 2]], point: [f64; 2]) -> Option<([f64; 2], f64)> {
    if ring.len() < 2 {
        return None;
    }
    let lengths = ring_lengths(ring);
    let total: f64 = lengths.iter().sum();
    let mut best: Option<(f64, [f64; 2], f64)> = None;
    let mut walked = 0.;
    for (index, length) in lengths.iter().enumerate() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let t = if *length > 0. {
            (((point[0] - a[0]) * dx + (point[1] - a[1]) * dy) / (length * length)).clamp(0., 1.)
        } else {
            0.
        };
        let projected = [a[0] + dx * t, a[1] + dy * t];
        let distance =
            ((point[0] - projected[0]).powi(2) + (point[1] - projected[1]).powi(2)).sqrt();
        if best.is_none_or(|(best_distance, _, _)| distance < best_distance) {
            best = Some((distance, projected, walked + length * t));
        }
        walked += length;
    }
    let (_, projected, arc) = best?;
    Some((
        projected,
        if total > 0. {
            (arc / total).rem_euclid(1.)
        } else {
            0.
        },
    ))
}

fn ring_lengths(ring: &[[f64; 2]]) -> Vec<f64> {
    (0..ring.len())
        .map(|index| {
            let a = ring[index];
            let b = ring[(index + 1) % ring.len()];
            ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt()
        })
        .collect()
}

/// One row of the explicit contour selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SelectionRow {
    pub reference: v5::GeometryRef,
    pub side: ContourSide,
    pub traversal: Option<TraversalDirection>,
}

/// Bind an explicit contour selection. Every reference must exist in the
/// displayed catalogue at its current revision: a stale reference is refused
/// as a whole instead of being rebound.
pub fn select_in(
    job: &CamJobV5,
    operation_id: &str,
    rows: &[SelectionRow],
) -> Result<CamJobV5, String> {
    settings_in(job, operation_id).ok_or("Add a profile operation before selecting contours")?;
    let available = contours(job)?;
    if rows
        .iter()
        .any(|row| !available.iter().any(|c| c.reference == row.reference))
    {
        return Err("Artwork changed; select the profile contours again".into());
    }
    let picks = rows
        .iter()
        .map(|row| v5::commands::ProfileContourPick {
            geometry: artwork::GeometryPick {
                artwork_item_id: row.reference.artwork_item_id.clone(),
                kind: row.reference.kind,
                local_geometry_id: row.reference.local_geometry_id.clone(),
            },
            side: row.side,
            traversal: row.traversal,
        })
        .collect::<Vec<_>>();
    v5::commands::set_contour_selection(job, operation_id, &picks)
        .map(|outcome| outcome.job)
        .map_err(|e| e.to_string())
}

/// The current selection with its stored side and traversal.
pub fn selection(job: &CamJobV5, operation_id: &str) -> Vec<SelectionRow> {
    settings_in(job, operation_id)
        .map(|s| {
            s.contours
                .iter()
                .map(|c| SelectionRow {
                    reference: c.geometry.clone(),
                    side: c.side,
                    traversal: c.traversal,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The stored selection written back as document rows.
pub fn set_selection_rows(
    job: &CamJobV5,
    operation_id: &str,
    rows: &[SelectionRow],
) -> Result<CamJobV5, String> {
    let mut candidate = job.clone();
    let settings =
        settings_in_mut(&mut candidate, operation_id).ok_or("Expected a profile operation")?;
    settings.contours = rows
        .iter()
        .map(|row| ProfileContourV5 {
            geometry: row.reference.clone(),
            side: row.side,
            traversal: row.traversal,
        })
        .collect();
    candidate.validate_structure().map_err(|e| e.to_string())?;
    Ok(candidate)
}

// ---------------------------------------------------------------------------
// Starts and tab anchors
// ---------------------------------------------------------------------------

/// Which anchor list an anchored edit addresses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorKind {
    Start,
    Tab,
}

/// One start or manual-tab anchor addressed by the qualified contour it
/// parameterizes, so its text and its gestures stay attached to that anchor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AnchorRow {
    /// Draft-store scope: the anchor's contour wire ID.
    pub scope: String,
    /// Position in the document's manual-anchor list (tab anchors only).
    pub index: usize,
    pub reference: v5::GeometryRef,
    pub fraction: f64,
    /// False when the anchor's source revision is no longer in the catalogue.
    pub resolved: bool,
}

/// The draft-store scope of one stored anchor: the qualified contour it
/// parameterizes.
pub fn anchor_scope(reference: &v5::GeometryRef) -> String {
    artwork::wire_id(
        &reference.artwork_item_id,
        reference.kind,
        &reference.local_geometry_id,
    )
}

pub fn start_row(job: &CamJobV5, operation_id: &str) -> Option<AnchorRow> {
    let s = settings_in(job, operation_id)?;
    let StartSelectionV5::Anchor(anchor) = &s.start else {
        return None;
    };
    Some(AnchorRow {
        scope: anchor_scope(&anchor.geometry),
        index: 0,
        reference: anchor.geometry.clone(),
        fraction: anchor.fraction_along_source_contour,
        resolved: anchor_resolved(job, anchor),
    })
}

pub fn tab_rows(job: &CamJobV5, operation_id: &str) -> Vec<AnchorRow> {
    let Some(s) = settings_in(job, operation_id) else {
        return vec![];
    };
    match &s.tabs {
        Some(TabSettingsV5 {
            placement: TabPlacementV5::Manual { anchors },
            ..
        }) => anchors
            .iter()
            .enumerate()
            .map(|(index, anchor)| AnchorRow {
                scope: anchor_scope(&anchor.geometry),
                index,
                reference: anchor.geometry.clone(),
                fraction: anchor.fraction_along_source_contour,
                resolved: anchor_resolved(job, anchor),
            })
            .collect(),
        _ => vec![],
    }
}

/// Whether one stored anchor still resolves against the current catalogue:
/// same owner, kind and local ID, and the imported source fingerprint.
fn anchor_resolved(job: &CamJobV5, anchor: &ContourAnchorV5) -> bool {
    let Ok(available) = contours(job) else {
        return false;
    };
    available.iter().any(|contour| {
        contour.reference == anchor.geometry
            && fingerprint_matches(job, &anchor.geometry, &anchor.source_geometry_fingerprint)
    })
}

/// The catalogue fingerprint of one qualified contour, if it still exists.
pub fn fingerprint_matches(job: &CamJobV5, reference: &v5::GeometryRef, fingerprint: &str) -> bool {
    let Ok(catalogue) = artwork::inspect_artwork(job) else {
        return false;
    };
    catalogue
        .items
        .iter()
        .find(|item| item.id == reference.artwork_item_id)
        .and_then(|item| item.catalogue.as_ref())
        .and_then(|catalogue| catalogue.contour(&reference.local_geometry_id))
        .is_some_and(|contour| contour.source_fingerprint == fingerprint)
}

/// The fraction of the anchor whose scope matches, for the draft store's
/// pending-text comparison.
pub fn anchor_value(job: &CamJobV5, operation_id: &str, scope: &str, field: usize) -> Option<f64> {
    let rows = match field {
        97 => tab_rows(job, operation_id),
        108 => start_row(job, operation_id).into_iter().collect(),
        _ => vec![],
    };
    rows.into_iter()
        .find(|row| row.scope == scope)
        .map(|row| row.fraction)
}

/// Move one anchor to a new source fraction. The anchor is addressed by the
/// contour it parameterizes; the stored fingerprint is kept, so an anchor on a
/// replaced source stays unresolved instead of silently rebinding.
pub fn set_anchor_fraction(
    job: &mut CamJobV5,
    operation_id: &str,
    target: AnchorKind,
    scope: &str,
    fraction: f64,
) -> Result<(), String> {
    if !(0. ..1.).contains(&fraction) {
        return Err("Anchor position must be a fraction from 0 up to 1".into());
    }
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    match target {
        AnchorKind::Start => {
            let StartSelectionV5::Anchor(anchor) = &mut s.start else {
                return Err("Choose a start anchor before moving it".into());
            };
            if anchor_scope(&anchor.geometry) != scope {
                return Err("The start anchor changed; move it again".into());
            }
            anchor.fraction_along_source_contour = fraction;
        }
        AnchorKind::Tab => {
            let Some(TabSettingsV5 {
                placement: TabPlacementV5::Manual { anchors },
                ..
            }) = s.tabs.as_mut()
            else {
                return Err("Choose manual tab placement before moving a tab".into());
            };
            let mut moved = false;
            for anchor in anchors.iter_mut() {
                if anchor_scope(&anchor.geometry) == scope {
                    anchor.fraction_along_source_contour = fraction;
                    moved = true;
                }
            }
            if !moved {
                return Err("That tab anchor no longer exists".into());
            }
        }
    }
    Ok(())
}

/// Bind a source anchor to an explicitly chosen closed contour at a fraction.
pub fn bind_anchor(
    job: &mut CamJobV5,
    operation_id: &str,
    wire_id: &str,
    fraction: f64,
) -> Result<ContourAnchorV5, String> {
    if !(0. ..1.).contains(&fraction) {
        return Err("Anchor position must be a fraction from 0 up to 1".into());
    }
    settings_in(job, operation_id).ok_or("Expected a profile operation")?;
    let catalogue = artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    let entry = catalogue
        .entry(wire_id)
        .ok_or("That contour is not in the current artwork")?;
    if entry.kind != v5::GeometryRefKind::ClosedContour {
        return Err("Anchors address closed contours".into());
    }
    Ok(ContourAnchorV5 {
        geometry: entry.reference.clone(),
        source_geometry_fingerprint: entry.source_fingerprint.clone(),
        fraction_along_source_contour: fraction,
    })
}

/// Choose the start: `None` restores the automatic source seam; otherwise the
/// start is pinned to one selected contour at a fraction.
pub fn set_start(
    job: &mut CamJobV5,
    operation_id: &str,
    wire_id: Option<&str>,
    fraction: f64,
) -> Result<(), String> {
    let selection = selection(job, operation_id);
    let start = match wire_id {
        None => StartSelectionV5::Automatic,
        Some(wire_id) => {
            let anchor = bind_anchor(job, operation_id, wire_id, fraction)?;
            if !selection.iter().any(|row| row.reference == anchor.geometry) {
                return Err("Choose a selected contour for the start".into());
            }
            StartSelectionV5::Anchor(Box::new(anchor))
        }
    };
    settings_in_mut(job, operation_id)
        .ok_or("Expected a profile operation")?
        .start = start;
    Ok(())
}

/// Add one manual tab anchor on a selected contour. Manual placement carries
/// one anchor per contour; use the automatic count for several tabs around a
/// single contour.
pub fn add_tab_anchor(
    job: &mut CamJobV5,
    operation_id: &str,
    wire_id: &str,
    fraction: f64,
) -> Result<(), String> {
    let selection = selection(job, operation_id);
    let anchor = bind_anchor(job, operation_id, wire_id, fraction)?;
    if !selection.iter().any(|row| row.reference == anchor.geometry) {
        return Err("Choose a selected contour for the tab".into());
    }
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    let tabs = s.tabs.as_mut().ok_or("Turn tabs on before adding one")?;
    let TabPlacementV5::Manual { anchors } = &mut tabs.placement else {
        return Err("Choose manual tab placement before adding one".into());
    };
    if anchors
        .iter()
        .any(|existing| existing.geometry == anchor.geometry)
    {
        return Err("That contour already carries a manual tab; move or remove it".into());
    }
    anchors.push(anchor);
    Ok(())
}

pub fn remove_tab_anchor(
    job: &mut CamJobV5,
    operation_id: &str,
    scope: &str,
) -> Result<(), String> {
    let s = settings_in_mut(job, operation_id).ok_or("Expected a profile operation")?;
    let tabs = s.tabs.as_mut().ok_or("This operation has no tabs")?;
    let TabPlacementV5::Manual { anchors } = &mut tabs.placement else {
        return Err("Manual tab placement is not selected".into());
    };
    let before = anchors.len();
    anchors.retain(|anchor| anchor_scope(&anchor.geometry) != scope);
    if anchors.len() == before {
        return Err("That tab anchor no longer exists".into());
    }
    Ok(())
}

/// Reattach one unresolved start or tab anchor to an explicitly chosen current
/// contour, keeping its stored fraction unless a replacement is supplied.
pub fn reattach(
    job: &CamJobV5,
    operation_id: &str,
    target: v5::commands::AnchorTarget,
    wire_id: &str,
    fraction: Option<f64>,
) -> Result<CamJobV5, String> {
    let catalogue = artwork::inspect_artwork(job).map_err(|e| e.to_string())?;
    let entry = catalogue
        .entry(wire_id)
        .ok_or("That contour is not in the current artwork")?;
    let pick = artwork::GeometryPick {
        artwork_item_id: entry.reference.artwork_item_id.clone(),
        kind: entry.reference.kind,
        local_geometry_id: entry.reference.local_geometry_id.clone(),
    };
    v5::commands::reattach_anchor(job, operation_id, target, &pick, fraction)
        .map(|outcome| outcome.job)
        .map_err(|e| e.to_string())
}

/// One explicit manual-tab-anchor edit. Every variant names the anchor by the
/// contour it belongs to, so a reordered or re-rendered list cannot redirect
/// the edit to another tab.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum TabAnchorAction {
    /// Bind one new manual anchor on a selected contour at a source fraction.
    Add { wire_id: String, fraction: f64 },
    /// Remove the manual anchor whose contour scope matches.
    Remove { scope: String },
}

/// Apply one manual-tab-anchor edit against the current catalogue. This runs
/// where the catalogue can be imported (the retained service), never in the
/// frame function.
pub fn tab_anchor(
    job: &CamJobV5,
    operation_id: &str,
    action: &TabAnchorAction,
) -> Result<CamJobV5, String> {
    let mut candidate = job.clone();
    match action {
        TabAnchorAction::Add { wire_id, fraction } => {
            add_tab_anchor(&mut candidate, operation_id, wire_id, *fraction)?;
        }
        TabAnchorAction::Remove { scope } => {
            remove_tab_anchor(&mut candidate, operation_id, scope)?;
        }
    }
    candidate.validate_structure().map_err(|e| e.to_string())?;
    Ok(candidate)
}

/// Choose the cut start against the current catalogue: the automatic source
/// seam, or one selected contour at an explicit fraction.
pub fn start_anchor(
    job: &CamJobV5,
    operation_id: &str,
    wire_id: Option<&str>,
    fraction: f64,
) -> Result<CamJobV5, String> {
    let mut candidate = job.clone();
    set_start(&mut candidate, operation_id, wire_id, fraction)?;
    candidate.validate_structure().map_err(|e| e.to_string())?;
    Ok(candidate)
}

/// Explicitly chosen new profile job: the SVG is imported with the default
/// filled interpretation (which yields closed contours for its outlines and
/// holes) and one Profile operation is created with nothing selected and every
/// machining value unset. No cutting, tool or machine preset is invented.
pub fn import_svg(filename: String, svg: String) -> Result<CamJobV5, String> {
    if svg.len() > 8_000_000 {
        return Err("SVG exceeds 8 MB".into());
    }
    let item = v5::ArtworkItem {
        id: v5::ArtworkItemId("artwork-1".into()),
        name: filename.clone(),
        content: v5::ArtworkContent::Svg(cam_core::job::SourceSnapshot {
            filename: filename.clone(),
            svg,
        }),
        import_settings: Default::default(),
        placement: Default::default(),
    };
    let resolved = artwork::resolve_artwork_item(&item).map_err(|e| e.to_string())?;
    if let Some(error) = resolved.import_error {
        return Err(error);
    }
    if !resolved
        .entries
        .iter()
        .any(|entry| entry.kind == v5::GeometryRefKind::ClosedContour)
    {
        return Err("SVG has no closed contours to profile. Filled regions and closed outlines are selectable; convert text to paths first.".into());
    }
    let job = CamJobV5 {
        schema_version: 5,
        name: filename,
        setup: cam_core::project::SetupSettings {
            stock: cam_core::project::StockSetup {
                thickness_mm: None,
                xy: Some(crate::authoring::svg_page_stock(&item)?),
            },
            ..Default::default()
        },
        artwork: vec![item],
        tools: vec![v5::JobToolV5 {
            id: "endmill".into(),
            name: "Endmill".into(),
            geometry: None,
            capabilities: Default::default(),
            library_origin: None,
        }],
        operations: vec![v5::OperationV5 {
            id: "profile".into(),
            name: "Profile".into(),
            enabled: true,
            settings: OperationSettingsV5::Profile(ProfileSettingsV5 {
                contours: vec![],
                assignment: v5::MillingAssignmentV5 {
                    tool_id: "endmill".into(),
                    spindle_rpm: None,
                    spindle_direction: None,
                    cutting_feed_mm_min: None,
                    plunge_feed_mm_min: None,
                    max_stepdown_mm: None,
                    stepover_mm: None,
                    applied_profile: None,
                },
                top: Default::default(),
                bottom: Default::default(),
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
        }],
        tolerances: cam_core::job::PlanningTolerances {
            motion_tolerance_mm: Some(0.01),
            verification_tolerance_mm: Some(0.05),
        },
        machine_configuration: None,
        legacy_machine_profile: None,
    };
    job.validate_structure().map_err(|e| e.to_string())?;
    if job.to_json().map_err(|e| e.to_string())?.len() > 8_000_000 {
        return Err("Embedded SVG exceeds the 8 MB portable job limit".into());
    }
    Ok(job)
}
