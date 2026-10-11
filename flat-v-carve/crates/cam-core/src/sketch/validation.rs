use super::*;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConstraintEvaluation {
    pub constraint_id: String,
    pub residual_mm: f64,
    pub measured: Option<f64>,
    pub reference: bool,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ValidationReport {
    pub issues: Vec<Diagnostic>,
    pub constraints: Vec<ConstraintEvaluation>,
}
impl ValidationReport {
    pub fn valid(&self) -> bool {
        self.issues.is_empty()
    }
}
fn valid_id(id: &str) -> Result<()> {
    if id.len() <= 80 && crate::preview::valid_id(id) {
        Ok(())
    } else {
        Err(located(
            "SKETCH_ID",
            id,
            "expected a nonempty identifier of at most 80 ASCII letters, digits, underscores or hyphens",
        ))
    }
}
fn label(label: &Option<String>) -> Result<()> {
    if label.as_ref().is_some_and(|s| s.len() > 1000) {
        Err(error("SKETCH_LIMIT", "label exceeds 1000 bytes"))
    } else {
        Ok(())
    }
}
fn finite(g: &SketchGeometry) -> Result<()> {
    if g.parameters()
        .iter()
        .any(|v| !v.is_finite() || v.abs() > MAX_COORDINATE_MM)
    {
        Err(error(
            "SKETCH_PARAMETER",
            "geometry values must be finite and within the sketch numeric range",
        ))
    } else {
        Ok(())
    }
}
fn point_line(p: Point, a: Point, b: Point) -> f64 {
    ((p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)).abs() / a.distance(b)
}
fn angle(a: Point, b: Point, c: Point, d: Point) -> f64 {
    let (x, y, u, v) = (b.x - a.x, b.y - a.y, d.x - c.x, d.y - c.y);
    (x * v - y * u).atan2(x * u + y * v)
}
fn angle_delta(a: f64, b: f64) -> f64 {
    (a - b).sin().atan2((a - b).cos()).abs()
}
impl SketchDocument {
    /// Save/open gate. Unsatisfied constraints and incomplete wires remain saveable;
    /// malformed operand types, unknown IDs, non-finite values and excess budgets do not.
    pub fn validate_structure(&self) -> Result<()> {
        if self.sketch_version != SKETCH_VERSION {
            return Err(error(
                "SKETCH_VERSION",
                format!(
                    "unsupported sketch version {}; expected {SKETCH_VERSION}",
                    self.sketch_version
                ),
            ));
        }
        if self.entities.len() > MAX_ENTITIES
            || self.constraints.len() > MAX_CONSTRAINTS
            || self.wires.len() > MAX_WIRES
            || self.wires.iter().map(|w| w.edges.len()).sum::<usize>() > MAX_ENTITIES
        {
            return Err(error(
                "SKETCH_LIMIT",
                "sketch exceeds entity, constraint or wire budgets",
            ));
        }
        let f = &self.frame;
        if !f.min.finite()
            || !f.width_mm.is_finite()
            || !f.height_mm.is_finite()
            || f.width_mm <= 0.
            || f.height_mm <= 0.
            || [
                f.min.x,
                f.min.y,
                f.min.x + f.width_mm,
                f.min.y + f.height_mm,
            ]
            .iter()
            .any(|v| v.abs() > MAX_COORDINATE_MM)
        {
            return Err(error(
                "SKETCH_FRAME",
                "frame must be finite, positive and within the sketch numeric range",
            ));
        }
        let mut ids = BTreeSet::new();
        for e in &self.entities {
            valid_id(&e.id)?;
            if !ids.insert(&e.id) {
                return Err(located("SKETCH_DUPLICATE_ID", &e.id, "duplicate entity ID"));
            }
            label(&e.label)?;
            finite(&e.geometry)?;
        }
        ids.clear();
        for w in &self.wires {
            valid_id(&w.id)?;
            if !ids.insert(&w.id) {
                return Err(located("SKETCH_DUPLICATE_ID", &w.id, "duplicate wire ID"));
            }
            label(&w.label)?;
            for e in &w.edges {
                if matches!(
                    self.entity(&e.entity_id)?.geometry,
                    SketchGeometry::Point { .. }
                ) {
                    return Err(located(
                        "SKETCH_OPERAND",
                        &w.id,
                        "a point cannot be a wire edge",
                    ));
                }
            }
        }
        ids.clear();
        for c in &self.constraints {
            valid_id(&c.id)?;
            if !ids.insert(&c.id) {
                return Err(located(
                    "SKETCH_DUPLICATE_ID",
                    &c.id,
                    "duplicate constraint ID",
                ));
            }
            if c.reference && !c.definition.is_dimension() {
                return Err(located(
                    "SKETCH_CONSTRAINT",
                    &c.id,
                    "only dimensions can be reference constraints",
                ));
            }
            use ConstraintKind::*;
            match &c.definition {
                Coincident { a, b }
                | Distance { a, b, .. }
                | HorizontalDistance { a, b, .. }
                | VerticalDistance { a, b, .. } => {
                    self.point(a)?;
                    self.point(b)?;
                }
                Horizontal { line } | Vertical { line } | Length { line, .. } => {
                    self.line(line)?;
                }
                Parallel { a, b }
                | Perpendicular { a, b }
                | EqualLength { a, b }
                | Angle { a, b, .. } => {
                    self.line(a)?;
                    self.line(b)?;
                }
                EqualRadius { a, b } => {
                    self.circle(a)?;
                    self.circle(b)?;
                }
                Radius { curve, .. } | Diameter { curve, .. } => {
                    self.circle(curve)?;
                }
                Tangent { line, curve } => {
                    self.line(line)?;
                    self.circle(curve)?;
                }
                PointOnLine { point, line } | Midpoint { point, line } => {
                    self.point(point)?;
                    self.line(line)?;
                }
                PointOnCircle { point, curve } => {
                    self.point(point)?;
                    self.circle(curve)?;
                }
                Symmetry { a, b, line } => {
                    self.point(a)?;
                    self.point(b)?;
                    self.line(line)?;
                }
                Fixed { entity, target } => {
                    let current = &self.entity(entity)?.geometry;
                    if std::mem::discriminant(current) != std::mem::discriminant(target) {
                        return Err(located(
                            "SKETCH_OPERAND",
                            &c.id,
                            "fixed target must have the same geometry kind",
                        ));
                    }
                    finite(target)?;
                }
                FixedPoint { point, position } => {
                    self.point(point)?;
                    finite(&SketchGeometry::Point {
                        position: *position,
                    })?;
                }
            }
            let dimension = match c.definition {
                Distance { value_mm, .. }
                | Length { value_mm, .. }
                | Radius { value_mm, .. }
                | Diameter { value_mm, .. } => Some((value_mm, true)),
                HorizontalDistance { value_mm, .. } | VerticalDistance { value_mm, .. } => {
                    Some((value_mm, false))
                }
                Angle { value_rad, .. } => Some((value_rad, false)),
                _ => None,
            };
            if dimension.is_some_and(|(v, positive)| {
                !v.is_finite() || v.abs() > MAX_COORDINATE_MM || (positive && v < 0.)
            }) {
                return Err(located(
                    "SKETCH_PARAMETER",
                    &c.id,
                    "invalid dimension value",
                ));
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|e| error("SKETCH_JSON", e.to_string()))?;
        if bytes.len() > MAX_SKETCH_BYTES {
            return Err(error("SKETCH_LIMIT", "sketch exceeds 2 MB"));
        }
        Ok(())
    }
    /// Numeric checks independent of the solver's cost/status. Topology is checked
    /// by materialization using the existing strict Region normalization.
    pub fn validate_solution(&self) -> Result<ValidationReport> {
        self.validate_structure()?;
        let mut report = ValidationReport::default();
        for e in &self.entities {
            let bad = match &e.geometry {
                SketchGeometry::Line { start, end } => {
                    start.distance(*end) <= RESIDUAL_TOLERANCE_MM
                }
                SketchGeometry::Circle { radius_mm, .. } => *radius_mm <= RESIDUAL_TOLERANCE_MM,
                SketchGeometry::Arc {
                    radius_mm,
                    sweep_angle_rad,
                    ..
                } => {
                    *radius_mm <= RESIDUAL_TOLERANCE_MM
                        || sweep_angle_rad.abs() < 1e-9
                        || sweep_angle_rad.abs() >= std::f64::consts::TAU
                }
                SketchGeometry::Point { .. } => false,
            };
            if bad {
                report.issues.push(located(
                    "SKETCH_DEGENERATE",
                    &e.id,
                    "collapsed line/radius or invalid circular arc sweep",
                ));
            }
        }
        for c in &self.constraints {
            let (residual, measured) = self.evaluate(&c.definition)?;
            if !residual.is_finite() || measured.is_some_and(|x| !x.is_finite()) {
                report.issues.push(located(
                    "SKETCH_DEGENERATE",
                    &c.id,
                    "constraint cannot be evaluated on degenerate geometry",
                ));
                continue;
            }
            report.constraints.push(ConstraintEvaluation {
                constraint_id: c.id.clone(),
                residual_mm: if c.reference { 0. } else { residual.abs() },
                measured,
                reference: c.reference,
            });
            if !c.reference && residual.abs() > RESIDUAL_TOLERANCE_MM {
                report.issues.push(located(
                    "SKETCH_UNSATISFIED",
                    &c.id,
                    format!(
                        "constraint residual {} mm exceeds {} mm",
                        residual.abs(),
                        RESIDUAL_TOLERANCE_MM
                    ),
                ));
            }
        }
        Ok(report)
    }
    pub fn require_valid_solution(&self) -> Result<()> {
        let report = self.validate_solution()?;
        if let Some(issue) = report.issues.into_iter().next() {
            Err(issue)
        } else {
            Ok(())
        }
    }
    pub fn evaluate(&self, c: &ConstraintKind) -> Result<(f64, Option<f64>)> {
        use ConstraintKind::*;
        let (residual, measured) = match c {
            Coincident { a, b } => (self.point(a)?.distance(self.point(b)?), None),
            Horizontal { line } => {
                let (a, b) = self.line(line)?;
                (b.y - a.y, None)
            }
            Vertical { line } => {
                let (a, b) = self.line(line)?;
                (b.x - a.x, None)
            }
            Parallel { a, b } => {
                let (p, q, r, s) = self.lines(a, b)?;
                (
                    angle(p, q, r, s).sin() * p.distance(q).min(r.distance(s)),
                    None,
                )
            }
            Perpendicular { a, b } => {
                let (p, q, r, s) = self.lines(a, b)?;
                (
                    angle(p, q, r, s).cos() * p.distance(q).min(r.distance(s)),
                    None,
                )
            }
            EqualLength { a, b } => {
                let (a, b, c, d) = self.lines(a, b)?;
                (a.distance(b) - c.distance(d), None)
            }
            EqualRadius { a, b } => (self.circle(a)?.1 - self.circle(b)?.1, None),
            Tangent { line, curve } => {
                let (a, b) = self.line(line)?;
                let (p, r) = self.circle(curve)?;
                (point_line(p, a, b) - r, None)
            }
            PointOnLine { point, line } => {
                let (a, b) = self.line(line)?;
                (point_line(self.point(point)?, a, b), None)
            }
            PointOnCircle { point, curve } => {
                let (p, r) = self.circle(curve)?;
                (self.point(point)?.distance(p) - r, None)
            }
            Midpoint { point, line } => {
                let (a, b) = self.line(line)?;
                (self.point(point)?.distance(a.lerp(b, 0.5)), None)
            }
            Symmetry { a, b, line } => {
                let p = self.point(a)?;
                let q = self.point(b)?;
                let (a, b) = self.line(line)?;
                let (dx, dy) = (b.x - a.x, b.y - a.y);
                let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / (dx * dx + dy * dy);
                (
                    q.distance(Point::new(
                        2. * (a.x + t * dx) - p.x,
                        2. * (a.y + t * dy) - p.y,
                    )),
                    None,
                )
            }
            Distance { a, b, value_mm } => {
                let v = self.point(a)?.distance(self.point(b)?);
                (v - value_mm, Some(v))
            }
            HorizontalDistance { a, b, value_mm } => {
                let v = self.point(a)?.x - self.point(b)?.x;
                (v - value_mm, Some(v))
            }
            VerticalDistance { a, b, value_mm } => {
                let v = self.point(a)?.y - self.point(b)?.y;
                (v - value_mm, Some(v))
            }
            Length { line, value_mm } => {
                let (a, b) = self.line(line)?;
                let v = a.distance(b);
                (v - value_mm, Some(v))
            }
            Radius { curve, value_mm } => {
                let v = self.circle(curve)?.1;
                (v - value_mm, Some(v))
            }
            Diameter { curve, value_mm } => {
                let d = 2. * self.circle(curve)?.1;
                (d - value_mm, Some(d))
            }
            Angle { a, b, value_rad } => {
                let (a, b, c, d) = self.lines(a, b)?;
                let v = angle(a, b, c, d);
                (
                    angle_delta(v, *value_rad) * a.distance(b).min(c.distance(d)),
                    Some(v),
                )
            }
            FixedPoint { point, position } => (self.point(point)?.distance(*position), None),
            Fixed { entity, target } => {
                let actual = &self.entity(entity)?.geometry;
                let scale = target.circle().map_or(1., |(_, r)| r.abs().max(1.));
                let mut max: f64 = 0.;
                for (i, (a, b)) in actual
                    .parameters()
                    .iter()
                    .zip(target.parameters())
                    .enumerate()
                {
                    max = max.max(
                        (a - b).abs()
                            * if i >= 3 && matches!(target, SketchGeometry::Arc { .. }) {
                                scale
                            } else {
                                1.
                            },
                    );
                }
                (max, None)
            }
        };
        Ok((residual, measured))
    }
    fn lines(&self, a: &str, b: &str) -> Result<(Point, Point, Point, Point)> {
        let (p, q) = self.line(a)?;
        let (r, s) = self.line(b)?;
        Ok((p, q, r, s))
    }
}
