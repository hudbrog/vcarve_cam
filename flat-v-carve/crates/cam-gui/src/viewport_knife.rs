use super::*;
use serde_json::{Value, json};

fn number(v: &Value) -> String {
    v.as_f64()
        .map(|n| format!("{n:.4}"))
        .unwrap_or_else(|| "unavailable".into())
}

impl Viewport {
    pub fn knife_chains(&self) -> Arc<Vec<crate::knife::Chain>> {
        self.knife_chains.clone()
    }
    pub fn is_knife(&self) -> bool {
        self.scene
            .as_ref()
            .is_some_and(|s| s.meta.report["gui2"]["knife"] == true)
    }
    pub fn stale_knife_evidence(&mut self) {
        if let Some(s) = &mut self.scene {
            s.meta.report["gui2"]["knifeEvidenceStale"] = json!(true);
        }
        self.overlay_signature = None;
    }
    pub fn accept_knife_evidence(&mut self, evidence: &Value, sha: &Value) {
        if let Some(s) = &mut self.scene
            && evidence["programSha256"] == *sha
            && !sha.is_null()
            && evidence["executionFingerprint"] == s.meta.report["gui2"]["executionFingerprint"]
        {
            s.meta.report["gui2"]["knifeEvidence"] = evidence.clone();
            s.meta.report["gui2"]["knifeEvidenceStale"] = json!(false);
            self.overlay_signature = None;
        }
    }
    pub fn knife_inspection_controls(&mut self, ui: &mut egui::Ui) {
        ui.heading("Knife inspection");
        ui.small("Stock remains unchanged. Traces project onto its top: orange pivot, cyan intended tip, green emitted replay, white modeled blade. Actual Z is shown below.");
        let Some(scene) = &self.scene else {
            return;
        };
        let report = &scene.meta.report["gui2"];
        let Some(motions) = report["knifeMotions"].as_array() else {
            ui.label("Generate knife toolpaths first.");
            return;
        };
        if motions.is_empty() {
            ui.label(
                "No knife motions were generated. Resolve the reported settings or geometry issue.",
            );
            return;
        }
        let index = self
            .selection
            .map(|p| p.motion as usize)
            .unwrap_or(self.playhead.saturating_sub(1))
            .min(motions.len().saturating_sub(1));
        let motion = &motions[index];
        ui.label(format!(
            "Motion {} · {} · pass {} · layer {}",
            index + 1,
            motion["purpose"]
                .as_str()
                .unwrap_or("unknown")
                .replace('_', " "),
            motion["pass"],
            motion["layer"]
        ));
        ui.label(format!(
            "Z: {} to {} mm",
            number(&motion["start"][2]),
            number(&motion["end"][2])
        ));
        ui.label(format!(
            "Modeled heading: {} to {}°",
            number(&motion["heading"][0]),
            number(&motion["heading"][1])
        ));
        let next = motions
            .iter()
            .enumerate()
            .skip(index + 1)
            .find(|(_, m)| m["purpose"] != motion["purpose"])
            .map(|(i, _)| i);
        let previous = (0..index)
            .rev()
            .find(|&i| motions[i]["purpose"] != motion["purpose"]);
        let mut seek = None;
        let previous_button = ui.button("Previous corner / entry");
        crate::app::observe_control("Previous corner / entry", previous_button.rect);
        if previous_button.clicked() {
            seek = previous.map(|i| i + 1);
        }
        let next_button = ui.button("Next corner / entry");
        crate::app::observe_control("Next corner / entry", next_button.rect);
        if next_button.clicked() {
            seek = next.map(|i| i + 1);
        }
        ui.small("Tip overlay shows up to the latest 2,048 motions at the playhead. Pivot paths use the complete paged execution.");
        let e = &report["knifeEvidence"];
        if e.is_null() {
            ui.label("Prepare output to inspect replay from actual emitted bytes.");
        } else if report["knifeEvidenceStale"] == true || !self.result_current {
            ui.colored_label(
                Color32::DARK_RED,
                "Emitted replay is stale. Prepare output again.",
            );
        } else {
            ui.label(format!(
                "Emitted replay: {}",
                e["status"].as_str().unwrap_or("unavailable")
            ));
            ui.label(format!(
                "Max tip deviation: {} mm / {} mm",
                number(&e["maxTipDeviationMm"]),
                number(&e["tipBudgetMm"])
            ));
            ui.label(format!(
                "Max heading error: {}° / {}°",
                number(&e["maxHeadingErrorDeg"]),
                number(&e["headingToleranceDeg"])
            ));
            ui.label(format!(
                "Precision: {} digits · offset X {} / Y {} / Z {} mm",
                e["outputDecimalPlaces"],
                number(&e["machineOffsetMm"][0]),
                number(&e["machineOffsetMm"][1]),
                number(&e["machineOffsetMm"][2])
            ));
            ui.small(format!(
                "Program SHA-256: {}",
                e["programSha256"].as_str().unwrap_or("missing")
            ));
            ui.label(format!(
                "Display samples: {} / {}",
                e["samples"].as_array().map_or(0, Vec::len),
                e["totalSamples"]
            ));
            if e["truncated"] == true {
                ui.small("Samples are bounded. Missing trace sections are not interpolated; aggregate status covers the full replay check.");
            }
            if let Some(sample) = e["samples"].as_array().and_then(|samples| {
                samples
                    .iter()
                    .find(|s| s["motionIndex"] == index && s["atStart"] == false)
            }) {
                ui.label(format!(
                    "Replayed heading: {}° · tip deviation: {} mm",
                    number(&sample["replayedHeadingDeg"]),
                    number(&sample["deviationMm"])
                ));
            } else {
                ui.small("No emitted sample for this motion.");
            }
        }
        if let Some(prefix) = seek {
            self.selection = None;
            self.playing = false;
            self.stock_seek(prefix);
        }
    }

    pub(super) fn knife_overlay(&self, out: &mut overlay::Overlay) {
        let Some(scene) = &self.scene else {
            return;
        };
        let report = &scene.meta.report["gui2"];
        let Some(motions) = report["knifeMotions"].as_array() else {
            return;
        };
        let point = |p: &Value, z: f64| -> Option<[f32; 3]> {
            Some(
                crate::compute::vertex(
                    [p["x"].as_f64()?, p["y"].as_f64()?, z.max(0.) + 0.03],
                    scene.meta.bounds,
                    [1.; 4],
                )
                .position,
            )
        };
        let mut line = |a: [f32; 3], b: [f32; 3], color: [f32; 4]| {
            out.lines
                .extend([Vertex { position: a, color }, Vertex { position: b, color }]);
        };
        let end = self.playhead.min(motions.len());
        for m in &motions[end.saturating_sub(2_048)..end] {
            let pivot = |p: &Value| -> Option<[f32; 3]> {
                Some(
                    crate::compute::vertex(
                        [
                            p[0].as_f64()?,
                            p[1].as_f64()?,
                            p[2].as_f64()?.max(0.) + 0.025,
                        ],
                        scene.meta.bounds,
                        [1.; 4],
                    )
                    .position,
                )
            };
            if let (Some(a), Some(b)) = (pivot(&m["start"]), pivot(&m["end"])) {
                line(
                    a,
                    b,
                    if m["contact"] == true {
                        [1., 0.62, 0.2, 1.]
                    } else {
                        [0.3, 0.36, 0.44, 0.45]
                    },
                );
            }
            if m["contact"] != true {
                continue;
            }
            if let (Some(a), Some(b)) = (
                point(&m["tipStart"], m["start"][2].as_f64().unwrap_or(0.)),
                point(&m["tipEnd"], m["end"][2].as_f64().unwrap_or(0.)),
            ) {
                line(a, b, [0.2, 0.95, 1., 1.]);
            }
        }
        if let Some(m) = end.checked_sub(1).and_then(|i| motions.get(i)) {
            let p = &m["end"];
            if let (Some(x), Some(y), Some(z), Some(tip)) = (
                p[0].as_f64(),
                p[1].as_f64(),
                p[2].as_f64(),
                point(&m["tipEnd"], p[2].as_f64().unwrap_or(0.)),
            ) {
                line(
                    crate::compute::vertex([x, y, z.max(0.) + 0.03], scene.meta.bounds, [1.; 4])
                        .position,
                    tip,
                    [1.; 4],
                );
            }
        }
        if report["knifeEvidenceStale"] == true || !self.result_current {
            return;
        }
        if let Some(samples) = report["knifeEvidence"]["samples"].as_array() {
            for pair in samples.windows(2) {
                let a = &pair[0];
                let b = &pair[1];
                let Some(index) = a["motionIndex"].as_u64().map(|i| i as usize) else {
                    continue;
                };
                if index >= end
                    || b["motionIndex"] != a["motionIndex"]
                    || a["atStart"] != true
                    || b["atStart"] != false
                {
                    continue;
                }
                let Some(m) = motions.get(index) else {
                    continue;
                };
                if m["contact"] != true {
                    continue;
                }
                if let (Some(a), Some(b)) = (
                    point(&a["replayedTipMm"], m["start"][2].as_f64().unwrap_or(0.)),
                    point(&b["replayedTipMm"], m["end"][2].as_f64().unwrap_or(0.)),
                ) {
                    line(a, b, [0.35, 1., 0.35, 1.]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blade_traces_are_visible_above_intact_stock_and_stale_replay_disappears() {
        use crate::session::{self, Command};
        let job = include_str!("../../../fixtures/gui6/knife.job.json").to_owned();
        let mut service = cam_service::retained::Retained::new();
        let (meta, payload) =
            session::execute(&mut service, Command::Generate { job: job.clone() }).unwrap();
        let handle = meta.report["gui2"]["handle"].as_str().unwrap().to_owned();
        let mut view = Viewport::default();
        view.load_scene(Ok((meta, payload)));
        view.result_current = true;
        let mut planned = overlay::Overlay::default();
        view.knife_overlay(&mut planned);
        assert!(planned.lines.iter().any(|v| v.color == [1., 0.62, 0.2, 1.]));
        assert!(planned.lines.iter().all(|v| v.position[2] > 0.));
        let (prepared, _) =
            session::execute(&mut service, Command::Prepare { job, handle }).unwrap();
        let report = &prepared.report["gui2"];
        view.accept_knife_evidence(
            &report["bundle"]["report"]["knifeEvidence"],
            &report["file"]["sha256"],
        );
        let mut emitted = overlay::Overlay::default();
        view.knife_overlay(&mut emitted);
        assert!(
            emitted
                .lines
                .iter()
                .any(|v| v.color == [0.35, 1., 0.35, 1.])
        );
        view.stale_knife_evidence();
        let mut stale = overlay::Overlay::default();
        view.knife_overlay(&mut stale);
        assert!(!stale.lines.iter().any(|v| v.color == [0.35, 1., 0.35, 1.]));
    }
}
