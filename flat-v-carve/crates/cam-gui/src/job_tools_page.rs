//! Job-wide physical cutters, followed by their assignment-specific settings.
use super::*;
use crate::{ui_theme as theme, ui_widgets};
use cam_core::project::{ToolGeometry, v5::OperationSettingsV5};

fn cell(ui: &mut egui::Ui, value: impl Into<String>) {
    let value = value.into();
    ui.add_sized(
        [ui.available_width(), 28.],
        egui::Label::new(&value).truncate(),
    )
    .on_hover_text(value);
}

fn geometry(tool: &cam_core::project::v5::JobToolV5) -> String {
    match &tool.geometry {
        Some(ToolGeometry::Endmill(g)) => format!("Endmill · Ø {} mm", g.diameter_mm),
        Some(ToolGeometry::Vbit(g)) => format!(
            "V-bit · {}° · Ø {} mm",
            g.included_angle_deg, g.max_cutting_diameter_mm
        ),
        Some(ToolGeometry::DragKnife(g)) => format!("Knife · {} mm offset", g.blade_offset_mm),
        Some(ToolGeometry::Drill(g)) => {
            format!("Drill · Ø {} mm · {}°", g.diameter_mm, g.tip_angle_deg)
        }
        None => "Never configured".into(),
    }
}

fn cutting_values(job: &CamJobV5, operation: &str, role: Role) -> (String, String) {
    let number = |value: Option<f64>| value.map_or_else(|| "Unset".into(), |v| v.to_string());
    let Some(op) = job.operations.iter().find(|op| op.id == operation) else {
        return ("Missing".into(), "Missing".into());
    };
    let assignment = match &op.settings {
        OperationSettingsV5::FlatVcarve(s) => {
            if role == Role::Vbit {
                &s.vbit
            } else {
                &s.endmill
            }
        }
        OperationSettingsV5::Face(s) => &s.assignment,
        OperationSettingsV5::Profile(s) => &s.assignment,
        OperationSettingsV5::Drill(s) => &s.assignment,
        OperationSettingsV5::DragKnife(s) => {
            return (number(s.assignment.cutting_feed_mm_min), "Off".into());
        }
    };
    (
        number(assignment.cutting_feed_mm_min),
        number(assignment.spindle_rpm),
    )
}

impl App {
    pub(super) fn job_tools_window(&mut self, ctx: &egui::Context) {
        if !self.resources.ready && !self.resources.busy && self.resources.error.is_none() {
            self.request_resources(ResourceIntent::Load, ctx);
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::PANEL).inner_margin(16.))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("Job tools & assignments");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if button(ui, "Close job tools", true).clicked() { self.close_resource(); }
                    });
                });
                ui.small("This job · physical geometry is shared; cutting values belong to each assignment.");
                ui.separator();
                let area = egui::ScrollArea::vertical().id_salt("job-tool-fields")
                    .auto_shrink([false, false]).show(ui, |ui| self.job_tools_content(ui, ctx));
                observe_control("Job tools viewport", area.inner_rect);
                observe_control("Job tools page", ui.max_rect());
            });
    }

    fn job_tools_content(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            ui.label("Open a job to view its copied cutters.");
            return;
        };
        let statuses = core::assignment_statuses(&job);
        if !job.tools.iter().any(|t| t.id == self.resources.job_tool) {
            self.resources.job_tool = job.tools.first().map(|t| t.id.clone()).unwrap_or_default();
        }
        ui_widgets::table_header(ui, &["Tool", "Geometry", "Usage", "Controller mapping"]);
        for tool in &job.tools {
            let users = statuses.iter().filter(|s| s.tool_id == tool.id).count();
            ui.columns(4, |cols| {
                let r = cols[0].add_sized(
                    [cols[0].available_width(), 32.],
                    egui::Button::new(&tool.name)
                        .selected(self.resources.job_tool == tool.id)
                        .truncate(),
                );
                observe_control(&format!("Job tool {}", tool.id), r.rect);
                if r.clicked() {
                    self.resources.job_tool = tool.id.clone();
                    self.resources.job_assignment = None;
                }
                cell(&mut cols[1], geometry(tool));
                cell(
                    &mut cols[2],
                    if users == 0 {
                        "Unused".into()
                    } else if users == 1 {
                        "1 assignment".into()
                    } else {
                        format!("{users} assignments")
                    },
                );
                let mappings: Vec<_> = job
                    .machine_configuration
                    .as_ref()
                    .into_iter()
                    .flat_map(|m| &m.tools)
                    .filter(|m| m.job_tool_id == tool.id)
                    .map(|m| {
                        format!(
                            "{} / {}",
                            m.tool_number
                                .map_or_else(|| "T unset".into(), |n| format!("T{n}")),
                            m.length_offset_number
                                .map_or_else(|| "H unset".into(), |n| format!("H{n}"))
                        )
                    })
                    .collect();
                let label = if mappings.is_empty() {
                    "Not mapped".into()
                } else {
                    mappings.join(", ")
                };
                let r = cols[3].link(label);
                observe_control(&format!("Job tool mapping {}", tool.id), r.rect);
                if r.clicked() {
                    self.navigate(3);
                }
            });
            ui.separator();
        }
        let Some(tool) = job.tools.iter().find(|t| t.id == self.resources.job_tool) else {
            ui.label("No tools in this job. Choose a cutter from an operation or add geometry from the library.");
            if button(ui, "Open tool library", true).clicked() {
                self.open_resource(ResourcePage::ToolLibrary);
            }
            return;
        };
        ui_widgets::section(ui, &format!("{} · assignments", tool.name), None);
        let users: Vec<_> = statuses.iter().filter(|s| s.tool_id == tool.id).collect();
        if self
            .resources
            .job_assignment
            .as_ref()
            .is_none_or(|(operation, role)| {
                !users
                    .iter()
                    .any(|s| &s.operation_id == operation && s.role == *role)
            })
        {
            self.resources.job_assignment = users.first().map(|s| (s.operation_id.clone(), s.role));
        }
        ui_widgets::table_header(
            ui,
            &[
                "Operation / stage",
                "Profile",
                "Feed · mm/min",
                "Spindle · RPM",
                "Status",
            ],
        );
        for status in &users {
            let (ordinal, op) = job
                .operations
                .iter()
                .enumerate()
                .find(|(_, op)| op.id == status.operation_id)
                .unwrap();
            let target = (status.operation_id.clone(), status.role);
            let values = cutting_values(&job, &status.operation_id, status.role);
            ui.columns(5, |cols| {
                let label = format!(
                    "{:02} · {} · {}",
                    ordinal + 1,
                    op.name,
                    crate::resources::role_word(status.role)
                );
                let r = cols[0]
                    .add_sized(
                        [cols[0].available_width(), 32.],
                        egui::Button::new(&label)
                            .selected(self.resources.job_assignment.as_ref() == Some(&target))
                            .truncate(),
                    )
                    .on_hover_text(label);
                observe_control(
                    &format!(
                        "Job assignment {} {}",
                        status.operation_id,
                        crate::resources::role_word(status.role)
                    ),
                    r.rect,
                );
                if r.clicked() {
                    self.resources.job_assignment = Some(target);
                }
                cell(
                    &mut cols[1],
                    status
                        .applied
                        .as_ref()
                        .map_or("Custom", |p| p.name_at_application.as_str()),
                );
                cell(&mut cols[2], values.0);
                cell(&mut cols[3], values.1);
                cell(&mut cols[4], format!("{:?}", status.status));
            });
        }
        if users.is_empty() {
            ui.label("Unused cutter · no operation currently refers to it.");
        }
        if let Some((operation, role)) = self.resources.job_assignment.clone() {
            let number =
                |value: Option<f64>| value.map_or_else(|| "Unset".into(), |v| v.to_string());
            ui.add_space(8.);
            let title = job
                .operations
                .iter()
                .enumerate()
                .find(|(_, op)| op.id == operation)
                .map(|(i, op)| {
                    format!(
                        "{:02} · {} / {}",
                        i + 1,
                        op.name,
                        crate::resources::role_word(role)
                    )
                })
                .unwrap_or_else(|| "Selected assignment".into());
            ui.strong(title);
            if let Some(assigned) = crate::resources::AssignedTool::of(&job, &operation, role) {
                ui.label(format!(
                    "{} · {}",
                    assigned.tool_label(),
                    assigned.profile_label()
                ));
            }
            if role == Role::Knife {
                if let Some(s) = crate::knife::settings_in(&job, &operation) {
                    let a = &s.assignment;
                    ui.label(format!(
                        "Cutting {} · plunge {} · swivel {} mm/min · stepdown {} mm",
                        number(a.cutting_feed_mm_min),
                        number(a.plunge_feed_mm_min),
                        number(a.swivel_feed_mm_min),
                        number(a.max_stepdown_mm)
                    ));
                }
            } else if let Ok(p) = crate::resources::capture_assignment_in(
                &job,
                &operation,
                role,
                "review".into(),
                "Review".into(),
            ) {
                ui.label(format!(
                    "Plunge {} mm/min · stepdown {} mm · stepover {} mm",
                    number(p.plunge_feed_mm_min),
                    number(p.max_stepdown_mm),
                    number(p.stepover_mm)
                ));
            }
            ui.horizontal_wrapped(|ui| {
                if button(ui, "Open operation", true).clicked() {
                    self.select_operation(&operation, ctx);
                }
                let reviewed = self.resources.base.as_ref().filter(|_| {
                    !self.resources.dirty
                        && self.resources.invalid.is_empty()
                        && !self.resources.busy
                });
                let can_reapply = users
                    .iter()
                    .any(|s| s.operation_id == operation && s.role == role && s.applied.is_some());
                if button(
                    ui,
                    "Reapply reviewed profile",
                    reviewed.is_some() && can_reapply && self.active.is_none(),
                )
                .clicked()
                {
                    self.resource_command(
                        R::Reapply {
                            operation,
                            role,
                            catalog: reviewed.unwrap().snapshot.clone(),
                        },
                        ctx,
                    );
                }
            });
        }
        ui_widgets::section(ui, "Copied geometry", None);
        if let Some(origin) = &tool.library_origin {
            ui.small(format!(
                "Copied from {} / {} · revision {}",
                origin.library_id, origin.tool_id, origin.copied_revision
            ));
        }
        let affected: Vec<_> = users
            .iter()
            .map(|s| {
                let op = job
                    .operations
                    .iter()
                    .find(|op| op.id == s.operation_id)
                    .unwrap();
                format!("{} / {}", op.name, crate::resources::role_word(s.role))
            })
            .collect();
        ui.label(if affected.is_empty() {
            "Geometry changes affect this unused cutter only.".into()
        } else {
            format!(
                "Geometry changes affect all {} assignments: {}.",
                affected.len(),
                affected.join(", ")
            )
        });
        self.resource_role(ui);
        ui.horizontal_wrapped(|ui| {
            let operation = self
                .document
                .as_ref()
                .and_then(|d| d.active_operation())
                .map(|op| op.id.clone());
            if button(
                ui,
                "Use tool in assignment",
                self.active.is_none() && operation.is_some(),
            )
            .clicked()
            {
                self.resource_command(
                    R::UseTool {
                        operation: operation.unwrap(),
                        role: self.resources.role,
                        tool: tool.id.clone(),
                    },
                    ctx,
                );
            }
            if button(ui, "Edit copied geometry", true).clicked() {
                match crate::resources::capture_tool(tool, tool.id.clone(), tool.name.clone()) {
                    Ok(t) => {
                        self.resources.job_tool_draft = Some((self.revision, t));
                        self.resources.job_raw.clear();
                        self.resources.job_invalid.clear();
                    }
                    Err(e) => self.status = e,
                }
            }
        });
        if let Some((revision, mut draft)) = self
            .resources
            .job_tool_draft
            .clone()
            .filter(|(_, draft)| draft.id == tool.id)
        {
            let mut form = crate::resources::Editor {
                raw: std::mem::take(&mut self.resources.job_raw),
                invalid: std::mem::take(&mut self.resources.job_invalid),
                ..Default::default()
            };
            ui.push_id(draft.id.clone(), |ui| {
                tool_form(ui, &mut draft, &mut form, "Copied")
            });
            self.resources.job_raw = form.raw;
            self.resources.job_invalid = form.invalid;
            self.resources.job_tool_draft = Some((revision, draft.clone()));
            let ready = revision == self.revision
                && self.active.is_none()
                && self.resources.job_invalid.is_empty();
            if revision != self.revision {
                ui.colored_label(
                    theme::ERROR,
                    "Job changed. Choose Edit copied geometry again to review the current tool.",
                );
            }
            ui.horizontal(|ui| {
                if button(ui, "Apply copied geometry", ready).clicked() {
                    let mut target = tool.clone();
                    target.name = draft.name;
                    target.assembly = draft.assembly;
                    target.capabilities.plunge_capable = draft.plunge_capable;
                    target.capabilities.ramp_capable = draft.ramp_capable;
                    target.geometry = Some(match draft.geometry {
                        LibraryGeometry::Endmill(g) => {
                            ToolGeometry::Endmill(cam_core::project::EndmillGeometry {
                                diameter_mm: g.diameter_mm,
                                cutting_length_mm: g.cutting_length_mm,
                            })
                        }
                        LibraryGeometry::Vbit(g) => ToolGeometry::Vbit(g),
                        LibraryGeometry::DragKnife(g) => ToolGeometry::DragKnife(g),
                        LibraryGeometry::Drill(g) => ToolGeometry::Drill(g),
                    });
                    self.resource_command(R::EditTool { tool: target }, ctx);
                }
                if button(ui, "Cancel geometry edit", true).clicked() {
                    self.resources.job_tool_draft = None;
                }
            });
        }
    }
}
