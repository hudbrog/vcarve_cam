//! Revision-gated transactions around the disposable Arael worker.
use super::*;
use cam_sketch_solver::DragPoint;

pub(super) struct Flight {
    request: worker::Request,
    timer: crate::clock::Timer,
}
pub(super) struct Drag {
    pub anchor: Point,
    pub before: SketchDocument,
    pub subjects: Vec<PointRef>,
    pub released: bool,
    last: Option<Point>,
}
pub(super) struct Solver {
    port: worker::Port,
    revision: u64,
    serial: u64,
    flight: Option<Flight>,
    dirty: bool,
    targets: Vec<DragPoint>,
    manual_before: Option<SketchDocument>,
    pub outcome: Option<worker::Outcome>,
    pub status: String,
}
impl Default for Solver {
    fn default() -> Self {
        Self {
            port: Default::default(),
            revision: 0,
            serial: 0,
            flight: None,
            dirty: true,
            targets: vec![],
            manual_before: None,
            outcome: None,
            status: "Awaiting solve".into(),
        }
    }
}
impl Editor {
    pub fn solving(&self) -> bool {
        self.solver.flight.is_some() || self.solver.dirty
    }
    pub fn solver_status(&self) -> &str {
        &self.solver.status
    }
    pub(super) fn invalidate_solve(&mut self) {
        if self.solver.flight.is_some() {
            self.solver.port.cancel();
        }
        self.solver.revision += 1;
        self.solver.flight = None;
        self.solver.targets.clear();
        self.solver.manual_before = None;
        self.solver.dirty = true;
        self.solver.outcome = None;
        self.solver.status = "Awaiting solve".into();
    }
    pub fn request_solve(&mut self) {
        self.cancel_gesture();
        self.invalidate_solve();
        self.solver.manual_before = Some(self.saved.model.clone());
    }
    pub fn cancel_solve(&mut self) {
        self.cancel_gesture();
        self.invalidate_solve();
        self.solver.dirty = false;
        self.solver.status = "Solve cancelled · geometry retained".into();
    }
    pub fn poll_solver(&mut self, ctx: &egui::Context) {
        while let Some(reply) = self.solver.port.poll() {
            self.receive_solve(reply);
        }
        if self
            .solver
            .flight
            .as_ref()
            .is_some_and(|f| f.timer.elapsed_ms() > worker::TIMEOUT_MS)
        {
            self.solver.port.cancel();
            self.solver.flight = None;
            self.solver.dirty = false;
            self.solver.status = "Solver timed out · geometry retained · retry Solve".into();
        }
        if self.solver.flight.is_none() && self.solver.dirty {
            self.solver.dirty = false;
            self.solver.serial += 1;
            let request = worker::Request {
                id: self.solver.serial,
                revision: self.solver.revision,
                model: self.saved.model.clone(),
                targets: self.solver.targets.clone(),
            };
            match self.solver.port.start(request.clone(), ctx.clone()) {
                Ok(()) => {
                    self.solver.flight = Some(Flight {
                        request,
                        timer: crate::clock::Timer::start(),
                    });
                    self.solver.status = "Solving…".into();
                }
                Err(e) => {
                    self.solver.port.cancel();
                    self.solver.status = e;
                }
            }
        }
        // Test transport is synchronous; production replies arrive on later frames.
        while let Some(reply) = self.solver.port.poll() {
            self.receive_solve(reply);
        }
        if self.solving() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        } else if self.drag.as_ref().is_some_and(|d| d.released) {
            let drag = self.drag.take().unwrap();
            self.record_history(drag.before);
        }
    }
    fn receive_solve(&mut self, reply: worker::Reply) {
        let Some(flight) = &self.solver.flight else {
            return;
        };
        if reply.id != flight.request.id || reply.revision != self.solver.revision {
            return;
        }
        let flight = self.solver.flight.take().unwrap();
        // A newer cursor target supersedes this result, including its diagnostics.
        if self.solver.dirty {
            return;
        }
        match reply.result {
            Err(e) => {
                self.solver.port.cancel();
                self.solver.status = e;
            }
            Ok(mut out) => {
                if out.accepted && !acceptable(&flight.request.model, &out.candidate) {
                    out.accepted = false;
                    out.issues.push(worker::Issue {
                        code: "invalid_candidate".into(),
                        source_id: None,
                        message:
                            "Solver candidate failed independent validation; geometry retained"
                                .into(),
                    });
                }
                if out.accepted {
                    // Avoid numerical noise changing an already satisfied draft on open.
                    let stable = acceptable(&flight.request.model, &flight.request.model)
                        && geometry_delta(&flight.request.model, &out.candidate) < 1e-7;
                    if !stable {
                        self.saved.model = out.candidate.clone();
                    }
                    if let Some(before) = self.solver.manual_before.take() {
                        self.record_history(before);
                    }
                    self.solver.status = match out.degrees_of_freedom {
                        Some(0) => "Fully constrained · 0 DOF".into(),
                        Some(n) => format!("Underconstrained · {n} DOF"),
                        None => "Solved · DOF unavailable".into(),
                    };
                    if out.drag_error_mm.is_some_and(|d| d > 1e-4) {
                        self.solver
                            .status
                            .push_str(" · Drag limited by constraints");
                    }
                } else {
                    self.solver.status = "Solve rejected · geometry retained".into();
                    self.solver.manual_before = None;
                }
                self.solver.outcome = Some(out);
            }
        }
    }
    pub(super) fn record_history(&mut self, before: SketchDocument) {
        if before != self.saved.model {
            self.undo.push(before);
            Self::trim(&mut self.undo);
            self.redo.clear();
        }
    }
    pub(super) fn start_drag(&mut self, anchor: Point, subjects: Vec<PointRef>) {
        self.cancel_gesture();
        let subjects: Vec<_> = subjects
            .into_iter()
            .filter(|p| cam_core::sketch::reference_entity(&p.entity_id).is_none())
            .collect();
        if subjects.is_empty() {
            self.message = "Origin and axes are fixed construction geometry.".into();
            return;
        }
        self.invalidate_solve();
        self.solver.dirty = false;
        self.drag = Some(Drag {
            anchor,
            before: self.saved.model.clone(),
            subjects,
            released: false,
            last: None,
        });
    }
    pub(super) fn drag_to(&mut self, target: Point) {
        let Some(drag) = &mut self.drag else { return };
        if drag.last == Some(target) {
            return;
        }
        drag.last = Some(target);
        let dx = target.x - drag.anchor.x;
        let dy = target.y - drag.anchor.y;
        self.solver.targets = drag
            .subjects
            .iter()
            .filter_map(|point| {
                drag.before.point(point).ok().map(|p| DragPoint {
                    point: point.clone(),
                    target: Point::new(p.x + dx, p.y + dy),
                })
            })
            .collect();
        self.solver.dirty = true;
    }
}
fn acceptable(input: &SketchDocument, candidate: &SketchDocument) -> bool {
    let mut shape = candidate.clone();
    if shape.entities.len() != input.entities.len() {
        return false;
    }
    for (a, b) in shape.entities.iter_mut().zip(&input.entities) {
        if std::mem::discriminant(&a.geometry) != std::mem::discriminant(&b.geometry) {
            return false;
        }
        if let (
            SketchGeometry::Arc {
                sweep_angle_rad: a, ..
            },
            SketchGeometry::Arc {
                sweep_angle_rad: b, ..
            },
        ) = (&a.geometry, &b.geometry)
            && a.signum() != b.signum()
        {
            return false;
        }
        a.geometry = b.geometry.clone();
    }
    shape == *input
        && candidate.require_valid_solution().is_ok()
        && candidate.wire_joints().is_ok_and(|j| {
            j.iter().all(|(a, b)| {
                candidate
                    .point(a)
                    .ok()
                    .zip(candidate.point(b).ok())
                    .is_some_and(|(a, b)| a.distance(b) <= 1e-6)
            })
        })
}
fn geometry_delta(a: &SketchDocument, b: &SketchDocument) -> f64 {
    a.entities
        .iter()
        .zip(&b.entities)
        .flat_map(|(a, b)| {
            a.geometry
                .parameters()
                .into_iter()
                .zip(b.geometry.parameters())
        })
        .map(|(a, b)| (a - b).abs())
        .fold(0., f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn editor() -> Editor {
        Editor::new(SavedDraft {
            target: None,
            source_revision: None,
            name: "Test".into(),
            model: serde_json::from_str(include_str!("../../../../fixtures/sketch/rectangle.json"))
                .unwrap(),
            had_document: true,
            constraint_form: Default::default(),
        })
    }
    #[test]
    fn stale_superseded_and_untrusted_candidates_never_replace_the_draft() {
        let mut e = editor();
        let before = e.saved.model.clone();
        let request = worker::Request {
            id: 4,
            revision: 2,
            model: before.clone(),
            targets: vec![],
        };
        let reply = worker::execute(request.clone());
        e.solver.revision = 2;
        e.solver.dirty = false;
        e.solver.flight = Some(Flight {
            request: request.clone(),
            timer: crate::clock::Timer::start(),
        });
        let mut stale = reply.clone();
        stale.revision = 1;
        e.receive_solve(stale);
        assert!(e.solver.flight.is_some());
        assert_eq!(e.saved.model, before);
        e.solver.dirty = true;
        e.receive_solve(reply.clone());
        assert!(e.solver.outcome.is_none());
        e.solver.flight = Some(Flight {
            request: request.clone(),
            timer: crate::clock::Timer::start(),
        });
        e.solver.dirty = false;
        let mut bad = reply.clone();
        bad.result.as_mut().unwrap().candidate.entities[0].id = "replaced".into();
        e.receive_solve(bad);
        assert!(!e.solver.outcome.as_ref().unwrap().accepted);
        assert_eq!(e.saved.model, before);
        e.solver.flight = Some(Flight {
            request,
            timer: crate::clock::Timer::start(),
        });
        e.cancel_solve();
        e.receive_solve(reply);
        assert_eq!(e.saved.model, before);
        assert!(!e.solving());
    }
}
