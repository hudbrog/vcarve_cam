//! Isolated qualification protocol, not the saved job model.
use arael::simple_lm::RootProblem;
use arael::{
    model::{CrossBlock, Param},
    refs::Ref,
    vect::vect2d,
};
use arael_sketch_solver::*;
use serde::{Deserialize, Serialize};
use web_time::Instant;
pub mod geometry;

#[derive(Clone, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub kind: String,
    pub values: Vec<f64>,
    #[serde(default)]
    pub fixed: Vec<bool>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Constraint {
    pub id: String,
    pub kind: String,
    pub a: usize,
    #[serde(default)]
    pub b: usize,
    #[serde(default)]
    pub c: usize,
    #[serde(default)]
    pub value: f64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Fixture {
    pub id: String,
    pub entities: Vec<Entity>,
    pub constraints: Vec<Constraint>,
    #[serde(default)]
    pub samples: usize,
    #[serde(default)]
    pub warm_constraint: Option<usize>,
    #[serde(default)]
    pub warm_step: f64,
    #[serde(default)]
    pub drift_isigma: Option<f64>,
    #[serde(default)]
    pub drag: Vec<[f64; 2]>,
    #[serde(default)]
    pub skip_polish: bool,
}
macro_rules! cross {
    ($t:ident { $($field:ident : $value:expr),* $(,)? }) => {
        $t { $($field: $value,)* nid: 0, cid: 0, hb: CrossBlock::new() }
    };
}
struct Model {
    sketch: Sketch,
    points: Vec<Option<Ref<Point>>>,
    lines: Vec<Option<Ref<Line>>>,
    arcs: Vec<Option<Ref<Arc>>>,
}
impl Model {
    fn new(f: &Fixture) -> Result<Self, String> {
        let mut m = Self {
            sketch: Sketch::new(),
            points: vec![None; f.entities.len()],
            lines: vec![None; f.entities.len()],
            arcs: vec![None; f.entities.len()],
        };
        if let Some(weight) = f.drift_isigma {
            m.sketch.drift_isigma = weight;
        }
        for (i, e) in f.entities.iter().enumerate() {
            let v = &e.values;
            let fixed = |n| e.fixed.get(n).copied().unwrap_or(false);
            match e.kind.as_str() {
                "point" => {
                    let r = m.sketch.add_point(vect2d::new(v[0], v[1]));
                    if fixed(0) && fixed(1) {
                        m.sketch.points[r].pos = Param::fixed(vect2d::new(v[0], v[1]));
                    }
                    m.points[i] = Some(r);
                }
                "line" => {
                    let r = m
                        .sketch
                        .add_line(vect2d::new(v[0], v[1]), vect2d::new(v[2], v[3]));
                    if fixed(0) && fixed(1) {
                        m.sketch.lines[r].p1 = Param::fixed(vect2d::new(v[0], v[1]));
                    }
                    if fixed(2) && fixed(3) {
                        m.sketch.lines[r].p2 = Param::fixed(vect2d::new(v[2], v[3]));
                    }
                    m.lines[i] = Some(r);
                }
                "circle" | "arc" => {
                    let closed = e.kind == "circle";
                    let r = m.sketch.add_arc(
                        vect2d::new(v[0], v[1]),
                        v[2],
                        if closed { 0. } else { v[3] },
                        if closed { std::f64::consts::TAU } else { v[4] },
                        closed,
                    );
                    if fixed(0) && fixed(1) {
                        m.sketch.arcs[r].center = Param::fixed(vect2d::new(v[0], v[1]));
                    }
                    if fixed(2) {
                        m.sketch.arcs[r].radius = Param::fixed(v[2]);
                    }
                    if !closed && fixed(3) {
                        m.sketch.arcs[r].start_angle = Param::fixed(v[3]);
                    }
                    if !closed && fixed(4) {
                        m.sketch.arcs[r].end_angle = Param::fixed(v[4]);
                    }
                    m.arcs[i] = Some(r);
                }
                kind => return Err(format!("Unsupported entity {kind}")),
            }
        }
        for c in &f.constraints {
            m.add(c)?;
        }
        Ok(m)
    }
    fn add(&mut self, c: &Constraint) -> Result<(), String> {
        let s = &mut self.sketch;
        let l = |i: usize| self.lines[i].expect("line operand");
        let p = |i: usize| self.points[i].expect("point operand");
        let a = |i: usize| self.arcs[i].expect("arc operand");
        match c.kind.as_str() {
            "horizontal" => s.lines[l(c.a)].constraints.horizontal = true,
            "vertical" => s.lines[l(c.a)].constraints.vertical = true,
            "length" => {
                let x = &mut s.lines[l(c.a)].constraints;
                x.has_length = true;
                x.length = c.value;
            }
            "radius" => {
                let x = &mut s.arcs[a(c.a)].constraints;
                x.has_target_radius = true;
                x.target_radius = c.value;
            }
            "join" => s.coincident_ll21.push(cross!(CoincidentLL21 {
                a: l(c.a),
                b: l(c.b)
            })),
            "parallel" => s.parallel.push(cross!(Parallel {
                a: l(c.a),
                b: l(c.b)
            })),
            "angle" => s.angle.push(cross!(AngleConstraint {
                a: l(c.a),
                b: l(c.b),
                angle: c.value
            })),
            "symmetry" => s.symmetry_pp.push(SymmetryPP {
                a: p(c.a),
                c: p(c.b),
                line: l(c.c),
                nid: 0,
                cid: 0,
                hb_ac: CrossBlock::new(),
                hb_al: CrossBlock::new(),
                hb_cl: CrossBlock::new(),
            }),
            "perpendicular" => s.perpendicular.push(cross!(Perpendicular {
                a: l(c.a),
                b: l(c.b),
                dir_sign: f64::NAN
            })),
            "equal_length" => s.equal_length.push(cross!(EqualLength {
                a: l(c.a),
                b: l(c.b)
            })),
            "equal_radius" => s.equal_radius.push(cross!(EqualRadius {
                a: a(c.a),
                b: a(c.b)
            })),
            "tangent" => s.tangent_la.push(cross!(TangentLA {
                line: l(c.a),
                arc: a(c.b),
                sign: 1.,
                p1_arc_start: false,
                p1_arc_end: false,
                p2_arc_start: false,
                p2_arc_end: false,
                dir_sign: f64::NAN
            })),
            "distance" => s.distance_pp.push(cross!(DistancePP {
                a: p(c.a),
                b: p(c.b),
                distance: c.value
            })),
            "hdistance" => s.hdistance_pp.push(cross!(HorizontalDistancePP {
                a: p(c.a),
                b: p(c.b),
                distance: c.value
            })),
            "vdistance" => s.vdistance_pp.push(cross!(VerticalDistancePP {
                a: p(c.a),
                b: p(c.b),
                distance: c.value
            })),
            "coincident" => s.coincident_pp.push(cross!(CoincidentPP {
                a: p(c.a),
                b: p(c.b)
            })),
            "point_on_line" => s.point_on_line.push(cross!(PointOnLine {
                point: p(c.a),
                line: l(c.b)
            })),
            "midpoint" => s.midpoint.push(cross!(MidpointConstraint {
                point: p(c.a),
                line: l(c.b)
            })),
            "point_on_circle" => s.point_on_arc.push(cross!(PointOnArc {
                point: p(c.a),
                arc: a(c.b)
            })),
            _ => return Err(format!("Unsupported constraint {}", c.kind)),
        }
        Ok(())
    }
    fn values(&self, f: &Fixture) -> Vec<Vec<f64>> {
        f.entities
            .iter()
            .enumerate()
            .map(|(i, e)| match e.kind.as_str() {
                "point" => {
                    let p = self.sketch.points[self.points[i].unwrap()].pos.value;
                    vec![p.x, p.y]
                }
                "line" => {
                    let l = &self.sketch.lines[self.lines[i].unwrap()];
                    vec![l.p1.value.x, l.p1.value.y, l.p2.value.x, l.p2.value.y]
                }
                _ => {
                    let a = &self.sketch.arcs[self.arcs[i].unwrap()];
                    let mut v = vec![a.center.value.x, a.center.value.y, a.radius.value];
                    if e.kind == "arc" {
                        v.extend([a.start_angle.value, a.end_angle.value]);
                    }
                    v
                }
            })
            .collect()
    }
    fn update_dimension(&mut self, c: &Constraint, value: f64) {
        match c.kind.as_str() {
            "length" => {
                self.sketch.lines[self.lines[c.a].unwrap()]
                    .constraints
                    .length = value
            }
            "radius" => {
                self.sketch.arcs[self.arcs[c.a].unwrap()]
                    .constraints
                    .target_radius = value
            }
            _ => panic!("unsupported warm dimension"),
        }
    }
}
pub fn run_fixture(f: &Fixture) -> Result<serde_json::Value, String> {
    let begin = Instant::now();
    let mut m = Model::new(f)?;
    let build_ms = begin.elapsed().as_secs_f64() * 1000.;
    let begin = Instant::now();
    let initial = m.sketch.solve();
    let initial_values = m.values(f);
    // One bounded warm refinement. Independent residuals still decide acceptance.
    let result = if f.skip_polish {
        initial
    } else {
        m.sketch.prepare_derived();
        let mut params = Vec::new();
        RootProblem::serialize(&mut m.sketch, &mut params);
        let config = arael::simple_lm::LmConfig {
            max_iters: 16,
            cost_threshold: 0.,
            initial_lambda: 1e-6,
            ..Default::default()
        };
        match arael::simple_lm::lm_solve(
            &params,
            &mut arael::simple_lm::SparseFaer::new(),
            &mut m.sketch,
            &config,
        ) {
            Ok(r) => {
                m.sketch.deserialize(&r.x);
                r
            }
            Err(_) => initial,
        }
    };
    let cold_ms = begin.elapsed().as_secs_f64() * 1000.;
    let values = m.values(f);
    let begin = Instant::now();
    let dof = m.sketch.dof();
    let diagnostic_ms = begin.elapsed().as_secs_f64() * 1000.;
    let mut warm = Vec::new();
    let mut warm_values = Vec::new();
    if let Some(ci) = f.warm_constraint {
        let mut session = arael::simple_lm::LmSession::new(arael::simple_lm::SparseFaer::new());
        m.sketch.prepare_derived();
        for n in 0..f.samples.max(1) {
            let v = f.constraints[ci].value + f.warm_step * (n + 1) as f64;
            m.update_dimension(&f.constraints[ci], v);
            let begin = Instant::now();
            let r = m.sketch.solve_prepared_with(&mut session);
            warm.push(begin.elapsed().as_secs_f64() * 1000.);
            warm_values.push(serde_json::json!({"value":v,"values":m.values(f),"status":format!("{:?}",r.status)}));
        }
    }
    let mut drag = Vec::new();
    if let Some(first) = f.drag.first() {
        let app = m.sketch.install_drag(
            DragTarget::LineP2(m.lines[0].unwrap()),
            vect2d::new(first[0], first[1]),
            None,
            Some(0.01),
        );
        let mut session = arael::simple_lm::LmSession::new(arael::simple_lm::SparseFaer::new());
        m.sketch.prepare_derived();
        for cursor in &f.drag {
            m.sketch
                .move_drag_helper(app.helper, vect2d::new(cursor[0], cursor[1]));
            let start = Instant::now();
            let r = m.sketch.solve_prepared_with(&mut session);
            drag.push(serde_json::json!({"cursor":cursor,"ms":start.elapsed().as_secs_f64()*1000.,"values":m.values(f),"status":format!("{:?}",r.status)}));
        }
        m.sketch.remove_drag(&app);
    }
    let released_dof = if drag.is_empty() {
        None
    } else {
        m.sketch.dof().ok()
    };
    Ok(
        serde_json::json!({"id":f.id,"backend":"arael","status":format!("{:?}",result.status),"iterations":result.iterations,"cost":result.end_cost,"initial_values":initial_values,"values":values,"dof":dof.as_ref().ok(),"dof_error":dof.err(),"build_ms":build_ms,"cold_ms":cold_ms,"diagnostic_ms":diagnostic_ms,"warm_ms":warm,"warm":warm_values,"drag":drag,"released_dof":released_dof}),
    )
}
pub fn run_json(input: &str) -> Result<String, String> {
    let fixtures: Vec<Fixture> = serde_json::from_str(input).map_err(|e| e.to_string())?;
    let results: Result<Vec<_>, _> = fixtures.iter().map(run_fixture).collect();
    serde_json::to_string(&results?).map_err(|e| e.to_string())
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run(input: &str) -> Result<String, wasm_bindgen::JsValue> {
    run_json(input).map_err(|e| wasm_bindgen::JsValue::from_str(&e))
}
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn geometry_probe() -> String {
    geometry::report()
}
