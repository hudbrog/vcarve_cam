//! Solver-independent, versioned 2D artwork. Coordinates are accepted local
//! millimeters, Y up. Reading, validating and machining never invoke a solver.
pub mod export;
pub mod materialize;
mod validation;
use crate::geometry::{Diagnostic, Point, Result};
use serde::{Deserialize, Serialize};
pub use validation::{ConstraintEvaluation, ValidationReport};

pub const SKETCH_VERSION: u32 = 1;
pub const MAX_ENTITIES: usize = 500;
pub const MAX_CONSTRAINTS: usize = 2000;
pub const MAX_WIRES: usize = 500;
pub const MAX_COORDINATE_MM: f64 = 100_000.;
/// Numerical acceptance, distinct from topology and CAM flattening tolerances.
pub const RESIDUAL_TOLERANCE_MM: f64 = 1e-6;
pub const MAX_SKETCH_BYTES: usize = 2_000_000;

// Built-in references cannot collide with authored IDs (which forbid '@'). They
// are immutable local construction geometry, never stored as entities or wires.
pub const ORIGIN_ID: &str = "@origin";
pub const X_AXIS_ID: &str = "@x_axis";
pub const Y_AXIS_ID: &str = "@y_axis";

pub fn reference_entities() -> &'static [Entity; 3] {
    static REFERENCES: std::sync::LazyLock<[Entity; 3]> = std::sync::LazyLock::new(|| {
        [
            (
                ORIGIN_ID,
                "Origin",
                SketchGeometry::Point {
                    position: Point::new(0., 0.),
                },
            ),
            (
                X_AXIS_ID,
                "X axis",
                SketchGeometry::Line {
                    start: Point::new(0., 0.),
                    end: Point::new(1., 0.),
                },
            ),
            (
                Y_AXIS_ID,
                "Y axis",
                SketchGeometry::Line {
                    start: Point::new(0., 0.),
                    end: Point::new(0., 1.),
                },
            ),
        ]
        .map(|(id, label, geometry)| Entity {
            id: id.into(),
            label: Some(label.into()),
            construction: true,
            geometry,
        })
    });
    &REFERENCES
}

pub fn reference_entity(id: &str) -> Option<&'static Entity> {
    reference_entities().iter().find(|e| e.id == id)
}

pub fn origin_ref() -> PointRef {
    PointRef {
        entity_id: ORIGIN_ID.into(),
        point: PointRole::Position,
    }
}

pub(crate) fn error(code: &str, message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("sketch")
}
pub(crate) fn located(code: &str, id: &str, message: impl Into<String>) -> Diagnostic {
    let mut d = error(code, message);
    d.source_id = Some(id.into());
    d
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchDocument {
    pub sketch_version: u32,
    /// Fixed local page envelope. Never inferred from geometry or used to resize stock.
    pub frame: SketchFrame,
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
    /// Persistent explicit connectivity. Nesting/regions are derived and verified.
    pub wires: Vec<Wire>,
}
impl Default for SketchDocument {
    fn default() -> Self {
        Self {
            sketch_version: SKETCH_VERSION,
            frame: SketchFrame::default(),
            entities: vec![],
            constraints: vec![],
            wires: vec![],
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchFrame {
    pub min: Point,
    pub width_mm: f64,
    pub height_mm: f64,
}
impl Default for SketchFrame {
    fn default() -> Self {
        Self {
            min: Point::new(-1000., -1000.),
            width_mm: 2000.,
            height_mm: 2000.,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entity {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub construction: bool,
    pub geometry: SketchGeometry,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SketchGeometry {
    Point {
        position: Point,
    },
    Line {
        start: Point,
        end: Point,
    },
    Circle {
        center: Point,
        radius_mm: f64,
    },
    Arc {
        center: Point,
        radius_mm: f64,
        start_angle_rad: f64,
        sweep_angle_rad: f64,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PointRef {
    pub entity_id: String,
    pub point: PointRole,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PointRole {
    Position,
    Start,
    End,
    Center,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wire {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub edges: Vec<WireEdge>,
    pub closed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WireEdge {
    pub entity_id: String,
    #[serde(default)]
    pub reversed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraint {
    pub id: String,
    #[serde(default)]
    pub reference: bool,
    pub definition: ConstraintKind,
}
/// Only reference dimensions are non-driving. Geometric constraints are mandatory.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConstraintKind {
    Coincident {
        a: PointRef,
        b: PointRef,
    },
    Horizontal {
        line: String,
    },
    Vertical {
        line: String,
    },
    Parallel {
        a: String,
        b: String,
    },
    Perpendicular {
        a: String,
        b: String,
    },
    EqualLength {
        a: String,
        b: String,
    },
    EqualRadius {
        a: String,
        b: String,
    },
    /// Tangent to the circle supporting an arc; endpoint contact is explicit coincidence.
    Tangent {
        line: String,
        curve: String,
    },
    PointOnLine {
        point: PointRef,
        line: String,
    },
    PointOnCircle {
        point: PointRef,
        curve: String,
    },
    Midpoint {
        point: PointRef,
        line: String,
    },
    Symmetry {
        a: PointRef,
        b: PointRef,
        line: String,
    },
    Distance {
        a: PointRef,
        b: PointRef,
        value_mm: f64,
    },
    HorizontalDistance {
        a: PointRef,
        b: PointRef,
        value_mm: f64,
    },
    VerticalDistance {
        a: PointRef,
        b: PointRef,
        value_mm: f64,
    },
    Length {
        line: String,
        value_mm: f64,
    },
    Radius {
        curve: String,
        value_mm: f64,
    },
    Diameter {
        curve: String,
        value_mm: f64,
    },
    Angle {
        a: String,
        b: String,
        value_rad: f64,
    },
    Fixed {
        entity: String,
        target: SketchGeometry,
    },
    FixedPoint {
        point: PointRef,
        position: Point,
    },
}
impl ConstraintKind {
    pub fn is_dimension(&self) -> bool {
        matches!(
            self,
            Self::Distance { .. }
                | Self::HorizontalDistance { .. }
                | Self::VerticalDistance { .. }
                | Self::Length { .. }
                | Self::Radius { .. }
                | Self::Diameter { .. }
                | Self::Angle { .. }
        )
    }
}
impl SketchGeometry {
    pub fn point(&self, role: PointRole) -> Option<Point> {
        match (self, role) {
            (Self::Point { position }, PointRole::Position) => Some(*position),
            (Self::Line { start, .. }, PointRole::Start) => Some(*start),
            (Self::Line { end, .. }, PointRole::End) => Some(*end),
            (Self::Circle { center, .. } | Self::Arc { center, .. }, PointRole::Center) => {
                Some(*center)
            }
            (
                Self::Arc {
                    center,
                    radius_mm,
                    start_angle_rad,
                    sweep_angle_rad,
                },
                PointRole::Start | PointRole::End,
            ) => {
                let angle = start_angle_rad
                    + if role == PointRole::End {
                        *sweep_angle_rad
                    } else {
                        0.
                    };
                Some(Point::new(
                    center.x + radius_mm * angle.cos(),
                    center.y + radius_mm * angle.sin(),
                ))
            }
            _ => None,
        }
    }
    pub fn line(&self) -> Option<(Point, Point)> {
        if let Self::Line { start, end } = self {
            Some((*start, *end))
        } else {
            None
        }
    }
    pub fn circle(&self) -> Option<(Point, f64)> {
        match self {
            Self::Circle { center, radius_mm }
            | Self::Arc {
                center, radius_mm, ..
            } => Some((*center, *radius_mm)),
            _ => None,
        }
    }
    pub fn parameters(&self) -> Vec<f64> {
        match self {
            Self::Point { position } => vec![position.x, position.y],
            Self::Line { start, end } => vec![start.x, start.y, end.x, end.y],
            Self::Circle { center, radius_mm } => vec![center.x, center.y, *radius_mm],
            Self::Arc {
                center,
                radius_mm,
                start_angle_rad,
                sweep_angle_rad,
            } => vec![
                center.x,
                center.y,
                *radius_mm,
                *start_angle_rad,
                *sweep_angle_rad,
            ],
        }
    }
}
impl SketchDocument {
    /// Connectivity is authored by ordered wires, independently of constraint
    /// annotations. Solvers must maintain these endpoint equalities.
    pub fn wire_joints(&self) -> Result<Vec<(PointRef, PointRef)>> {
        let mut joints = vec![];
        for wire in &self.wires {
            if wire.edges.len() == 1
                && matches!(
                    self.entity(&wire.edges[0].entity_id)?.geometry,
                    SketchGeometry::Circle { .. }
                )
            {
                continue;
            }
            for i in 0..wire.edges.len() {
                if i + 1 == wire.edges.len() && !wire.closed {
                    break;
                }
                let a = &wire.edges[i];
                let b = &wire.edges[(i + 1) % wire.edges.len()];
                let a = PointRef {
                    entity_id: a.entity_id.clone(),
                    point: if a.reversed {
                        PointRole::Start
                    } else {
                        PointRole::End
                    },
                };
                let b = PointRef {
                    entity_id: b.entity_id.clone(),
                    point: if b.reversed {
                        PointRole::End
                    } else {
                        PointRole::Start
                    },
                };
                self.point(&a)?;
                self.point(&b)?;
                joints.push((a, b));
            }
        }
        Ok(joints)
    }
    pub fn entity(&self, id: &str) -> Result<&Entity> {
        reference_entity(id)
            .or_else(|| self.entities.iter().find(|e| e.id == id))
            .ok_or_else(|| located("SKETCH_REFERENCE", id, "unknown sketch entity"))
    }
    pub fn point(&self, r: &PointRef) -> Result<Point> {
        self.entity(&r.entity_id)?
            .geometry
            .point(r.point)
            .ok_or_else(|| {
                located(
                    "SKETCH_OPERAND",
                    &r.entity_id,
                    "point role does not exist on this geometry",
                )
            })
    }
    pub fn line(&self, id: &str) -> Result<(Point, Point)> {
        self.entity(id)?
            .geometry
            .line()
            .ok_or_else(|| located("SKETCH_OPERAND", id, "expected a line"))
    }
    pub fn circle(&self, id: &str) -> Result<(Point, f64)> {
        self.entity(id)?
            .geometry
            .circle()
            .ok_or_else(|| located("SKETCH_OPERAND", id, "expected a circle or circular arc"))
    }
    /// Exact source intent, excluding annotations, constraints and construction-only geometry.
    /// Validity is checked separately before materialization and retained machining.
    pub fn geometry_digest(&self) -> Result<String> {
        let mut entities = self
            .entities
            .iter()
            .filter(|e| !e.construction && !matches!(e.geometry, SketchGeometry::Point { .. }))
            .map(|e| (&e.id, &e.geometry))
            .collect::<Vec<_>>();
        entities.sort_by_key(|e| e.0);
        let mut wires = self
            .wires
            .iter()
            .filter(|w| {
                w.edges
                    .iter()
                    .any(|edge| self.entity(&edge.entity_id).is_ok_and(|e| !e.construction))
            })
            .map(|w| (&w.id, &w.edges, w.closed))
            .collect::<Vec<_>>();
        wires.sort_by_key(|w| w.0);
        crate::plan_hash::hash(&(SKETCH_VERSION, &self.frame, entities, wires))
            .map_err(|e| error("SKETCH_JSON", e.to_string()))
    }
}
