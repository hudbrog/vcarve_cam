use super::*;
use cam_core::project::v5::inspection::{PlanInspection, StageInspection};

#[derive(Default)]
pub(super) struct Inspection {
    pub point: Option<[f64; 2]>,
    pinned: Option<Pinned>,
}
struct Pinned {
    meta: PreviewMeta,
    cells: Vec<u8>,
    position: StagePosition,
    fingerprint: String,
    detail: serde_json::Value,
    removed: f64,
}
#[derive(Clone, Debug, serde::Serialize)]
struct StagePosition {
    stage: String,
    operation: String,
    fraction: f64,
}
impl StagePosition {
    fn at(stages: &[StageInspection], prefix: usize) -> Option<Self> {
        let s = stages
            .iter()
            .find(|s| prefix >= s.motion_range.0 && prefix <= s.motion_range.1)?;
        Some(Self {
            stage: s.stage_id.clone(),
            operation: s.operation_id.clone(),
            fraction: (prefix - s.motion_range.0) as f64 / s.motion_count.max(1) as f64,
        })
    }
    fn prefix(&self, stages: &[StageInspection]) -> Option<usize> {
        let s = stages
            .iter()
            .find(|s| s.stage_id == self.stage && s.operation_id == self.operation)?;
        Some(s.motion_range.0 + (self.fraction * s.motion_count as f64).round() as usize)
    }
}

#[derive(Clone, Copy, Debug, serde::Serialize)]
struct Sample {
    xy: [f64; 2],
    surface_z: f64,
    depth: f64,
    cell_mm: f64,
    depth_quantum_mm: f64,
}
fn sample(meta: &PreviewMeta, cells: &[u8], point: [f64; 2]) -> Option<Sample> {
    let [x, y] = point;
    if !x.is_finite()
        || !y.is_finite()
        || x < meta.stock.x0
        || y < meta.stock.y0
        || x >= meta.stock.x1
        || y >= meta.stock.y1
    {
        return None;
    }
    let col = ((x - meta.stock.x0) / meta.cell_mm).floor() as usize;
    let row = ((y - meta.stock.y0) / meta.cell_mm).floor() as usize;
    if col >= meta.cols || row >= meta.rows {
        return None;
    }
    let tile = row / sim::TILE * meta.tiles_x + col / sim::TILE;
    let at = (tile * sim::TILE * sim::TILE + row % sim::TILE * sim::TILE + col % sim::TILE) * 4;
    let packed = u32::from_le_bytes(cells.get(at..at + 4)?.try_into().ok()?);
    let quantum = meta.stock.thickness_mm / 65535.;
    let depth = (packed & 65535) as f64 * quantum;
    Some(Sample {
        xy: [
            meta.stock.x0 + (col as f64 + 0.5) * meta.cell_mm,
            meta.stock.y0 + (row as f64 + 0.5) * meta.cell_mm,
        ],
        surface_z: -depth,
        depth,
        cell_mm: meta.cell_mm,
        depth_quantum_mm: quantum,
    })
}

impl Viewport {
    pub fn reset_inspection(&mut self) {
        self.inspection = Inspection::default();
    }
    pub fn export_ready(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|s| s.meta.report["gui2"]["checks"]["exportReady"] == true)
    }
    /// The generated plan's read-only inspection, when a current scene carries
    /// one. Editors read resolved heights and generated evidence from here.
    pub fn plan_inspection(&self) -> Option<PlanInspection> {
        serde_json::from_value(self.scene.as_ref()?.meta.report["gui2"]["inspection"].clone()).ok()
    }
    fn stock_bytes(&self) -> Option<&[u8]> {
        let s = self.stock.as_ref()?;
        match &s.local {
            Some(cells) => Some(cells.as_slice()),
            None => self.scene.as_ref()?.payload.get(s.range.clone()),
        }
    }
    fn sampled(&self) -> Option<Sample> {
        sample(
            &self.stock.as_ref()?.meta,
            self.stock_bytes()?,
            self.inspection.point?,
        )
    }
    pub(super) fn comparison_prefix(&self) -> Option<usize> {
        self.inspection
            .pinned
            .as_ref()?
            .position
            .prefix(&self.plan_inspection()?.stages)
    }
    fn pin_inspection(&mut self) {
        let Some(plan) = self.plan_inspection() else {
            return;
        };
        let Some(position) = StagePosition::at(&plan.stages, self.stock_prefix) else {
            return;
        };
        let Some(stock) = &self.stock else { return };
        let Some(cells) = self.stock_bytes() else {
            return;
        };
        self.inspection.pinned = Some(Pinned {
            meta: stock.meta.clone(),
            cells: cells.to_vec(),
            position,
            fingerprint: plan.execution_fingerprint,
            detail: self.scene.as_ref().unwrap().meta.report["gui2"]["detailResidual"].clone(),
            removed: stock.stats.removed_volume_mm3,
        });
    }
    pub fn inspection_controls(&mut self, ui: &mut egui::Ui) {
        let Some(plan) = self.plan_inspection() else {
            ui.label("Generate a carving to inspect its stages and stock depth.");
            return;
        };
        if !self.export_ready() {
            ui.colored_label(
                Color32::from_rgb(164, 83, 12),
                "Generation is incomplete or basic checks failed. This result cannot be exported.",
            );
        }
        let pos = StagePosition::at(&plan.stages, self.stock_prefix);
        let stage = pos
            .as_ref()
            .and_then(|p| plan.stages.iter().find(|s| s.stage_id == p.stage));
        ui.horizontal_wrapped(|ui| {
            ui.strong(if self.result_current {
                "Current result"
            } else {
                "Stale result · regenerate after edits"
            });
            if let Some(s) = stage {
                ui.label(format!(
                    "{} · tool {} · {}/{} motions",
                    s.stage_id,
                    s.tool_id,
                    self.stock_prefix.saturating_sub(s.motion_range.0),
                    s.motion_count
                ));
            }
            let pin = ui.add_enabled(
                !self.stock_loading && self.requested_stock.is_none(),
                egui::Button::new("Pin comparison"),
            );
            crate::app::observe_control("Pin comparison", pin.rect);
            if pin.clicked() {
                self.pin_inspection();
            }
            if self.inspection.pinned.is_some() {
                let matched = ui.add_enabled(
                    self.comparison_prefix().is_some() && !self.stock_loading,
                    egui::Button::new("Match pinned position"),
                );
                crate::app::observe_control("Match pinned position", matched.rect);
                if matched.clicked()
                    && let Some(prefix) = self.comparison_prefix()
                {
                    self.playing = false;
                    self.stock_seek(prefix);
                }
                if ui.button("Clear comparison").clicked() {
                    self.inspection.pinned = None;
                }
            }
        });
        ui.horizontal(|ui| {
            let b = self.scene.as_ref().unwrap().meta.bounds;
            let mut xy = self
                .inspection
                .point
                .unwrap_or([(b[0] + b[2]) / 2., (b[1] + b[3]) / 2.]);
            ui.label("Inspect XY");
            let x = ui.add(egui::DragValue::new(&mut xy[0]).speed(0.1).prefix("X "));
            crate::app::observe_control("Inspect X", x.rect);
            let y = ui.add(egui::DragValue::new(&mut xy[1]).speed(0.1).prefix("Y "));
            crate::app::observe_control("Inspect Y", y.rect);
            if x.changed() || y.changed() || self.inspection.point.is_none() {
                self.inspection.point = Some(xy);
            }
            ui.label("mm");
        });
        ui.small("Click viewport to choose XY on the stock-top plane.");
        if let Some(s) = self.sampled() {
            ui.label(format!(
                "Cell centre ({:.3}, {:.3}) · surface Z {:.4} mm · removed depth {:.4} mm",
                s.xy[0], s.xy[1], s.surface_z, s.depth
            ));
            ui.small(format!("Display grid {:.4} mm; depth quantum {:.6} mm. Readout is sampled stock, not a machining tolerance check.", s.cell_mm, s.depth_quantum_mm));
        } else {
            ui.label("Point is outside the displayed stock.");
        }
        if let Some(pinned) = &self.inspection.pinned {
            let matched = self.comparison_prefix() == Some(self.stock_prefix)
                && !self.stock_loading
                && self.requested_stock.is_none();
            ui.small(format!(
                "Pinned {} · {} at {:.1}% · detail {} mm · {:.2} mm³ removed",
                &pinned.fingerprint[..pinned.fingerprint.len().min(12)],
                pinned.position.stage,
                pinned.position.fraction * 100.,
                pinned.detail,
                pinned.removed
            ));
            if matched {
                if let (Some(previous), Some(current)) = (
                    self.inspection
                        .point
                        .and_then(|xy| sample(&pinned.meta, &pinned.cells, xy)),
                    self.sampled(),
                ) {
                    ui.label(format!("Same XY, corresponding stage position: pinned depth {:.4} to displayed {:.4} mm · Δ {:+.4} mm", previous.depth, current.depth, current.depth-previous.depth));
                }
            } else {
                ui.label("Use Match pinned position to compare the same stage progress. A missing stage cannot be compared.");
            }
        }
        if let Some(pick) = self
            .selection
            .filter(|p| self.visible_motion_range().contains(&(p.motion as usize)))
        {
            let scene = self.scene.as_ref().unwrap();
            if let Some(points) = self.picker.as_ref().and_then(|p| p.endpoints(pick.motion)) {
                let b = scene.meta.bounds;
                let scale = (b[2] - b[0]).max(b[3] - b[1]).max(0.001) / 1.6;
                let end = points[1];
                let stage = plan.stages.iter().find(|s| {
                    (s.motion_range.0..s.motion_range.1).contains(&(pick.motion as usize))
                });
                ui.label(format!(
                    "Motion {} · {} · endpoint ({:.3}, {:.3}, {:.3}) mm",
                    pick.motion,
                    stage.map_or("", |s| s.stage_id.as_str()),
                    end[0] as f64 * scale + (b[0] + b[2]) / 2.,
                    end[1] as f64 * scale + (b[1] + b[3]) / 2.,
                    end[2] as f64 * scale
                ));
            }
        }
    }
    pub(super) fn inspection_marker(&self, ui: &egui::Ui, rect: egui::Rect) {
        if self.artwork.enabled {
            return;
        }
        if let (Some(xy), Some(scene)) = (self.inspection.point, &self.scene) {
            let point = crate::artwork_view::screen_point(
                self.camera(rect),
                scene.meta.bounds,
                rect,
                cam_core::geometry::Point::new(xy[0], xy[1]),
            );
            let painter = ui.painter().with_clip_rect(rect);
            painter.circle_stroke(
                point,
                5.,
                egui::Stroke::new(2., Color32::from_rgb(158, 42, 123)),
            );
        }
    }
    pub fn inspection_snapshot(&self) -> serde_json::Value {
        serde_json::json!({"sample":self.sampled(), "position":self.plan_inspection().and_then(|p| StagePosition::at(&p.stages, self.stock_prefix)), "pinned":self.inspection.pinned.as_ref().map(|p| &p.position), "comparisonPrefix":self.comparison_prefix(), "current":self.result_current})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readout_samples_real_packed_stock_across_tile_boundaries() {
        let stock = sim::Stock {
            x0: 0.,
            y0: 0.,
            x1: 30.,
            y1: 30.,
            thickness_mm: 10.,
        };
        let mut field =
            sim::Field::new(stock, &[sim::ToolSpec::Endmill { diameter: 3. }], 0.1).unwrap();
        field
            .apply(
                &sim::Motion {
                    kind: "cut".into(),
                    tool: 0,
                    x0: 27.,
                    y0: 27.,
                    z0: -2.,
                    x1: 28.,
                    y1: 27.,
                    z1: -2.,
                },
                0.,
                1.,
            )
            .unwrap();
        let meta = PreviewMeta {
            stock,
            cols: 300,
            rows: 300,
            cell_mm: 0.1,
            reference_cell_mm: 0.025,
            tiles_x: 2,
            tiles_y: 2,
            retained_bytes: 0,
            frames: vec![],
        };
        let bytes = field.packed_tile_bytes();
        let cut = sample(&meta, &bytes, [27., 27.]).unwrap();
        assert!((cut.depth - 2.).abs() <= cut.depth_quantum_mm);
        assert_eq!(cut.surface_z, -cut.depth);
        assert_eq!(sample(&meta, &bytes, [1., 1.]).unwrap().depth, 0.);
        assert!(sample(&meta, &bytes, [30., 1.]).is_none());
        assert!(sample(&meta, &bytes, [-0.1, 1.]).is_none());
        assert!(sample(&meta, &[], [27., 27.]).is_none());
    }
    #[test]
    fn comparison_matches_stage_identity_and_fraction_after_motion_count_changes() {
        let mut stage = StageInspection {
            stage_id: "carving/finish".into(),
            operation_id: "carving".into(),
            tool_id: "vbit".into(),
            role: cam_core::sequence::StageRole::VcarveFinish,
            motion_range: (100, 200),
            motion_count: 100,
        };
        let p = StagePosition::at(&[stage.clone()], 175).unwrap();
        stage.motion_range = (300, 500);
        stage.motion_count = 200;
        assert_eq!(p.prefix(&[stage.clone()]), Some(450));
        stage.stage_id = "other-stage".into();
        assert_eq!(p.prefix(&[stage]), None);
    }
}
