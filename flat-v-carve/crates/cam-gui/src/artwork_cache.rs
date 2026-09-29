//! Memoised artwork catalogue resolution for the processes that resolve the
//! same artwork repeatedly: the compute worker inspects the job once per
//! reading (selection validation, the scene projection, the knife and
//! profile readings), so one user action re-imported every item's SVG four
//! or five times.
//!
//! An item's resolution is a pure function of its identity — id, name,
//! placement, and the content revision that already hashes the source text
//! with its interpretation — so a resolution is kept under that key and
//! reused until one of those inputs changes. The assembled whole-job
//! catalogue is kept for the last artwork state; going back and forth
//! between artwork states re-resolves no item, only the assembly.
use cam_core::project::v5::{self, CamJobV5};
use std::{cell::RefCell, collections::HashMap, sync::Arc};

thread_local! {
    /// Resolved item catalogues by item identity. Bounded by retain to the
    /// current document's items whenever a fresh assembly runs.
    static ITEMS: RefCell<HashMap<String, Arc<v5::artwork::ItemCatalogue>>> =
        RefCell::new(HashMap::new());
    /// The assembled catalogue of the last inspected artwork state, so a job
    /// whose artwork did not change returns without any assembly work.
    static ASSEMBLED: RefCell<Option<(String, Arc<v5::artwork::CombinedCatalogue>)>> =
        const { RefCell::new(None) };
}

/// Everything `resolve_artwork_item` reads: the item's id, name, placement,
/// and the content revision (source text plus interpretation). Equal keys
/// resolve to equal catalogues, so the key is a sound memo identity.
fn item_key(item: &v5::ArtworkItem) -> Result<String, String> {
    let revision = item.source_revision().map_err(|e| e.to_string())?;
    Ok(crate::compute::hash(
        &serde_json::to_vec(&(&item.id.0, &item.name, &revision, &item.placement))
            .map_err(|e| e.to_string())?,
    ))
}

/// The artwork state of a whole job: the ordered item identities. Item order
/// is part of it because the assembled catalogue is ordered.
fn job_key(job: &CamJobV5) -> Result<String, String> {
    let mut keys = String::new();
    for item in &job.artwork {
        keys.push_str(&item_key(item)?);
        keys.push('\0');
    }
    Ok(crate::compute::hash(keys.as_bytes()))
}

/// Resolve one item, reusing the kept resolution while its identity holds.
fn resolve_item(item: &v5::ArtworkItem) -> Result<Arc<v5::artwork::ItemCatalogue>, String> {
    let key = item_key(item)?;
    if let Some(kept) = ITEMS.with(|cache| cache.borrow().get(&key).cloned()) {
        return Ok(kept);
    }
    let resolved = Arc::new(v5::artwork::resolve_artwork_item(item).map_err(|e| e.to_string())?);
    ITEMS.with(|cache| cache.borrow_mut().insert(key, Arc::clone(&resolved)));
    Ok(resolved)
}

/// Cached [`v5::artwork::inspect_artwork`]: the same combined catalogue for
/// the same artwork state, resolved once per item instead of once per
/// reading. Error reporting matches the core resolution.
pub(crate) fn inspect(job: &CamJobV5) -> Result<Arc<v5::artwork::CombinedCatalogue>, String> {
    let key = job_key(job)?;
    if let Some((kept, catalogue)) = ASSEMBLED.with(|assembled| assembled.borrow().clone())
        && kept == key
    {
        return Ok(catalogue);
    }
    let items = job
        .artwork
        .iter()
        .map(|item| resolve_item(item).map(|resolved| (*resolved).clone()))
        .collect::<Result<Vec<_>, String>>()?;
    // The same wire-id uniqueness contract as the core inspection.
    let mut wire_ids = std::collections::BTreeSet::new();
    for entry in items.iter().flat_map(|item| item.entries.iter()) {
        if !wire_ids.insert(entry.wire_id.as_str()) {
            return Err(format!(
                "CONTOUR_ID_COLLISION: catalogue entries collide on wire ID '{}'",
                entry.wire_id
            ));
        }
    }
    for entry in items.iter().flat_map(|item| item.point_entries.iter()) {
        if !wire_ids.insert(entry.wire_id.as_str()) {
            return Err(format!(
                "CONTOUR_ID_COLLISION: catalogue entries collide on wire ID '{}'",
                entry.wire_id
            ));
        }
    }
    let catalogue = Arc::new(v5::artwork::CombinedCatalogue { items });
    ASSEMBLED.with(|assembled| {
        *assembled.borrow_mut() = Some((key, Arc::clone(&catalogue)));
    });
    // Keep only this document's items; replaced artwork lets its resolution
    // go the next time anything resolves a fresh assembly.
    let mut keep = Vec::with_capacity(job.artwork.len());
    for item in &job.artwork {
        keep.push(item_key(item)?);
    }
    ITEMS.with(|cache| cache.borrow_mut().retain(|key, _| keep.contains(key)));
    Ok(catalogue)
}

/// Every artwork-derived reading a scene builds from, derived once per
/// artwork state. The catalogue feeds the pickers and the command gates;
/// the components, chains, contours and marker points feed the scene report
/// the display adopts; the contour points are the shared outline geometry
/// every scene normalizes against its own bounds.
pub(crate) struct Projections {
    /// Identity of the artwork state these projections resolve.
    pub key: String,
    pub components: Vec<crate::authoring::Component>,
    pub chains: Vec<crate::knife::Chain>,
    pub contours: Vec<crate::profile::Contour>,
    pub points: Vec<crate::scene::ScenePoint>,
    /// Outline line segments in setup millimetres with source colour.
    pub contour_points: Vec<([f64; 3], [f32; 4])>,
    /// `[item id, start, len]` into `contour_points`, in draw order.
    pub spans: Vec<(String, usize, usize)>,
    /// Union of the placed components, chains and marker points; `None`
    /// when the artwork places nothing.
    pub artwork_bounds: Option<[f64; 4]>,
}

thread_local! {
    /// The projections of the last artwork state.
    static PROJECTIONS: RefCell<Option<(String, Arc<Projections>)>> = const { RefCell::new(None) };
}

/// The shared artwork readings for `job`, derived once per artwork state: a
/// command whose artwork did not change — an operation edit, a revalidation,
/// a preview — reuses them instead of re-deriving every projection.
pub(crate) fn projections(job: &CamJobV5) -> Result<Arc<Projections>, String> {
    let key = job_key(job)?;
    if let Some((kept, projections)) = PROJECTIONS.with(|cache| cache.borrow().clone())
        && kept == key
    {
        return Ok(projections);
    }
    let catalogue = inspect(job)?;
    let components = crate::authoring::catalogue_components(&catalogue);
    let chains = crate::knife::chains_of(&catalogue);
    let contours = crate::profile::contours_of(&catalogue);
    let points: Vec<crate::scene::ScenePoint> = catalogue
        .items
        .iter()
        .flat_map(|item| {
            item.point_entries
                .iter()
                .map(|point| crate::scene::ScenePoint {
                    reference: point.reference.clone(),
                    center: [point.center.x, point.center.y],
                    diameter_mm: point.diameter_mm,
                    paint: point.paint,
                    exact: point.exact,
                })
        })
        .collect();
    let mut contour_points: Vec<([f64; 3], [f32; 4])> = Vec::new();
    let mut spans = Vec::new();
    for component in &components {
        let start = contour_points.len();
        let color = crate::scene::artwork_color(component.paint);
        for ring in &component.rings {
            for i in 0..ring.len() {
                for p in [ring[i], ring[(i + 1) % ring.len()]] {
                    contour_points.push(([p[0], p[1], 0.02], color));
                }
            }
        }
        spans.push((
            component.reference.artwork_item_id.0.clone(),
            start,
            contour_points.len(),
        ));
    }
    for chain in &chains {
        let start = contour_points.len();
        let color = crate::scene::artwork_color(chain.paint);
        let segments = chain
            .vertices
            .len()
            .saturating_sub(usize::from(!chain.closed));
        for i in 0..segments {
            for p in [
                chain.vertices[i],
                chain.vertices[(i + 1) % chain.vertices.len()],
            ] {
                contour_points.push(([p[0], p[1], 0.02], color));
            }
        }
        spans.push((
            chain.reference.artwork_item_id.0.clone(),
            start,
            contour_points.len(),
        ));
    }
    let mut artwork_bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for component in &components {
        artwork_bounds[0] = artwork_bounds[0].min(component.bounds[0]);
        artwork_bounds[1] = artwork_bounds[1].min(component.bounds[1]);
        artwork_bounds[2] = artwork_bounds[2].max(component.bounds[2]);
        artwork_bounds[3] = artwork_bounds[3].max(component.bounds[3]);
    }
    for point in chains.iter().flat_map(|chain| chain.vertices.iter()) {
        artwork_bounds[0] = artwork_bounds[0].min(point[0]);
        artwork_bounds[1] = artwork_bounds[1].min(point[1]);
        artwork_bounds[2] = artwork_bounds[2].max(point[0]);
        artwork_bounds[3] = artwork_bounds[3].max(point[1]);
    }
    for point in &points {
        artwork_bounds[0] = artwork_bounds[0].min(point.center[0]);
        artwork_bounds[1] = artwork_bounds[1].min(point.center[1]);
        artwork_bounds[2] = artwork_bounds[2].max(point.center[0]);
        artwork_bounds[3] = artwork_bounds[3].max(point.center[1]);
    }
    let artwork_bounds = artwork_bounds[0].is_finite().then_some(artwork_bounds);
    let projections = Arc::new(Projections {
        key: key.clone(),
        components,
        chains,
        contours,
        points,
        contour_points,
        spans,
        artwork_bounds,
    });
    PROJECTIONS.with(|cache| {
        *cache.borrow_mut() = Some((key, Arc::clone(&projections)));
    });
    Ok(projections)
}

/// Whether a reply for `job` should carry the artwork report fields: the
/// display says which artwork key it already holds, and the fields travel
/// only when that is not this job's artwork state. An unreadable artwork
/// state ships them, so the failure path keeps today's behaviour.
pub(crate) fn reply_includes_artwork(job: &CamJobV5, have: Option<&str>) -> bool {
    match (job_key(job), have) {
        (Ok(key), Some(have)) => key != have,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cam_core::job::SourceSnapshot;

    const ONE_RECT: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='40mm' height='20mm' \
        viewBox='0 0 40 20'><path id='a' fill='#000' d='M0 0H20V20H0Z'/></svg>";
    const TWO_RECTS: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='40mm' height='20mm' \
        viewBox='0 0 40 20'><path id='a' fill='#000' d='M0 0H20V20H0Z'/><path id='b' fill='#000' \
        d='M20 0H40V20H20Z'/></svg>";

    fn imported() -> CamJobV5 {
        crate::authoring::import_svg("art.svg".into(), ONE_RECT.into()).unwrap()
    }

    /// The same artwork state — including from a different job value —
    /// shares one resolution.
    #[test]
    fn unchanged_artwork_shares_one_resolution() {
        let job = imported();
        let first = inspect(&job).unwrap();
        assert!(Arc::ptr_eq(&first, &inspect(&job).unwrap()));
        let mut edited = job.clone();
        edited.name = "a job edit that cannot touch artwork".into();
        assert!(Arc::ptr_eq(&first, &inspect(&edited).unwrap()));
    }

    /// Replaced content resolves again; the new resolution is the new
    /// artwork, and the old state still resolves without re-importing.
    #[test]
    fn replaced_content_resolves_again() {
        let job = imported();
        let id = job.artwork[0].id.clone();
        let before = inspect(&job).unwrap();
        let replaced = cam_core::project::v5::commands::replace_artwork(
            &job,
            &id,
            SourceSnapshot {
                filename: "two.svg".into(),
                svg: TWO_RECTS.into(),
            },
            None,
        )
        .unwrap()
        .job;
        let after = inspect(&replaced).unwrap();
        assert!(!Arc::ptr_eq(&before, &after));
        let count = |c: &v5::artwork::CombinedCatalogue| {
            c.items[0]
                .entries
                .iter()
                .filter(|e| e.kind == v5::GeometryRefKind::FilledComponent)
                .count()
        };
        assert_eq!(count(&before), 1);
        assert_eq!(count(&after), 2, "the replacement's own reading");
        // The earlier item resolves again after the replacement released it,
        // and its resolution is then kept while its identity holds.
        let item_before = resolve_item(&job.artwork[0]).unwrap();
        assert!(Arc::ptr_eq(
            &item_before,
            &resolve_item(&job.artwork[0]).unwrap()
        ));
    }

    /// A rename or a placement move is a different reading of the same
    /// content, so both rekey the resolution.
    #[test]
    fn renames_and_placements_rekey() {
        let job = imported();
        let id = job.artwork[0].id.clone();
        let before = inspect(&job).unwrap();
        let renamed = cam_core::project::v5::commands::rename_artwork(&job, &id, "moved art")
            .unwrap()
            .job;
        let after = inspect(&renamed).unwrap();
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(after.items[0].name, "moved art");
        let mut moved = job.clone();
        moved.artwork[0].placement.origin_mm.x += 5.;
        assert!(!Arc::ptr_eq(&before, &inspect(&moved).unwrap()));
    }

    /// The cached resolution is the core resolution: same entries, same
    /// order, same wire IDs, for a job that imports cleanly.
    #[test]
    fn matches_the_core_resolution() {
        let job = imported();
        let cached = inspect(&job).unwrap();
        let direct = v5::artwork::inspect_artwork(&job).unwrap();
        let wires = |c: &v5::artwork::CombinedCatalogue| {
            c.items
                .iter()
                .flat_map(|item| item.entries.iter().map(|e| e.wire_id.clone()))
                .collect::<Vec<_>>()
        };
        assert_eq!(wires(&cached), wires(&direct));
        assert_eq!(cached.items.len(), direct.items.len());
    }

    /// The scene-level readings are derived once per artwork state: a job
    /// whose artwork stands shares them across commands, and a placement
    /// move re-derives them.
    #[test]
    fn projections_follow_the_artwork_state() {
        let job = imported();
        let first = projections(&job).unwrap();
        assert!(Arc::ptr_eq(&first, &projections(&job).unwrap()));
        let mut renamed_job = job.clone();
        renamed_job.name = "a job edit that cannot touch artwork".into();
        assert!(Arc::ptr_eq(&first, &projections(&renamed_job).unwrap()));
        let mut moved = job;
        moved.artwork[0].placement.origin_mm.x += 1.;
        let next = projections(&moved).unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        let shift = next.artwork_bounds.unwrap()[0] - first.artwork_bounds.unwrap()[0];
        assert!((shift + 1.).abs() < 1e-9, "placed bounds follow the move");
    }
}
