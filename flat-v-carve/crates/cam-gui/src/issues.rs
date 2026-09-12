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
    // Address the operation the path actually names, not just the first one.
    let operation = job
        .operations
        .iter()
        .map(|o| o.id.as_str())
        .find(|id| path.starts_with(&format!("operations[{id}].")))
        .or_else(|| job.operations.first().map(|o| o.id.as_str()))
        .unwrap_or("");
    let local = path
        .strip_prefix(&format!("operations[{operation}]."))
        .unwrap_or(path);
    if path == format!("operations[{operation}]") {
        // The whole-operation requirement (for example an XY work-zero
        // selection that needs physical stock dimensions) lives in Setup.
        return Some((1, "Stock width".into()));
    }
    if crate::session::kind(job, operation) == Some(crate::session::OperationKind::Face) {
        return match local {
            "stepdown_mm" => Some((2, "Stepdown".into())),
            "stepover_mm" => Some((2, "Stepover".into())),
            "pass_angle_deg" => Some((2, "Face pass angle".into())),
            "entry_overrun_mm" => Some((2, "Face entry overrun".into())),
            "exit_overrun_mm" => Some((2, "Face exit overrun".into())),
            "margins.min_x_mm" => Some((2, "Face margin min X".into())),
            "margins.max_x_mm" => Some((2, "Face margin max X".into())),
            "margins.min_y_mm" => Some((2, "Face margin min Y".into())),
            "margins.max_y_mm" => Some((2, "Face margin max Y".into())),
            "top.offset_mm" => Some((2, "Face top offset".into())),
            "bottom.offset_mm" => Some((2, "Face bottom offset".into())),
            "assignment.cutting_feed_mm_min" => Some((2, "Roughing feed".into())),
            "assignment.plunge_feed_mm_min" => Some((2, "Plunge feed".into())),
            "assignment.spindle_rpm" => Some((2, "Spindle speed".into())),
            "assignment.max_stepdown_mm" => Some((2, "Tool stepdown limit".into())),
            "assignment.spindle_direction" => Some((2, "Face CW".into())),
            // The assignment's tool geometry is entered in the Face tool group.
            "assignment.tool" | "assignment.tool_id" => Some((2, "Endmill diameter".into())),
            "setup.stock.thickness_mm" => Some((1, "Stock thickness".into())),
            "setup.clearance_above_stock_mm" => Some((1, "Clearance".into())),
            "setup.stock.xy" => Some((1, "Stock width".into())),
            "tolerances.motion_tolerance_mm" => Some((7, "Motion tolerance".into())),
            "tolerances.verification_tolerance_mm" => Some((7, "Verification tolerance".into())),
            _ => None,
        };
    }
    if local.starts_with("chains[") {
        return Some((2, "Unresolved knife selections".into()));
    }
    if local.starts_with("components[") {
        return Some((2, "Unresolved selections".into()));
    }
    if crate::session::kind(job, operation) == Some(crate::session::OperationKind::Profile) {
        return match local {
            "stepdown_mm" => Some((2, "Stepdown".into())),
            "through_cut_allowance_mm" => Some((2, "Through-cut allowance".into())),
            "top.offset_mm" => Some((2, "Profile top offset".into())),
            "bottom.offset_mm" => Some((2, "Profile bottom offset".into())),
            "direction" => Some((2, "Cut direction unset".into())),
            "order" => Some((2, "Order: Inner before outer".into())),
            "finish.enabled" | "finish" => Some((2, "Add radial finishing".into())),
            "finish.radial_allowance_mm" => Some((2, "Finish allowance".into())),
            "finish.feed_mm_min" => Some((2, "Finish feed".into())),
            "entry" | "entry.max_angle_deg" => Some((2, "Entry ramp angle".into())),
            "entry.feed_mm_min" => Some((2, "Entry ramp feed".into())),
            "lead_in.length_mm" => Some((2, "Lead-in length".into())),
            "lead_in.feed_mm_min" => Some((2, "Lead-in feed".into())),
            "lead_in.radius_mm" => Some((2, "Lead-in radius".into())),
            "lead_in.sweep_deg" => Some((2, "Lead-in sweep".into())),
            "lead_out.length_mm" => Some((2, "Lead-out length".into())),
            "lead_out.feed_mm_min" => Some((2, "Lead-out feed".into())),
            "lead_out.radius_mm" => Some((2, "Lead-out radius".into())),
            "lead_out.sweep_deg" => Some((2, "Lead-out sweep".into())),
            "start" => Some((2, "Profile start".into())),
            "tabs" => Some((2, "Add tabs".into())),
            "tabs.height_mm" => Some((2, "Tab height".into())),
            "tabs.width_mm" => Some((2, "Tab width".into())),
            "tabs.placement" => Some((2, "Tab placement".into())),
            "assignment.cutting_feed_mm_min" => Some((2, "Roughing feed".into())),
            "assignment.plunge_feed_mm_min" => Some((2, "Plunge feed".into())),
            "assignment.spindle_rpm" => Some((2, "Spindle speed".into())),
            "assignment.max_stepdown_mm" => Some((2, "Tool stepdown limit".into())),
            "assignment.spindle_direction" => Some((2, "Profile CW".into())),
            // The assignment's tool geometry is entered in the tool group.
            "assignment.tool" | "assignment.tool_id" => Some((2, "Endmill diameter".into())),
            "setup.stock.thickness_mm" => Some((1, "Stock thickness".into())),
            "setup.clearance_above_stock_mm" => Some((1, "Clearance".into())),
            "setup.stock.xy" => Some((1, "Stock width".into())),
            "tolerances.motion_tolerance_mm" => Some((7, "Motion tolerance".into())),
            "tolerances.verification_tolerance_mm" => Some((7, "Verification tolerance".into())),
            _ if local.starts_with("contours[") => Some((2, "Unresolved profile contours".into())),
            "contours" | "artwork" => Some((2, "Select all profile contours".into())),
            _ if local.starts_with("tools[") && local.contains("ramp_capable") => {
                Some((2, "Ramp yes".into()))
            }
            _ => None,
        };
    }
    let field = match local {
        "assignment.tool" | "assignment.tool_id" => 61,
        "assignment.cutting_feed_mm_min" => 63,
        "assignment.plunge_feed_mm_min" => 64,
        "assignment.swivel_feed_mm_min" => 65,
        "assignment.max_stepdown_mm" => 66,
        "stepdown_mm" => 67,
        "swivel_depth_mm" => 68,
        "corner_threshold_deg" => 69,
        "alignment.initial_heading_deg" => 72,
        "setup.stock.xy" => 42,
        "chains" | "artwork" => return Some((2, "Select all knife chains".into())),
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
        "component_ids" | "components" => return Some((2, "Select all filled components".into())),
        "rough" => return Some((2, "Depth-dependent clearing".into())),
        "finish" => return Some((2, "Combined".into())),
        "endmill.plunge_capable" => return Some((2, "Plunge yes".into())),
        "endmill.spindle_direction" => return Some((2, "Endmill CW".into())),
        "vbit.spindle_direction" => return Some((2, "V-bit CW".into())),
        "endmill.ramp_capable" => return Some((2, "Ramp yes".into())),
        "vbit.plunge_capable" => return Some((2, "V-bit plunge yes".into())),
        "endmill.tool_id" => return Some((2, "Change endmill tool".into())),
        "vbit.tool_id" => return Some((2, "Change V-bit tool".into())),
        _ => return None,
    };
    let tab = if matches!(field, 23 | 25) {
        7
    } else if matches!(field, 6 | 7 | 30 | 40..=43) {
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

    #[test]
    fn selection_issues_route_to_the_operation_geometry_group() {
        let mut job = crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        let operation = job.operations[0].id.clone();
        assert_eq!(
            target(&job, &format!("operations[{operation}].components[0]")),
            Some((2, "Unresolved selections".into()))
        );
        assert_eq!(
            target(&job, &format!("operations[{operation}].components")),
            Some((2, "Select all filled components".into()))
        );
        // The knife panel owns its own list inside the same operation tab.
        job.operations[0].settings = crate::knife::import_svg(
            "chains.svg".into(),
            include_str!("../../../fixtures/gui6/chains.svg").into(),
        )
        .unwrap()
        .operations[0]
            .settings
            .clone();
        assert_eq!(
            target(&job, &format!("operations[{operation}].chains[0]")),
            Some((2, "Unresolved knife selections".into()))
        );
        assert_eq!(
            target(&job, &format!("operations[{operation}].chains")),
            Some((2, "Select all knife chains".into()))
        );
        assert_eq!(
            target(&job, &format!("operations[{operation}].artwork")),
            Some((2, "Select all knife chains".into()))
        );
    }

    #[test]
    fn face_planner_fields_route_to_the_face_editor() {
        let empty = crate::operation_authoring::empty_job();
        let job = crate::operation_authoring::apply(
            &empty,
            crate::operation_authoring::add(crate::operation_authoring::Kind::Face, &empty),
        )
        .unwrap();
        let id = job.operations[0].id.clone();
        let issues = cam_core::project::v5::inspection::inspect_face_fields(&job, &id).unwrap();
        assert!(issues.len() > 5, "{issues:?}");
        for issue in issues {
            let path = issue.field_path.as_deref().unwrap();
            let (tab, label) = target(&job, path)
                .unwrap_or_else(|| panic!("no destination for {path} ({})", issue.message));
            assert!(
                matches!(
                    label.as_str(),
                    "Stock thickness"
                        | "Clearance"
                        | "Stock width"
                        | "Motion tolerance"
                        | "Verification tolerance"
                ) || FIELDS.contains(&label.as_str()),
                "{path} → {label} (tab {tab})"
            );
        }
    }

    #[test]
    fn profile_planner_fields_route_to_the_profile_editor() {
        let job = crate::profile::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap();
        let id = job.operations[0].id.clone();
        let rows = crate::profile::contours(&job)
            .unwrap()
            .into_iter()
            .map(|contour| crate::profile::SelectionRow {
                reference: contour.reference,
                side: contour.suggested_side,
                traversal: None,
            })
            .collect::<Vec<_>>();
        let job = crate::profile::select_in(&job, &id, &rows).unwrap();
        let issues = cam_core::project::v5::inspection::inspect_profile_fields(&job, &id).unwrap();
        assert!(issues.len() > 5, "{issues:?}");
        for issue in issues {
            let path = issue.field_path.as_deref().unwrap();
            let (tab, label) = target(&job, path)
                .unwrap_or_else(|| panic!("no destination for {path} ({})", issue.message));
            assert!(
                matches!(
                    label.as_str(),
                    "Stock thickness"
                        | "Clearance"
                        | "Stock width"
                        | "Motion tolerance"
                        | "Verification tolerance"
                        | "Ramp yes"
                        | "Cut direction unset"
                        | "Select all profile contours"
                        | "Unresolved profile contours"
                        | "Add tabs"
                        | "Add radial finishing"
                        | "Profile start"
                        | "Profile CW"
                ) || FIELDS.contains(&label.as_str()),
                "{path} → {label} (tab {tab})"
            );
        }
        assert_eq!(
            target(&job, &format!("operations[{id}].contours")),
            Some((2, "Select all profile contours".into()))
        );
        assert_eq!(
            target(&job, &format!("operations[{id}].stepdown_mm")),
            Some((2, "Stepdown".into()))
        );
        assert_eq!(
            target(&job, &format!("operations[{id}].direction")),
            Some((2, "Cut direction unset".into()))
        );
    }
}
