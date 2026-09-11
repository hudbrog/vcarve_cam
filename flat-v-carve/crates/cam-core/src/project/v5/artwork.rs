//! Document-level artwork resolution (plan section 22.4): per-item import
//! through the one existing SVG importer, and the combined owner-qualified
//! catalogue the UI picks from. This module is the single boundary that
//! turns an [`ArtworkItem`] into catalogue geometry; reference inspection
//! and the H2 document commands both consume it, so there is no second SVG
//! machining importer and no filename-based geometry matching anywhere.
//!
//! Owner-qualified wire IDs are derived as `"{item}:{kind}:{local}"`. Item
//! IDs use the document identifier alphabet (no colons) and the kind tag is
//! fixed, so splitting at the first two colons reverses the mapping even
//! though local geometry IDs such as `plate::0` may contain colons. Two
//! owners therefore never collide — not with identical bytes, identical
//! filenames or repeated local IDs — and no raw filename is ever embedded.
use super::{
    ArtworkItem, ArtworkItemId, CamJobV5, GeometryRef, GeometryRefKind, SourceRevision, error,
};
use crate::{
    contours::{Contour, ContourCatalogue, ContourRole},
    geometry::{Point, Result},
    project::ContourSide,
};
use std::collections::BTreeSet;

/// Build one item's catalogue by reassembling a temporary frozen-schema-4
/// import job around the item's snapshot, interpretation and placement. This
/// keeps the contour/chain ID derivation and fingerprint algorithm in one
/// authority (plan section 22.2: reassemble `ImportOptions` only here).
pub fn item_catalogue(item: &ArtworkItem) -> Result<ContourCatalogue> {
    let snapshot = match &item.content {
        super::ArtworkContent::Svg(snapshot) => snapshot.clone(),
    };
    let job = crate::project::CamJob {
        schema_version: crate::project::CAM_JOB_SCHEMA_VERSION,
        name: String::new(),
        source: Some(snapshot),
        import: item.import_options(),
        setup: Default::default(),
        tools: vec![],
        operations: vec![],
        tolerances: Default::default(),
        legacy_machine_profile: None,
    };
    ContourCatalogue::build(&job)
}

/// Axis-aligned bounds in setup coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetupBounds {
    pub min_x_mm: f64,
    pub min_y_mm: f64,
    pub max_x_mm: f64,
    pub max_y_mm: f64,
}

impl SetupBounds {
    fn of_vertices(vertices: &[Point]) -> Option<Self> {
        let first = vertices.first()?;
        let mut bounds = Self {
            min_x_mm: first.x,
            min_y_mm: first.y,
            max_x_mm: first.x,
            max_y_mm: first.y,
        };
        for point in &vertices[1..] {
            bounds.min_x_mm = bounds.min_x_mm.min(point.x);
            bounds.min_y_mm = bounds.min_y_mm.min(point.y);
            bounds.max_x_mm = bounds.max_x_mm.max(point.x);
            bounds.max_y_mm = bounds.max_y_mm.max(point.y);
        }
        Some(bounds)
    }

    pub fn union(self, other: Self) -> Self {
        Self {
            min_x_mm: self.min_x_mm.min(other.min_x_mm),
            min_y_mm: self.min_y_mm.min(other.min_y_mm),
            max_x_mm: self.max_x_mm.max(other.max_x_mm),
            max_y_mm: self.max_y_mm.max(other.max_y_mm),
        }
    }
}

/// A user's pick of one catalogue entry: the owning item, the local geometry
/// space and its local ID. The binding source revision is attached by the
/// catalogue/command, never supplied by the caller, so an assignment cannot
/// carry a stale revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GeometryPick {
    pub artwork_item_id: ArtworkItemId,
    pub kind: GeometryRefKind,
    pub local_geometry_id: String,
}

const COMPONENT_TAG: &str = "component";
const CONTOUR_TAG: &str = "contour";
const CHAIN_TAG: &str = "chain";

fn kind_tag(kind: GeometryRefKind) -> &'static str {
    match kind {
        GeometryRefKind::FilledComponent => COMPONENT_TAG,
        GeometryRefKind::ClosedContour => CONTOUR_TAG,
        GeometryRefKind::Centerline => CHAIN_TAG,
    }
}

fn kind_from_tag(tag: &str) -> Option<GeometryRefKind> {
    match tag {
        COMPONENT_TAG => Some(GeometryRefKind::FilledComponent),
        CONTOUR_TAG => Some(GeometryRefKind::ClosedContour),
        CHAIN_TAG => Some(GeometryRefKind::Centerline),
        _ => None,
    }
}

/// The derived wire ID of one catalogue entry: opaque, but carrying an
/// explicit owner and reversible to its pick (plan section 22.3).
pub fn wire_id(item: &ArtworkItemId, kind: GeometryRefKind, local_id: &str) -> String {
    format!("{}:{}:{}", item.0, kind_tag(kind), local_id)
}

/// Reverse of [`wire_id`]. Item IDs cannot contain colons, so the first two
/// separators are unambiguous however many colons the local ID carries.
pub fn parse_wire_id(wire: &str) -> Option<GeometryPick> {
    let mut parts = wire.splitn(3, ':');
    let item = parts.next()?.to_string();
    let kind = kind_from_tag(parts.next()?)?;
    let local = parts.next()?;
    if !crate::preview::valid_id(&item) || local.is_empty() {
        return None;
    }
    Some(GeometryPick {
        artwork_item_id: ArtworkItemId(item),
        kind,
        local_geometry_id: local.to_string(),
    })
}

/// One owner-qualified catalogue entry presented to picking UIs: display and
/// assignment metadata with bounded derived data (no full vertex lists —
/// planner-facing resolved geometry is H3, inspection DTOs are H4).
#[derive(Clone, Debug, PartialEq)]
pub struct CatalogueEntry {
    pub wire_id: String,
    /// Canonical reference at the item's current source revision.
    pub reference: GeometryRef,
    pub kind: GeometryRefKind,
    pub role: ContourRole,
    pub closed: bool,
    /// Advisory side (outside for outers, inside for holes, on for chains);
    /// the stored side stays an explicit assignment choice.
    pub suggested_side: ContourSide,
    pub source_fingerprint: String,
    pub bounds: SetupBounds,
    pub perimeter_mm: f64,
}

/// One item's resolved state: current revision, the imported catalogue when
/// the content imports cleanly, and owner-qualified entries.
#[derive(Clone, Debug)]
pub struct ItemCatalogue {
    pub id: ArtworkItemId,
    pub name: String,
    pub revision: SourceRevision,
    /// `None` when this item's content fails to import; the error text is
    /// preserved so inspection and commands can report it.
    pub catalogue: Option<ContourCatalogue>,
    pub import_error: Option<String>,
    pub entries: Vec<CatalogueEntry>,
}

fn entry(
    item: &ArtworkItemId,
    revision: &SourceRevision,
    kind: GeometryRefKind,
    contour: &Contour,
) -> Option<CatalogueEntry> {
    Some(CatalogueEntry {
        wire_id: wire_id(item, kind, &contour.id),
        reference: GeometryRef {
            artwork_item_id: item.clone(),
            kind,
            local_geometry_id: contour.id.clone(),
            source_revision: revision.clone(),
        },
        kind,
        role: contour.role,
        closed: contour.closed,
        suggested_side: contour.suggested_side(),
        source_fingerprint: contour.source_fingerprint.clone(),
        bounds: SetupBounds::of_vertices(&contour.vertices)?,
        perimeter_mm: contour.perimeter_mm,
    })
}

/// Resolve one artwork item: import its content through the single importer
/// boundary and derive owner-qualified entries (filled components from their
/// outer contours, closed contours, centerline chains). Import failure is
/// resolved state, not an error: the item stays inspectable and saveable.
pub fn resolve_artwork_item(item: &ArtworkItem) -> Result<ItemCatalogue> {
    let revision = item.source_revision()?;
    let resolved = item_catalogue(item);
    let (catalogue, import_error) = match resolved {
        Ok(catalogue) => (Some(catalogue), None),
        Err(e) => (None, Some(e.message)),
    };
    let mut entries = vec![];
    if let Some(catalogue) = &catalogue {
        let id = &item.id;
        for contour in &catalogue.contours {
            entries.extend(entry(
                id,
                &revision,
                GeometryRefKind::ClosedContour,
                contour,
            ));
        }
        for chain in &catalogue.open_chains {
            entries.extend(entry(id, &revision, GeometryRefKind::Centerline, chain));
        }
        // Filled components are addressed by the importer's component ID;
        // every component owns exactly one outer contour, which carries its
        // fingerprint and bounds.
        let mut seen = BTreeSet::new();
        for contour in &catalogue.contours {
            if contour.role == ContourRole::Outer && seen.insert(contour.component_id.clone()) {
                let component = Contour {
                    id: contour.component_id.clone(),
                    component_id: contour.component_id.clone(),
                    ..contour.clone()
                };
                entries.extend(entry(
                    id,
                    &revision,
                    GeometryRefKind::FilledComponent,
                    &component,
                ));
            }
        }
    }
    Ok(ItemCatalogue {
        id: item.id.clone(),
        name: item.name.clone(),
        revision,
        catalogue,
        import_error,
        entries,
    })
}

/// The combined catalogue over every item: owner-qualified entries with
/// distinct identities, in item order. It is not a global union of the
/// artwork — Flat V-carve unions only its selected filled regions (H3) —
/// and coincident cuts across items stay separately selectable here.
#[derive(Clone, Debug, Default)]
pub struct CombinedCatalogue {
    pub items: Vec<ItemCatalogue>,
}

impl CombinedCatalogue {
    pub fn item(&self, id: &ArtworkItemId) -> Option<&ItemCatalogue> {
        self.items.iter().find(|item| item.id == *id)
    }

    pub fn entry(&self, wire: &str) -> Option<&CatalogueEntry> {
        self.items
            .iter()
            .flat_map(|item| item.entries.iter())
            .find(|entry| entry.wire_id == wire)
    }

    /// Union of the placed geometry bounds of the explicitly listed items.
    /// Hidden or locked items participate when explicitly included; their
    /// visibility is workspace state this document-level resolver never sees.
    pub fn placed_bounds(&self, ids: &[ArtworkItemId]) -> Result<SetupBounds> {
        let mut bounds: Option<SetupBounds> = None;
        for id in ids {
            let item = self.item(id).ok_or_else(|| {
                error(
                    "ARTWORK_REFERENCE",
                    format!("unknown artwork item '{}'", id.0),
                )
            })?;
            for entry in &item.entries {
                bounds = Some(match bounds {
                    Some(accumulated) => accumulated.union(entry.bounds),
                    None => entry.bounds,
                });
            }
        }
        bounds.ok_or_else(|| {
            error(
                "ARTWORK_COMMAND",
                "the selected artwork has no placed geometry to fit stock against",
            )
        })
    }
}

/// Resolve every item of the job into the combined catalogue (plan section
/// 22.4: `inspect_artwork`). Items importing cleanly is not required; their
/// failures are reported through the resolved state.
pub fn inspect_artwork(job: &CamJobV5) -> Result<CombinedCatalogue> {
    let mut items = vec![];
    for item in &job.artwork {
        items.push(resolve_artwork_item(item)?);
    }
    // Wire IDs are unique by construction: item IDs are unique in the
    // document, local geometry IDs are unique inside one item's catalogue
    // (and across its contour/chain halves), and the kind tag separates the
    // remaining spaces. The check stays as a build-time guarantee.
    let mut wire_ids = BTreeSet::new();
    for entry in items.iter().flat_map(|item| item.entries.iter()) {
        if !wire_ids.insert(entry.wire_id.as_str()) {
            return Err(error(
                "CONTOUR_ID_COLLISION",
                format!("catalogue entries collide on wire ID '{}'", entry.wire_id),
            ));
        }
    }
    Ok(CombinedCatalogue { items })
}
