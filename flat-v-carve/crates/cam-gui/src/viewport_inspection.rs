use super::*;
use cam_core::project::v5::inspection::{PlanInspection, StageInspection};

#[derive(Default)]
pub(super) struct Inspection {
    pub point: Option<[f64; 2]>,
    /// Axis the section plot cuts along through `point`.
    pub axis: SectionAxis,
    pinned: Option<Pinned>,
}

/// Section line through the inspected point: X varies across the stock at the
/// point's Y, or the other way round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SectionAxis {
    #[default]
    X,
    Y,
}

impl SectionAxis {
    fn label(self) -> &'static str {
        match self {
            Self::X => "Section X",
            Self::Y => "Section Y",
        }
    }
}

/// One raster cross-section. Every sample is a cell centre of the *displayed*
/// raster, so the plot states exactly what the display resolved and nothing
/// more; it is never fed back into the CAM engine.
struct Section {
    axis: SectionAxis,
    /// Coordinate held constant by this section, in millimetres.
    fixed_mm: f64,
    /// First sample's coordinate along the varying axis, in millimetres.
    start_mm: f64,
    cell_mm: f64,
    quantum_mm: f64,
    /// Removed depth at each cell centre, in millimetres.
    depths: Vec<f64>,
    /// Sample nearest the inspected point.
    at: usize,
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
    let quantum = meta.stock.thickness_mm / 65535.;
    let depth = cell_depth(meta, cells, col, row)?;
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

/// Removed depth of one display cell, in millimetres.
fn cell_depth(meta: &PreviewMeta, cells: &[u8], col: usize, row: usize) -> Option<f64> {
    if col >= meta.cols || row >= meta.rows {
        return None;
    }
    let tile = row / sim::TILE * meta.tiles_x + col / sim::TILE;
    let at = (tile * sim::TILE * sim::TILE + row % sim::TILE * sim::TILE + col % sim::TILE) * 4;
    let packed = u32::from_le_bytes(cells.get(at..at + 4)?.try_into().ok()?);
    Some((packed & 65535) as f64 * (meta.stock.thickness_mm / 65535.))
}

impl Viewport {
    pub fn reset_inspection(&mut self) {
        self.inspection = Inspection::default();
    }
    /// Renderer diagnostics: what a failed device or resource looks like from
    /// the application's side, and how it recovers from retained CPU data.
    /// These controls change nothing about the document or the result; any
    /// test or review bridge like this must stay development-facing.
    fn renderer_diagnostics(&mut self, ui: &mut egui::Ui) {
        let header = egui::CollapsingHeader::new("Renderer diagnostics")
            .id_salt("renderer-diagnostics")
            .show(ui, |ui| {
                let probe = self.renderer_probe();
                let frame_p50 = probe["frameMs"]["p50"].as_f64().unwrap_or(0.);
                let frame_p95 = probe["frameMs"]["p95"].as_f64().unwrap_or(0.);
                ui.small(format!(
                    "Build {} · protocol {} · {} required page(s) {:?}",
                    probe["build"]["version"].as_str().unwrap_or("unknown"),
                    probe["build"]["protocol"].as_str().unwrap_or("unknown"),
                    probe["requiredPages"].as_u64().unwrap_or(0),
                    probe["requiredPageRange"],
                ));
                ui.small(format!(
                    "{} · {} recovery step(s) · {} page(s) deferred · {} stock tile(s) pending · {} frame(s) sampled, p50 {frame_p50:.1} ms · p95 {frame_p95:.1} ms",
                    if probe["unavailable"] == true {
                        "Failure injected: the viewport draws nothing"
                    } else {
                        "Drawing normally"
                    },
                    probe["recoveries"].as_u64().unwrap_or(0),
                    probe["pagesDeferred"].as_u64().unwrap_or(0),
                    probe["pendingTiles"].as_u64().unwrap_or(0),
                    probe["frameMs"]["samples"].as_u64().unwrap_or(0),
                ));
                ui.small(format!(
                    "Backend {} · {} resident page(s) / {:.1} MiB · {} page upload(s) / {:.1} MiB total · {} skipped (cache hits)",
                    probe["backend"].as_str().unwrap_or("unknown"),
                    probe["residentPages"].as_u64().unwrap_or(0),
                    probe["residentBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["pageUploads"].as_u64().unwrap_or(0),
                    probe["uploadBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["pagesSkipped"].as_u64().unwrap_or(0),
                ));
                ui.small(format!(
                    "Memory · scene {:.1} MiB · packed checkpoints {:.1} MiB · worker field {:.1} MiB · worker checkpoints {:.1} MiB · decoded motions {:.1} MiB · GPU pages {:.1} MiB · stock tiles {:.1} MiB = {:.1} MiB of the {:.0} MiB display budget{}",
                    probe["memory"]["sceneBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["checkpointBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["workerFieldBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["workerCheckpointBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["workerMotionBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["gpuPageBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["stockTileBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["declaredDisplayBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    probe["memory"]["displayBudgetBytes"].as_f64().unwrap_or(0.) / 1_048_576.,
                    if probe["memory"]["withinBudget"] == true {
                        ""
                    } else {
                        " — over budget"
                    },
                ));
                ui.small(
                    "WASM linear memory, the JS heap and total GPU memory are not visible to this panel; they are an unknown, not zero.",
                );
                ui.small(
                    "A failure or recovery never changes the document, the retained result, the playhead or the display resolution.",
                );
                ui.horizontal_wrapped(|ui| {
                    let inject = ui.add_enabled(
                        !self.gpu_unavailable,
                        egui::Button::new("Inject renderer failure"),
                    );
                    crate::app::observe_control("Inject renderer failure", inject.rect);
                    if inject.clicked() {
                        self.inject_renderer_failure();
                    }
                    let recover = ui.add_enabled(
                        self.gpu_unavailable || self.gpu,
                        egui::Button::new("Rebuild renderer resources"),
                    );
                    crate::app::observe_control("Rebuild renderer resources", recover.rect);
                    if recover.clicked() {
                        self.rebuild_renderer_resources();
                    }
                });
            });
        crate::app::observe_control("Renderer diagnostics", header.header_response.rect);
    }
    /// Section through the inspected point, drawn from the displayed raster.
    /// The plot feeds nothing back into the engine: it is a labelled display of
    /// what the current resolution resolved.
    fn section_controls(&mut self, ui: &mut egui::Ui) {
        if self.inspection.point.is_none() || self.stock.is_none() {
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Section");
            for axis in [SectionAxis::X, SectionAxis::Y] {
                let selected = self.inspection.axis == axis;
                let response = ui.selectable_label(selected, axis.label());
                crate::app::observe_control(axis.label(), response.rect);
                if response.clicked() {
                    self.inspection.axis = axis;
                }
            }
            if let Some(section) = self.section() {
                let along = match section.axis {
                    SectionAxis::X => "X",
                    SectionAxis::Y => "Y",
                };
                ui.small(format!("along {along} at {:.3} mm", section.fixed_mm));
            }
        });
        let Some(section) = self.section() else {
            return;
        };
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width().max(180.), 96.),
            egui::Sense::hover(),
        );
        crate::app::observe_control("Section plot", rect);
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 2., Color32::from_rgb(20, 22, 26));
        let count = section.depths.len();
        if count == 0 {
            return;
        }
        let max_depth = section
            .depths
            .iter()
            .copied()
            .fold(0.0_f64, f64::max)
            .max(section.quantum_mm * 4.)
            .max(1e-6);
        let left = rect.left() + 6.;
        let width = (rect.width() - 12.).max(1.);
        let to_point = |index: usize, depth: f64| {
            let fraction = if count <= 1 {
                0.5
            } else {
                index as f32 / (count - 1) as f32
            };
            egui::pos2(
                left + fraction * width,
                rect.top() + 6. + (depth / max_depth) as f32 * (rect.height() - 22.),
            )
        };
        // The original stock top, then the material the raster says was removed.
        let top = to_point(0, 0.).y;
        painter.line_segment(
            [
                egui::pos2(rect.left() + 3., top),
                egui::pos2(rect.right() - 3., top),
            ],
            egui::Stroke::new(1., Color32::from_rgb(70, 76, 86)),
        );
        let points: Vec<egui::Pos2> = section
            .depths
            .iter()
            .enumerate()
            .map(|(index, depth)| to_point(index, *depth))
            .collect();
        for pair in points.windows(2) {
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pair[0],
                    pair[1],
                    egui::pos2(pair[1].x, top),
                    egui::pos2(pair[0].x, top),
                ],
                Color32::from_rgb(52, 84, 120),
                egui::Stroke::NONE,
            ));
        }
        painter.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(1.4, Color32::from_rgb(150, 200, 255)),
        ));
        // The inspected point and the width of one display cell.
        let at = section.at.min(count - 1);
        let marker = points[at];
        painter.line_segment(
            [
                egui::pos2(marker.x, rect.top() + 3.),
                egui::pos2(marker.x, rect.bottom() - 3.),
            ],
            egui::Stroke::new(1., Color32::from_rgb(255, 210, 90)),
        );
        let cell_px = (width / count.max(1) as f32).max(1.);
        let bar_y = rect.bottom() - 8.;
        painter.line_segment(
            [egui::pos2(left, bar_y), egui::pos2(left + cell_px, bar_y)],
            egui::Stroke::new(3., Color32::from_rgb(220, 220, 220)),
        );
        let cut = section.depths.iter().filter(|depth| **depth > 0.).count();
        ui.small(format!(
            "{count} cell-centre samples from {:.3} mm · scale bar is one {:.4} mm cell · deepest {:.4} mm · {cut} of {count} cells cut · raster approximation only",
            section.start_mm, section.cell_mm, max_depth
        ));
    }
    /// The display grid's cell size in millimetres, when a stock field is
    /// displayed. Features smaller than a cell can be missed by the raster.
    pub fn display_cell_mm(&self) -> Option<f64> {
        Some(self.stock.as_ref()?.meta.cell_mm)
    }
    /// Samples the section plot would draw right now; zero when no section is
    /// available. Published so the browser check can see the plot exist.
    pub fn section_sample_count(&self) -> usize {
        self.section().map_or(0, |section| section.depths.len())
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
    /// The cross-section through the inspected point, taken from the displayed
    /// raster only. A section is the honest way to show what the cell size
    /// bought: the samples are cell centres and their depth quantum is stated.
    fn section(&self) -> Option<Section> {
        let stock = self.stock.as_ref()?;
        let meta = &stock.meta;
        let cells = self.stock_bytes()?;
        let point = self.inspection.point?;
        if !point.iter().all(|value| value.is_finite()) {
            return None;
        }
        if point[0] < meta.stock.x0
            || point[1] < meta.stock.y0
            || point[0] >= meta.stock.x1
            || point[1] >= meta.stock.y1
        {
            return None;
        }
        let cell = meta.cell_mm;
        let quantum = meta.stock.thickness_mm / 65535.;
        let (col, row) = (
            ((point[0] - meta.stock.x0) / cell).floor(),
            ((point[1] - meta.stock.y0) / cell).floor(),
        );
        let (axis, count, fixed_mm, start_mm, fixed_index) = match self.inspection.axis {
            SectionAxis::X => (
                SectionAxis::X,
                meta.cols,
                meta.stock.y0 + (row + 0.5) * cell,
                meta.stock.x0 + 0.5 * cell,
                row,
            ),
            SectionAxis::Y => (
                SectionAxis::Y,
                meta.rows,
                meta.stock.x0 + (col + 0.5) * cell,
                meta.stock.y0 + 0.5 * cell,
                col,
            ),
        };
        let at = match self.inspection.axis {
            SectionAxis::X => col,
            SectionAxis::Y => row,
        };
        let at = at.clamp(0., (count.saturating_sub(1)) as f64) as usize;
        let mut depths = Vec::with_capacity(count);
        for index in 0..count {
            let (col, row) = match self.inspection.axis {
                SectionAxis::X => (index, fixed_index as usize),
                SectionAxis::Y => (fixed_index as usize, index),
            };
            depths.push(cell_depth(meta, cells, col, row)?);
        }
        Some(Section {
            axis,
            fixed_mm,
            start_mm,
            cell_mm: cell,
            quantum_mm: quantum,
            depths,
            at,
        })
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
            // Labeled approximation: the raster is a display model. State what
            // it resolved, what the plan asked for, and where the exact
            // generated geometry lives instead of implying cell accuracy.
            let reference = self
                .stock
                .as_ref()
                .map_or(s.cell_mm, |stock| stock.meta.reference_cell_mm);
            if s.cell_mm > reference * 1.001 {
                ui.colored_label(
                    Color32::from_rgb(164, 83, 12),
                    format!(
                        "Display raster {:.4} mm cells · {:.1}× coarser than the plan's {:.4} mm requested resolution. Features narrower than one cell may not appear; the generated boundaries drawn in the viewport are exact and are not rasterized.",
                        s.cell_mm,
                        s.cell_mm / reference.max(f64::MIN_POSITIVE),
                        reference
                    ),
                );
            } else {
                ui.small(format!(
                    "Display raster {:.4} mm cells, at the plan's requested resolution ({:.4} mm).",
                    s.cell_mm, reference
                ));
            }
            ui.small(format!(
                "Depth quantum {:.6} mm. The readout is sampled display stock, not a machining tolerance check.",
                s.depth_quantum_mm
            ));
        } else {
            ui.label("Point is outside the displayed stock.");
        }
        self.section_controls(ui);
        self.renderer_diagnostics(ui);
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
    use crate::stock_preview::DisplayPreset;
    #[test]
    fn readout_samples_real_packed_stock_across_tile_boundaries() {
        let stock = sim::Stock {
            x0: 0.,
            y0: 0.,
            x1: 30.,
            y1: 30.,
            thickness_mm: 10.,
        };
        let mut field = sim::Field::new(
            stock,
            &[sim::ToolSpec::Endmill {
                diameter: 3.,
                cutting_length: 12.,
            }],
            0.1,
        )
        .unwrap();
        field
            .apply(
                &sim::Motion {
                    kind: "cut".into(),
                    tool: 0,
                    stage: 0,
                    interpolation: sim::Interpolation::Feed,
                    feed_mm_min: Some(100.),
                    arc: None,
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
            ladder_frames: 0,
            preset: DisplayPreset::Standard,
            key: "readout-fixture".into(),
            frames: vec![],
            dropped_stage_marks: 0,
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

    /// The section is the display's own cross-section: cell centres of the
    /// displayed raster, in both axes, with the inspected point marked.
    #[test]
    fn the_section_samples_the_displayed_raster_in_both_axes() {
        let stock = sim::Stock {
            x0: 0.,
            y0: 0.,
            x1: 20.,
            y1: 20.,
            thickness_mm: 8.,
        };
        let mut field = sim::Field::new(
            stock,
            &[sim::ToolSpec::Endmill {
                diameter: 2.,
                cutting_length: 12.,
            }],
            0.5,
        )
        .unwrap();
        // One cut band along Y at x = 10 with a 3 mm depth.
        field
            .apply(
                &sim::Motion {
                    kind: "cut".into(),
                    tool: 0,
                    stage: 0,
                    interpolation: sim::Interpolation::Feed,
                    feed_mm_min: Some(100.),
                    arc: None,
                    x0: 10.,
                    y0: 2.,
                    z0: -3.,
                    x1: 10.,
                    y1: 18.,
                    z1: -3.,
                },
                0.,
                1.,
            )
            .unwrap();
        let bytes = field.packed_tile_bytes();
        let meta = PreviewMeta {
            stock,
            cols: 40,
            rows: 40,
            cell_mm: 0.5,
            reference_cell_mm: 0.1,
            tiles_x: 1,
            tiles_y: 1,
            retained_bytes: bytes.len(),
            ladder_frames: 1,
            preset: DisplayPreset::Standard,
            key: "section-fixture".into(),
            frames: vec![],
            dropped_stage_marks: 0,
        };
        let mut view = Viewport {
            stock: Some(StockView {
                meta,
                identity: 7,
                range: 0..bytes.len(),
                local: Some(Arc::new(bytes)),
                cell_versions: Arc::new(vec![]),
                stats: sim::Stats::default(),
                prefix: 0,
                clock: None,
                raster_revision: 0,
                last_transfer_bytes: 0,
                last_replayed: 0,
            }),
            ..Default::default()
        };
        view.inspection.point = Some([10., 10.]);

        view.inspection.axis = SectionAxis::X;
        let x = view.section().unwrap();
        assert_eq!(x.axis, SectionAxis::X);
        assert_eq!(x.depths.len(), 40, "one sample per displayed cell");
        assert_eq!(x.at, 20, "the inspected point's own cell");
        assert!((x.fixed_mm - 10.25).abs() < 1e-9);
        assert!(x.depths[20] > 0., "the cut band is resolved");
        assert_eq!(x.depths[0], 0., "intact stock is reported as intact");
        assert_eq!(x.depths[39], 0.);

        view.inspection.axis = SectionAxis::Y;
        let y = view.section().unwrap();
        assert_eq!(y.axis, SectionAxis::Y);
        assert_eq!(y.depths.len(), 40);
        assert_eq!(y.at, 20);
        assert!((y.fixed_mm - 10.25).abs() < 1e-9);
        assert!(y.depths[20] > 0.);
        assert_eq!(y.depths[0], 0.);
        assert_eq!(y.depths[39], 0.);

        // Outside the stock there is no section to draw, exactly as there is no
        // sample readout.
        view.inspection.point = Some([40., 10.]);
        assert!(view.section().is_none());
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
