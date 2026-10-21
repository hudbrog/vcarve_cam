//! Explicit Arael solves, shared by native and wasm. No solver handles are saved.
//! Cancellation is checked between bounded LM stages. A caller that needs to
//! interrupt a running stage must terminate its worker.
use arael::{
    model::{CrossBlock, Param},
    refs::Ref,
    simple_lm::{LmConfig, LmSession, RootProblem, SparseFaer},
    vect::vect2d,
};
use arael_sketch_solver::*;
use cam_core::{
    geometry::{Diagnostic, Point as CamPoint, Result},
    sketch::{
        ConstraintKind as K, PointRef, PointRole, SketchDocument, SketchGeometry as Geometry,
        ValidationReport, reference_entities, reference_entity,
    },
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SOLVER_REVISION: &str = "arael-18d86e1fda91814a4a42d00001f3e8461c638bb4";
pub const MAX_ITERATIONS: usize = 208;
/// Temporary cursor preferences, never part of the saved constraint model.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DragPoint {
    pub point: PointRef,
    pub target: CamPoint,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    Accepted,
    Rejected,
    Cancelled,
}
#[derive(Clone, Debug, Serialize)]
pub struct SolveReport {
    pub status: SolveStatus,
    /// A diagnostic candidate, never automatically committed to the document.
    pub candidate: SketchDocument,
    pub validation: ValidationReport,
    pub degrees_of_freedom: Option<usize>,
    pub iterations: usize,
    pub solver_status: String,
    pub solver_revision: &'static str,
}
impl SolveReport {
    pub fn accepted(&self) -> bool {
        matches!(self.status, SolveStatus::Accepted)
    }
}
fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new("SKETCH_SOLVER", message).at_stage("sketch")
}
macro_rules! cross {($t:ident{$($f:ident:$v:expr),*$(,)?})=>{$t{$($f:$v,)*nid:0,cid:0,hb:CrossBlock::new()}};}
enum Handle {
    Point(Ref<Point>),
    Line(Ref<Line>),
    Arc(Ref<Arc>),
}
struct Model {
    sketch: Sketch,
    handles: BTreeMap<String, Handle>,
    points: BTreeMap<PointRef, Ref<Point>>,
}
impl Model {
    fn new(doc: &SketchDocument) -> Result<Self> {
        let mut m = Self {
            sketch: Sketch::new(),
            handles: BTreeMap::new(),
            points: BTreeMap::new(),
        };
        for e in doc.entities.iter().chain(reference_entities()) {
            // Conflicting fixed targets remain in the independent validation.
            let target = doc.constraints.iter().find_map(|c| match &c.definition {
                K::Fixed { entity, target } if entity == &e.id => Some(target),
                _ => None,
            });
            let g = target.unwrap_or(&e.geometry);
            let fixed = target.is_some() || reference_entity(&e.id).is_some();
            let h = match *g {
                Geometry::Point { position } => {
                    let p = vect2d::new(position.x, position.y);
                    let r = m.sketch.add_point(p);
                    if fixed {
                        m.sketch.points[r].pos = Param::fixed(p);
                    }
                    Handle::Point(r)
                }
                Geometry::Line { start, end } => {
                    let a = vect2d::new(start.x, start.y);
                    let b = vect2d::new(end.x, end.y);
                    let r = m.sketch.add_line(a, b);
                    if fixed {
                        m.sketch.lines[r].p1 = Param::fixed(a);
                        m.sketch.lines[r].p2 = Param::fixed(b);
                    }
                    Handle::Line(r)
                }
                Geometry::Circle { center, radius_mm }
                | Geometry::Arc {
                    center, radius_mm, ..
                } => {
                    let (sa, ea, circle) = if let Geometry::Arc {
                        start_angle_rad,
                        sweep_angle_rad,
                        ..
                    } = g
                    {
                        (*start_angle_rad, start_angle_rad + sweep_angle_rad, false)
                    } else {
                        (0., std::f64::consts::TAU, true)
                    };
                    let p = vect2d::new(center.x, center.y);
                    let r = m.sketch.add_arc(p, radius_mm, sa, ea, circle);
                    if fixed {
                        let a = &mut m.sketch.arcs[r];
                        a.center = Param::fixed(p);
                        a.radius = Param::fixed(radius_mm);
                        a.start_angle = Param::fixed(sa);
                        a.end_angle = Param::fixed(ea);
                    }
                    Handle::Arc(r)
                }
            };
            m.handles.insert(e.id.clone(), h);
        }
        for c in &doc.constraints {
            if !c.reference {
                m.add(doc, &c.definition)?;
            }
        }
        for (a, b) in doc.wire_joints()? {
            m.add(doc, &K::Coincident { a, b })?;
        }
        Ok(m)
    }
    fn line(&self, id: &str) -> Ref<Line> {
        match self.handles[id] {
            Handle::Line(r) => r,
            _ => unreachable!("validated operand"),
        }
    }
    fn arc(&self, id: &str) -> Ref<Arc> {
        match self.handles[id] {
            Handle::Arc(r) => r,
            _ => unreachable!("validated operand"),
        }
    }
    fn point(&mut self, doc: &SketchDocument, r: &PointRef) -> Result<Ref<Point>> {
        if let Some(p) = self.points.get(r) {
            return Ok(*p);
        }
        if let Handle::Point(p) = self.handles[&r.entity_id] {
            return Ok(p);
        }
        let p = doc.point(r)?;
        let point = self.sketch.add_point(vect2d::new(p.x, p.y));
        if reference_entity(&r.entity_id).is_some() {
            self.sketch.points[point].pos = Param::fixed(vect2d::new(p.x, p.y));
            self.points.insert(r.clone(), point);
            return Ok(point);
        }
        match self.handles[&r.entity_id] {
            Handle::Line(line) => match r.point {
                PointRole::Start => self.sketch.coincident_lp1.push(cross!(CoincidentLP1 {
                    line: line,
                    point: point
                })),
                PointRole::End => self.sketch.coincident_lp2.push(cross!(CoincidentLP2 {
                    line: line,
                    point: point
                })),
                _ => unreachable!("validated role"),
            },
            Handle::Arc(arc) => match r.point {
                PointRole::Center => {
                    self.sketch
                        .coincident_arc_center
                        .push(cross!(CoincidentArcCenter {
                            point: point,
                            arc: arc
                        }))
                }
                PointRole::Start => {
                    self.sketch
                        .coincident_arc_start
                        .push(cross!(CoincidentArcStart {
                            point: point,
                            arc: arc
                        }))
                }
                PointRole::End => self
                    .sketch
                    .coincident_arc_end
                    .push(cross!(CoincidentArcEnd {
                        point: point,
                        arc: arc
                    })),
                _ => unreachable!("validated role"),
            },
            _ => unreachable!("handled point"),
        }
        self.points.insert(r.clone(), point);
        Ok(point)
    }
    fn add(&mut self, doc: &SketchDocument, c: &K) -> Result<()> {
        match c {
            K::Fixed { .. } => {}
            K::FixedPoint { point, position } => {
                let p = self.point(doc, point)?;
                self.sketch.points[p].pos = Param::fixed(vect2d::new(position.x, position.y));
            }
            K::Horizontal { line } => {
                let r = self.line(line);
                self.sketch.lines[r].constraints.horizontal = true;
            }
            K::Vertical { line } => {
                let r = self.line(line);
                self.sketch.lines[r].constraints.vertical = true;
            }
            K::Length { line, value_mm } => {
                let a = self.point(
                    doc,
                    &PointRef {
                        entity_id: line.clone(),
                        point: PointRole::Start,
                    },
                )?;
                let b = self.point(
                    doc,
                    &PointRef {
                        entity_id: line.clone(),
                        point: PointRole::End,
                    },
                )?;
                self.sketch.distance_pp.push(cross!(DistancePP {
                    a: a,
                    b: b,
                    distance: *value_mm
                }));
            }
            K::Radius { curve, value_mm } | K::Diameter { curve, value_mm } => {
                let radius = if matches!(c, K::Diameter { .. }) {
                    *value_mm / 2.
                } else {
                    *value_mm
                };
                // Separate equations preserve redundant/conflicting dimensions.
                let a = self.arc(curve);
                let b = self.sketch.add_arc(
                    vect2d::new(0., 0.),
                    radius,
                    0.,
                    std::f64::consts::TAU,
                    true,
                );
                self.sketch.arcs[b].center = Param::fixed(vect2d::new(0., 0.));
                self.sketch.arcs[b].radius = Param::fixed(radius);
                self.sketch
                    .equal_radius
                    .push(cross!(EqualRadius { a: a, b: b }));
            }
            K::Coincident { a, b }
            | K::Distance { a, b, .. }
            | K::HorizontalDistance { a, b, .. }
            | K::VerticalDistance { a, b, .. } => {
                let a = self.point(doc, a)?;
                let b = self.point(doc, b)?;
                match c {
                    K::Coincident { .. } => {
                        self.sketch.hdistance_pp.push(cross!(HorizontalDistancePP {
                            a: a,
                            b: b,
                            distance: 0.
                        }));
                        self.sketch.vdistance_pp.push(cross!(VerticalDistancePP {
                            a: a,
                            b: b,
                            distance: 0.
                        }));
                    }
                    K::Distance { value_mm, .. } => {
                        self.sketch.distance_pp.push(cross!(DistancePP {
                            a: a,
                            b: b,
                            distance: *value_mm
                        }))
                    }
                    K::HorizontalDistance { value_mm, .. } => {
                        self.sketch.hdistance_pp.push(cross!(HorizontalDistancePP {
                            a: a,
                            b: b,
                            distance: *value_mm
                        }))
                    }
                    K::VerticalDistance { value_mm, .. } => {
                        self.sketch.vdistance_pp.push(cross!(VerticalDistancePP {
                            a: a,
                            b: b,
                            distance: *value_mm
                        }))
                    }
                    _ => unreachable!(),
                }
            }
            K::Parallel { a, b }
            | K::Perpendicular { a, b }
            | K::EqualLength { a, b }
            | K::Angle { a, b, .. } => {
                let a = self.line(a);
                let b = self.line(b);
                match c {
                    K::Parallel { .. } => {
                        self.sketch.parallel.push(cross!(Parallel { a: a, b: b }))
                    }
                    K::Perpendicular { .. } => {
                        self.sketch.perpendicular.push(cross!(Perpendicular {
                            a: a,
                            b: b,
                            dir_sign: f64::NAN
                        }))
                    }
                    K::EqualLength { .. } => self
                        .sketch
                        .equal_length
                        .push(cross!(EqualLength { a: a, b: b })),
                    K::Angle { value_rad, .. } => self.sketch.angle.push(cross!(AngleConstraint {
                        a: a,
                        b: b,
                        angle: value_rad.sin().atan2(value_rad.cos())
                    })),
                    _ => unreachable!(),
                }
            }
            K::EqualRadius { a, b } => self.sketch.equal_radius.push(cross!(EqualRadius {
                a: self.arc(a),
                b: self.arc(b)
            })),
            K::Tangent { line, curve } => self.sketch.tangent_la.push(cross!(TangentLA {
                line: self.line(line),
                arc: self.arc(curve),
                sign: 1.,
                p1_arc_start: false,
                p1_arc_end: false,
                p2_arc_start: false,
                p2_arc_end: false,
                dir_sign: f64::NAN
            })),
            K::PointOnLine { point, line } | K::Midpoint { point, line } => {
                let point = self.point(doc, point)?;
                let line = self.line(line);
                if matches!(c, K::PointOnLine { .. }) {
                    self.sketch.point_on_line.push(cross!(PointOnLine {
                        point: point,
                        line: line
                    }));
                } else {
                    self.sketch.midpoint.push(cross!(MidpointConstraint {
                        point: point,
                        line: line
                    }));
                }
            }
            K::PointOnCircle { point, curve } => {
                let point = self.point(doc, point)?;
                self.sketch.point_on_arc.push(cross!(PointOnArc {
                    point: point,
                    arc: self.arc(curve)
                }));
            }
            K::Symmetry { a, b, line } => {
                let a = self.point(doc, a)?;
                let c = self.point(doc, b)?;
                let line = self.line(line);
                self.sketch.symmetry_pp.push(SymmetryPP {
                    a,
                    c,
                    line,
                    nid: 0,
                    cid: 0,
                    hb_ac: CrossBlock::new(),
                    hb_al: CrossBlock::new(),
                    hb_cl: CrossBlock::new(),
                });
            }
        }
        Ok(())
    }
    fn candidate(&self, doc: &SketchDocument) -> SketchDocument {
        let mut result = doc.clone();
        for e in &mut result.entities {
            e.geometry = match self.handles[&e.id] {
                Handle::Point(r) => {
                    let p = self.sketch.points[r].pos.value;
                    Geometry::Point {
                        position: CamPoint::new(p.x, p.y),
                    }
                }
                Handle::Line(r) => {
                    let l = &self.sketch.lines[r];
                    Geometry::Line {
                        start: CamPoint::new(l.p1.value.x, l.p1.value.y),
                        end: CamPoint::new(l.p2.value.x, l.p2.value.y),
                    }
                }
                Handle::Arc(r) => {
                    let a = &self.sketch.arcs[r];
                    let center = CamPoint::new(a.center.value.x, a.center.value.y);
                    let radius_mm = a.radius.value;
                    if matches!(e.geometry, Geometry::Circle { .. }) {
                        Geometry::Circle { center, radius_mm }
                    } else {
                        Geometry::Arc {
                            center,
                            radius_mm,
                            start_angle_rad: a.start_angle.value,
                            sweep_angle_rad: a.end_angle.value - a.start_angle.value,
                        }
                    }
                }
            };
        }
        result
    }
}

pub fn solve(doc: &SketchDocument) -> Result<SolveReport> {
    solve_with_cancel(doc, || false)
}

/// Bounded JSON transport for a headless worker or command-line client.
pub fn solve_json(input: &str) -> Result<String> {
    if input.len() > cam_core::sketch::MAX_SKETCH_BYTES {
        return Err(error("sketch exceeds 2 MB"));
    }
    let doc =
        serde_json::from_str(input).map_err(|e| error(format!("invalid sketch JSON: {e}")))?;
    serde_json::to_string(&solve(&doc)?).map_err(|e| error(e.to_string()))
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(js_name = solveSketch)]
pub fn solve_wasm(input: &str) -> std::result::Result<String, wasm_bindgen::JsValue> {
    solve_json(input).map_err(|e| wasm_bindgen::JsValue::from_str(&e.to_string()))
}

/// Three graduated stages and a short precision refinement. Independent
/// millimetre residuals, not Arael's weighted cost/status, decide acceptance.
pub fn solve_with_cancel(
    doc: &SketchDocument,
    cancelled: impl FnMut() -> bool,
) -> Result<SolveReport> {
    solve_drag_with_cancel(doc, &[], cancelled)
}

pub fn solve_drag_with_cancel(
    doc: &SketchDocument,
    targets: &[DragPoint],
    mut cancelled: impl FnMut() -> bool,
) -> Result<SolveReport> {
    doc.validate_structure()?;
    if targets.len() > cam_core::sketch::MAX_ENTITIES * 3 {
        return Err(error("too many drag targets"));
    }
    for t in targets {
        if reference_entity(&t.point.entity_id).is_some() {
            return Err(error("origin and axes cannot be dragged"));
        }
        doc.point(&t.point)?;
        if [t.target.x, t.target.y]
            .iter()
            .any(|x| !x.is_finite() || x.abs() > cam_core::sketch::MAX_COORDINATE_MM)
        {
            return Err(error("invalid drag target"));
        }
    }
    let cancelled_report = |candidate: SketchDocument, iterations| -> Result<SolveReport> {
        Ok(SolveReport {
            validation: candidate.validate_solution()?,
            candidate,
            status: SolveStatus::Cancelled,
            degrees_of_freedom: None,
            iterations,
            solver_status: "cancelled".into(),
            solver_revision: SOLVER_REVISION,
        })
    };
    if cancelled() {
        return cancelled_report(doc.clone(), 0);
    }
    let mut m = Model::new(doc)?;
    let mut helpers = vec![];
    for t in targets {
        let actual = m.point(doc, &t.point)?;
        let helper = m
            .sketch
            .add_helper_point(vect2d::new(t.target.x, t.target.y));
        m.sketch.points[helper].drag_pull = 1.;
        m.sketch.coincident_pp.push(cross!(CoincidentPP {
            a: actual,
            b: helper
        }));
        helpers.push((helper, t.target));
    }
    m.sketch.prepare_derived();
    let strength = m.sketch.constraint_isigma;
    let mut session = LmSession::new(SparseFaer::new());
    let mut iterations = 0;
    let mut status = String::from("unchanged");
    for (scale, max_iters) in [(0.01, 64), (0.1, 64), (1., 64), (1., 16)] {
        if cancelled() {
            return cancelled_report(m.candidate(doc), iterations);
        }
        m.sketch.constraint_isigma = strength * scale;
        for (helper, target) in &helpers {
            // Final projection removes the cursor penalty, so an infeasible
            // cursor cannot trade driving-constraint accuracy for proximity.
            m.sketch.points[*helper].drag_pull = if max_iters == 16 { 0. } else { 1. };
            if max_iters != 16 {
                m.sketch.points[*helper].pos.value = vect2d::new(target.x, target.y);
            }
        }
        let mut params = Vec::new();
        RootProblem::serialize(&mut m.sketch, &mut params);
        if params.is_empty() {
            break;
        }
        let config = LmConfig {
            max_iters,
            initial_lambda: 1e-6,
            abs_precision: 0.,
            rel_precision: 0.,
            cost_threshold: if max_iters == 16 {
                0.
            } else {
                params.len() as f64 * 1e-8 / (scale * scale)
            },
            ..Default::default()
        };
        match session.solve_x0(&params, &mut m.sketch, &config) {
            Ok(r) => {
                iterations += r.iterations;
                status = format!("{:?}", r.status);
                RootProblem::deserialize(&mut m.sketch, &r.x);
            }
            Err(e) => {
                status = format!("failed: {e}");
                break;
            }
        }
    }
    m.sketch.constraint_isigma = strength;
    let candidate = m.candidate(doc);
    if cancelled() {
        return cancelled_report(candidate, iterations);
    }
    let mut validation = match candidate.validate_solution() {
        Ok(v) => v,
        Err(e) => ValidationReport {
            issues: vec![e],
            constraints: vec![],
        },
    };
    for (old, new) in doc.entities.iter().zip(&candidate.entities) {
        if let (
            Geometry::Arc {
                sweep_angle_rad: a, ..
            },
            Geometry::Arc {
                sweep_angle_rad: b, ..
            },
        ) = (&old.geometry, &new.geometry)
            && a.signum() != b.signum()
        {
            validation
                .issues
                .push(error(format!("arc '{}' changed orientation", old.id)));
        }
    }
    for (a, b) in doc.wire_joints()? {
        if candidate.point(&a)?.distance(candidate.point(&b)?)
            > cam_core::sketch::RESIDUAL_TOLERANCE_MM
        {
            let mut issue = error("wire endpoints did not remain connected");
            issue.source_id = Some(a.entity_id);
            validation.issues.push(issue);
        }
    }
    let degrees_of_freedom = if validation.valid() {
        m.sketch.dof().ok()
    } else {
        None
    };
    Ok(SolveReport {
        status: if validation.valid() {
            SolveStatus::Accepted
        } else {
            SolveStatus::Rejected
        },
        candidate,
        validation,
        degrees_of_freedom,
        iterations,
        solver_status: status,
        solver_revision: SOLVER_REVISION,
    })
}
