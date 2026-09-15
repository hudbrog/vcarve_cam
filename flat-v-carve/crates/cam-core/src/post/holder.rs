//! Holder geometry, by catalogue name.
//!
//! A machine configuration names the holder it uses (an ER collet chuck, say)
//! and this module resolves the name into a body. Nothing here is machining
//! truth: a holder changes no toolpath and no emitted program. It exists so the
//! simulation can draw the assembly that will actually be bolted to the spindle
//! and warn when it would hit the job.
//!
//! ## The catalogue's numbers
//!
//! A body is a stack of truncated cones — the same representation Fusion uses
//! for its `holder` and `shaft` blocks (see the machine-simulation plan §2):
//! `height`, `lower-diameter`, `upper-diameter`.
//!
//! The **nut outside diameters** are the published DIN 6499 / Rego-Fix ER series
//! sizes: ER11 19 mm, ER16 22 mm, ER20 25 mm, ER25 32 mm, ER32 40 mm, ER40 50 mm.
//! The nut is the part of a collet chuck that hangs lowest and widest, so it is
//! the surface a collision check has to clear.
//!
//! The **nut heights and the chuck bodies above them are nominal**, not a
//! catalogue: chuck bodies differ by manufacturer and by the machine's spindle
//! interface. They are drawn so the assembly reads as a body rather than a ring,
//! and a machine that needs exact geometry carries its own `segments` on the
//! selection. Nothing in this module claims a dimension it was not given.
use serde::{Deserialize, Serialize};

/// One trunk of a holder body: a truncated cone from `height_mm` above its base.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HolderSegment {
    pub height_mm: f64,
    pub lower_diameter_mm: f64,
    pub upper_diameter_mm: f64,
}

impl HolderSegment {
    pub fn validate(&self) -> std::result::Result<(), String> {
        for (value, name) in [
            (self.height_mm, "height_mm"),
            (self.lower_diameter_mm, "lower_diameter_mm"),
            (self.upper_diameter_mm, "upper_diameter_mm"),
        ] {
            if !value.is_finite() || value <= 0. {
                return Err(format!("holder segment {name} must be finite and positive"));
            }
        }
        Ok(())
    }
}

/// What a machine says it holds the tool with: a catalogue entry by id, or the
/// machine's own `segments`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HolderSelection {
    /// Catalogue id, for example `"er20"`, or `"custom"` with `segments`.
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub segments: Vec<HolderSegment>,
}

impl HolderSelection {
    /// A selection of a catalogue entry.
    pub fn catalogue(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            segments: Vec::new(),
        }
    }

    /// The body this selection resolves to: the machine's own segments when it
    /// carries them, otherwise the catalogue entry, otherwise nothing — a name
    /// nobody knows is not a body.
    pub fn body(&self) -> Option<Vec<HolderSegment>> {
        if !self.segments.is_empty() {
            return Some(self.segments.clone());
        }
        catalogue_entry(&self.id).map(|entry| entry.segments())
    }

    pub fn validate(&self) -> std::result::Result<(), String> {
        if self.id.is_empty() || self.id.len() > 40 || !self.id.is_ascii() {
            return Err("holder id must be 1..=40 ASCII characters".into());
        }
        if !self.segments.is_empty() && self.segments.len() > 8 {
            return Err("a holder body has at most 8 segments".into());
        }
        for segment in &self.segments {
            segment.validate()?;
        }
        Ok(())
    }
}

/// A catalogue entry: the standard collet nut plus a nominal chuck body.
pub struct CatalogueEntry {
    pub id: &'static str,
    pub label: &'static str,
    /// Published DIN 6499 nut outside diameter.
    pub nut_diameter_mm: f64,
}

impl CatalogueEntry {
    /// Nut first, then a body of the same diameter: a chuck body is never
    /// narrower than its nut in practice, and drawing it no wider keeps the
    /// catalogue from claiming a dimension the standard does not state.
    pub fn segments(&self) -> Vec<HolderSegment> {
        vec![
            HolderSegment {
                height_mm: self.nut_diameter_mm * 0.85,
                lower_diameter_mm: self.nut_diameter_mm,
                upper_diameter_mm: self.nut_diameter_mm,
            },
            HolderSegment {
                height_mm: self.nut_diameter_mm * 1.6,
                lower_diameter_mm: self.nut_diameter_mm,
                upper_diameter_mm: self.nut_diameter_mm,
            },
        ]
    }
}

/// The holders a machine may select. Only the nut diameter is a standard
/// dimension; see this module's header for what is nominal.
pub const CATALOGUE: [CatalogueEntry; 6] = [
    CatalogueEntry {
        id: "er11",
        label: "ER11 collet chuck",
        nut_diameter_mm: 19.,
    },
    CatalogueEntry {
        id: "er16",
        label: "ER16 collet chuck",
        nut_diameter_mm: 22.,
    },
    CatalogueEntry {
        id: "er20",
        label: "ER20 collet chuck",
        nut_diameter_mm: 25.,
    },
    CatalogueEntry {
        id: "er25",
        label: "ER25 collet chuck",
        nut_diameter_mm: 32.,
    },
    CatalogueEntry {
        id: "er32",
        label: "ER32 collet chuck",
        nut_diameter_mm: 40.,
    },
    CatalogueEntry {
        id: "er40",
        label: "ER40 collet chuck",
        nut_diameter_mm: 50.,
    },
];

pub fn catalogue_entry(id: &str) -> Option<&'static CatalogueEntry> {
    CATALOGUE.iter().find(|entry| entry.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_catalogue_name_resolves_to_a_nut_of_the_published_diameter() {
        for (id, diameter) in [
            ("er11", 19.),
            ("er16", 22.),
            ("er20", 25.),
            ("er25", 32.),
            ("er32", 40.),
            ("er40", 50.),
        ] {
            let body = HolderSelection::catalogue(id)
                .body()
                .expect("a known holder");
            assert_eq!(body[0].lower_diameter_mm, diameter, "{id} nut");
            assert_eq!(body[0].upper_diameter_mm, diameter, "{id} nut");
            assert!(body[0].height_mm > 0.);
            for segment in &body {
                segment.validate().unwrap();
            }
        }
        // A body never claims to be narrower than its own nut.
        for entry in &CATALOGUE {
            for segment in entry.segments() {
                assert!(segment.lower_diameter_mm >= entry.nut_diameter_mm * 0.999);
                assert!(segment.upper_diameter_mm >= entry.nut_diameter_mm * 0.999);
            }
        }
    }

    #[test]
    fn a_machine_can_carry_its_own_segments_instead_of_a_catalogue_name() {
        let custom = HolderSelection {
            id: "custom".into(),
            segments: vec![HolderSegment {
                height_mm: 30.,
                lower_diameter_mm: 24.,
                upper_diameter_mm: 30.,
            }],
        };
        custom.validate().unwrap();
        assert_eq!(custom.body().unwrap().len(), 1);
        // An unknown name is not a body, and it is not an error either: the
        // display says nothing is modelled rather than inventing one.
        assert!(HolderSelection::catalogue("er99").body().is_none());
        assert!(HolderSelection::catalogue("er99").validate().is_ok());
        assert!(HolderSelection::catalogue("").validate().is_err());
        assert!(
            HolderSelection {
                id: "custom".into(),
                segments: vec![HolderSegment {
                    height_mm: 0.,
                    lower_diameter_mm: 20.,
                    upper_diameter_mm: 20.,
                }],
            }
            .validate()
            .is_err()
        );
    }
}
