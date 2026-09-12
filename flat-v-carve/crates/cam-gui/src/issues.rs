use super::*;

/// Exact typed field paths emitted by the core, never message matching.
fn target(job: &CamJobV5, path: &str) -> Option<(usize, String)> {
    if let Some(label) = path.strip_prefix("editor.fields.") {
        let field = FIELDS.iter().position(|f| *f == label)?;
        let tab = match field {
            26..=29 => 0,
            6 | 7 | 30 | 31 | 40..=45 => 1,
            23 | 25 => 7,
            32 | 33 | 38 | 39 => 3,
            _ => 2,
        };
        return Some((tab, label.into()));
    }
    let operation = &job.operations[0].id;
    let local = path
        .strip_prefix(&format!("operations[{operation}]."))
        .unwrap_or(path);
    if local.starts_with("components[") {
        return Some((0, "Unresolved assignments".into()));
    }
    let field = match local {
        "max_depth_mm" => 0,
        "wall_allowance_mm" => 1,
        "endmill.cutting_feed_mm_min" => 2,
        "vbit.cutting_feed_mm_min" => 3,
        "max_floor_ridge_mm" => 4,
        "max_detail_residual_mm" => 5,
        "setup.stock.thickness_mm" => 6,
        "setup.clearance_above_stock_mm" => 7,
        "endmill.max_stepdown_mm" => 8,
        "endmill.stepover_mm" => 9,
        "endmill.plunge_feed_mm_min" => 10,
        "endmill.spindle_rpm" => 11,
        "tools[endmill].geometry" => 12,
        "tools[vbit].geometry" => 16,
        "vbit.max_stepdown_mm" => 20,
        "vbit.plunge_feed_mm_min" => 21,
        "vbit.spindle_rpm" => 22,
        "tolerances.motion_tolerance_mm" => 23,
        "tolerances.verification_tolerance_mm" => 25,
        "setup.start_xy_mm" => 30,
        "vbit.stepover_mm" => 46,
        "top" => 47,
        "component_ids" | "components" => return Some((0, "Select all filled components".into())),
        "rough" => return Some((2, "Depth-dependent clearing".into())),
        "finish" => return Some((2, "Combined".into())),
        "endmill.plunge_capable" => return Some((2, "Plunge yes".into())),
        "endmill.spindle_direction" => return Some((2, "Endmill CW".into())),
        "vbit.spindle_direction" => return Some((2, "V-bit CW".into())),
        "endmill.ramp_capable" => return Some((2, "Ramp yes".into())),
        "vbit.plunge_capable" => return Some((2, "V-bit plunge yes".into())),
        "endmill.tool_id" => return Some((2, "Endmill assignment tool".into())),
        "vbit.tool_id" => return Some((2, "V-bit assignment tool".into())),
        _ => return None,
    };
    let tab = if matches!(field, 23 | 25) {
        7
    } else if matches!(field, 6 | 7 | 30) {
        1
    } else {
        2
    };
    Some((tab, FIELDS[field].into()))
}

impl App {
    pub(super) fn issue_panel(&mut self, ctx: &egui::Context) {
        if self.issues.is_empty() {
            return;
        }
        egui::TopBottomPanel::bottom("machining-issues").show(ctx, |ui| {
            ui.strong("Settings need attention");
            egui::ScrollArea::vertical()
                .max_height(100.)
                .show(ui, |ui| {
                    for (index, issue) in self.issues.clone().iter().enumerate() {
                        let destination = self.document.as_ref().and_then(|d| {
                            issue.field_path.as_deref().and_then(|p| target(&d.job, p))
                        });
                        if let Some((tab, label)) = destination {
                            let r = ui.add(
                                egui::Label::new(
                                    RichText::new(format!("{label}: {}", issue.message))
                                        .color(ui.visuals().hyperlink_color),
                                )
                                .wrap()
                                .sense(egui::Sense::click()),
                            );
                            observe_control(&format!("Issue {index}"), r.rect);
                            if r.clicked() {
                                self.navigate(tab);
                                self.search.clear();
                                self.issue_focus = Some(label);
                            }
                        } else {
                            ui.label(&issue.message);
                        }
                    }
                });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_planner_missing_field_has_a_typed_destination() {
        let mut job = crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        crate::authoring::set_mode(&mut job, cam_core::project::FlatVcarveMode::Combined);
        let issues = cam_core::project::v5::inspection::inspect_flat_vcarve_fields(
            &job,
            &job.operations[0].id,
        )
        .unwrap();
        assert!(issues.len() > 15);
        for mut issue in issues {
            issue.message = "This text deliberately contains no owner information".into();
            assert!(
                target(&job, issue.field_path.as_deref().unwrap()).is_some(),
                "{:?}",
                issue
            );
        }
        assert!(target(&job, "operations[unrelated].endmill.spindle_rpm").is_none());
    }
}
