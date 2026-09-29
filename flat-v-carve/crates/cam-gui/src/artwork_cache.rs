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
}
