#![cfg(not(target_arch = "wasm32"))]
use cam_core::{geometry::Point, sketch::*};
use cam_gui_runtime::sketch::worker::{Reply, Request};
use std::{
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};
struct Worker {
    child: Child,
    folder: PathBuf,
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        for name in ["pending", "request", "result", "response"] {
            let _ = std::fs::remove_file(self.folder.join(name));
        }
        let _ = std::fs::remove_dir(&self.folder);
    }
}
impl Worker {
    fn request(&mut self, r: Request) -> Reply {
        std::fs::write(self.folder.join("pending"), serde_json::to_vec(&r).unwrap()).unwrap();
        std::fs::rename(self.folder.join("pending"), self.folder.join("request")).unwrap();
        let timer = Instant::now();
        while !self.folder.join("response").exists() {
            assert!(
                self.child.try_wait().unwrap().is_none(),
                "solver child exited"
            );
            assert!(timer.elapsed() < Duration::from_secs(20));
            std::thread::sleep(Duration::from_millis(10));
        }
        let reply =
            serde_json::from_slice(&std::fs::read(self.folder.join("response")).unwrap()).unwrap();
        std::fs::remove_file(self.folder.join("response")).unwrap();
        reply
    }
}
#[test]
fn native_solver_process_handles_resize_conflict_and_drag_with_request_identity() {
    let folder = std::env::temp_dir().join(format!("sketch-worker-test-{}", std::process::id()));
    std::fs::create_dir(&folder).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_cam-gui"));
    command.arg("--sketch-worker").arg(&folder);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut worker = Worker {
        child: command.spawn().unwrap(),
        folder,
    };
    let model: SketchDocument =
        serde_json::from_str(include_str!("../../../fixtures/sketch/rectangle.json")).unwrap();
    let reply = worker.request(Request {
        id: 1,
        revision: 17,
        model: model.clone(),
        targets: vec![],
    });
    assert_eq!((reply.id, reply.revision), (1, 17));
    let out = reply.result.unwrap();
    assert!(out.accepted);
    assert_eq!(out.degrees_of_freedom, Some(0));
    let circle = model
        .entities
        .iter()
        .find(|e| matches!(e.geometry, SketchGeometry::Circle { .. }))
        .unwrap();
    let point = PointRef {
        entity_id: circle.id.clone(),
        point: PointRole::Center,
    };
    let reply = worker.request(Request {
        id: 2,
        revision: 18,
        model: model.clone(),
        targets: vec![cam_sketch_solver::DragPoint {
            point: point.clone(),
            target: Point::new(500., 500.),
        }],
    });
    let out = reply.result.unwrap();
    assert!(out.accepted);
    assert!(out.drag_error_mm.unwrap() > 100.);
    assert!(
        out.candidate
            .point(&point)
            .unwrap()
            .distance(model.point(&point).unwrap())
            < 1e-6
    );
    let mut conflict = model.clone();
    conflict.constraints.push(Constraint {
        id: "conflict".into(),
        reference: false,
        definition: ConstraintKind::Radius {
            curve: circle.id.clone(),
            value_mm: 50.,
        },
    });
    let reply = worker.request(Request {
        id: 3,
        revision: 19,
        model: conflict,
        targets: vec![],
    });
    let out = reply.result.unwrap();
    assert!(!out.accepted);
    assert!(!out.issues.is_empty());
    let mut resized = model.clone();
    if let ConstraintKind::Length { value_mm, .. } = &mut resized.constraints[8].definition {
        *value_mm = 25.;
    }
    let out = worker
        .request(Request {
            id: 4,
            revision: 20,
            model: resized,
            targets: vec![],
        })
        .result
        .unwrap();
    assert!(out.accepted);
    assert_eq!(out.degrees_of_freedom, Some(0));
    let (a, b) = out.candidate.entities[0].geometry.line().unwrap();
    assert!((a.distance(b) - 25.).abs() < 1e-6);
    for (a, b) in out.candidate.wire_joints().unwrap() {
        assert!(
            out.candidate
                .point(&a)
                .unwrap()
                .distance(out.candidate.point(&b).unwrap())
                < 1e-6
        );
    }
}
