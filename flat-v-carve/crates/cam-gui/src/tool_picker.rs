//! One cutter picker for every operation.
//!
//! A Flat V-carve operation's two stage assignments, a Face or Profile
//! operation's single `Milling` assignment and a drag knife's `Knife`
//! assignment all address the same thing: a job-tool snapshot, copied cutting
//! values that may carry a library provenance and a baseline, and the loaded
//! tool library. This module renders that one picker for all of them — the
//! words differ per operation, the layout, the commands and the
//! applied/modified/reset meaning do not.
//!
//! The picker emits document commands only: choosing a job tool, applying a
//! library tool or cutting profile, resetting to the copied baseline, clearing
//! the copied values, or opening the library editor for this assignment.
use super::*;
use crate::resources::ResourceCommand as R;
use cam_core::project::ToolGeometry;
use cam_core::project::v5::resources::{self as core, AssignmentRole as Role, ProfileStatus};
use cam_core::tool_library::LibraryGeometry;

/// One assignment's cutter, with the words its editor uses. The probes are the
/// stable names the framework tests and the browser tours address.
pub(super) struct Cutter {
    pub operation: String,
    pub role: Role,
    /// Heading shown next to the job tool's name ("Endmill", "Cutter", "Knife").
    pub heading: &'static str,
    /// Prefix of the library picker's probes ("Roughing", "Finish", "Profile",
    /// "Face", "Knife").
    pub prefix: &'static str,
    /// Probe of the job-tool combo.
    pub assignment_probe: String,
    /// Probe of the button that reveals the picker.
    pub change_probe: String,
    /// Probe of the job-tool combo's per-tool rows.
    pub assign_row_probe: String,
    /// Label and probe of the "clear the copied values" action.
    pub clear_label: String,
    /// Label and probe of the baseline reset action.
    pub reset_label: String,
    /// Probe of the button that opens the library editor for this assignment.
    pub profiles_probe: String,
    /// Probe of the menu that holds the baseline actions.
    pub actions_probe: String,
}

impl Cutter {
    fn new(operation: &str, role: Role, section: &'static str, noun: &'static str) -> Self {
        Self {
            operation: operation.into(),
            role,
            heading: noun,
            prefix: section,
            assignment_probe: format!("{section} assignment tool"),
            change_probe: format!("Change {} tool", noun.to_lowercase()),
            assign_row_probe: format!("Assign {} ", noun.to_lowercase()),
            clear_label: format!("Clear {} cutting values", noun.to_lowercase()),
            reset_label: format!("Reset {} overrides", section.to_lowercase()),
            profiles_probe: format!("{section} profiles"),
            actions_probe: format!("{section} profile actions"),
        }
    }

    /// The Flat V-carve roughing stage.
    pub(super) fn endmill(operation: &str) -> Self {
        let mut cutter = Self::new(operation, Role::Endmill, "Roughing", "Endmill");
        // The established probe names this assignment after the cutter.
        cutter.assignment_probe = "Endmill assignment tool".into();
        cutter
    }

    /// The Flat V-carve finishing stage.
    pub(super) fn vbit(operation: &str) -> Self {
        let mut cutter = Self::new(operation, Role::Vbit, "Finish", "V-bit");
        // The established probes spell this assignment "V-bit".
        cutter.assignment_probe = "V-bit assignment tool".into();
        cutter.change_probe = "Change V-bit tool".into();
        cutter.assign_row_probe = "Assign V-bit ".into();
        cutter.clear_label = "Clear V-bit cutting values".into();
        cutter
    }

    /// The single milling assignment of a Face or Profile operation.
    pub(super) fn milling(operation: &str, word: &'static str) -> Self {
        Self::new(operation, Role::Milling, word, "Cutter")
    }

    /// A drag knife's assignment.
    pub(super) fn knife(operation: &str) -> Self {
        Self::new(operation, Role::Knife, "Knife", "Knife")
    }
}

/// A stable slot per role for the picker's own selection state and for the
/// reveal toggle, so two assignments of one operation never share them.
pub(super) fn role_index(role: Role) -> usize {
    match role {
        Role::Endmill => 0,
        Role::Vbit => 1,
        Role::Milling => 2,
        Role::Knife => 3,
    }
}

fn geometry_fits(geometry: &ToolGeometry, role: Role) -> bool {
    match role {
        Role::Endmill => matches!(geometry, ToolGeometry::Endmill(_)),
        Role::Vbit => matches!(geometry, ToolGeometry::Vbit(_)),
        Role::Milling => matches!(geometry, ToolGeometry::Endmill(_)),
        Role::Knife => matches!(geometry, ToolGeometry::DragKnife(_)),
    }
}

fn library_fits(geometry: &LibraryGeometry, role: Role) -> bool {
    match role {
        Role::Endmill | Role::Milling => matches!(geometry, LibraryGeometry::Endmill(_)),
        Role::Vbit => matches!(geometry, LibraryGeometry::Vbit(_)),
        Role::Knife => matches!(geometry, LibraryGeometry::DragKnife(_)),
    }
}

impl App {
    /// The one cutter picker. Every operation renders this same element, in
    /// this order:
    ///
    /// 1. the assignment's job tool, with the picker that changes it
    ///    (library tool + cutting profile, the job's own snapshots, clearing),
    /// 2. the copied cutting profile, its provenance, and the baseline actions.
    pub(super) fn tool_picker(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, cutter: &Cutter) {
        let Some(job) = self.document.as_ref().map(|d| d.job.clone()) else {
            return;
        };
        let Some(state) = core::assignment_statuses(&job)
            .into_iter()
            .find(|status| status.operation_id == cutter.operation && status.role == cutter.role)
        else {
            return;
        };
        let index = role_index(cutter.role);
        let tool_name = job
            .tools
            .iter()
            .find(|tool| tool.id == state.tool_id)
            .map(|tool| tool.name.clone())
            .unwrap_or_else(|| state.tool_id.clone());
        let heading = ui.strong(format!("{}: {tool_name}", cutter.heading));
        observe_control(&format!("{} picker", cutter.heading), heading.rect);
        let changing = self.operation_picker == Some(index);
        let change = button(
            ui,
            if changing {
                "Done choosing tool"
            } else {
                "Change tool…"
            },
            true,
        );
        observe_control(&cutter.change_probe, change.rect);
        if change.clicked() {
            self.operation_picker = if changing { None } else { Some(index) };
            self.resources.role = cutter.role;
        }
        if self.operation_picker == Some(index) {
            ui.scope(|ui| {
                ui.set_max_width(ui.available_width().min(350.));
                self.library_pick(ui, ctx, cutter, index);
                if button(ui, "Browse library…", self.active.is_none()).clicked() {
                    self.resources.role = cutter.role;
                    self.resources.machines_view = false;
                    self.resources.open = true;
                    if !self.resources.ready && !self.resources.busy {
                        self.request_resources(ResourceIntent::Load, ctx);
                    }
                    self.operation_picker = None;
                }
                ui.separator();
                self.job_tool_pick(ui, ctx, &job, cutter);
            });
        }
        ui.separator();
        self.cutting_profile(ui, ctx, cutter, &state);
    }

    /// The job's own tool snapshots, filtered to this assignment's role.
    fn job_tool_pick(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        job: &CamJobV5,
        cutter: &Cutter,
    ) {
        help::label(ui, "Assigned job tool");
        let Some(status) = core::assignment_statuses(job)
            .into_iter()
            .find(|status| status.operation_id == cutter.operation && status.role == cutter.role)
        else {
            return;
        };
        let mut selected = status.tool_id.clone();
        let before = selected.clone();
        let current = job
            .tools
            .iter()
            .find(|tool| tool.id == selected)
            .map(|tool| tool.name.as_str())
            .unwrap_or("Missing tool");
        let response = egui::ComboBox::from_id_salt(("assignment-tool", role_index(cutter.role)))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for tool in &job.tools {
                    // An empty snapshot is a legitimate incomplete state; a
                    // snapshot of the wrong kind is not offered.
                    let fits = tool
                        .geometry
                        .as_ref()
                        .is_none_or(|geometry| geometry_fits(geometry, cutter.role));
                    if !fits {
                        continue;
                    }
                    let r = ui.selectable_value(
                        &mut selected,
                        tool.id.clone(),
                        format!("{} · {}", tool.name, tool.id),
                    );
                    observe_control(&format!("{}{}", cutter.assign_row_probe, tool.id), r.rect);
                }
            });
        observe_control(&cutter.assignment_probe, response.response.rect);
        if selected != before && !selected.is_empty() {
            self.resource_command(
                R::UseTool {
                    operation: cutter.operation.clone(),
                    role: cutter.role,
                    tool: selected,
                },
                ctx,
            );
        }
        if button(ui, &cutter.clear_label, self.active.is_none()).clicked() {
            self.resource_command(
                R::Clear {
                    operation: cutter.operation.clone(),
                    role: cutter.role,
                },
                ctx,
            );
        }
        ui.small("Choosing another job tool clears this assignment's cutting values; enter values for the chosen cutter.");
    }

    /// The loaded library's matching tools and cutting profiles.
    fn library_pick(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        cutter: &Cutter,
        index: usize,
    ) {
        if !self.resources.picker_loaded {
            self.resources.picker_loaded = true;
            if !self.resources.ready && !self.resources.busy {
                self.request_resources(ResourceIntent::Load, ctx);
            }
        }
        help::label(ui, "From library");
        let Some(catalog) = self.resources.base.as_ref().map(|s| s.snapshot.clone()) else {
            ui.small(if self.resources.busy {
                "Loading library tools…"
            } else {
                "No saved library tools yet. Open the library to add or import tools."
            });
            return;
        };
        let tools: Vec<_> = catalog
            .library
            .tools
            .iter()
            .filter(|tool| library_fits(&tool.geometry, cutter.role))
            .collect();
        if tools.is_empty() {
            ui.small(format!(
                "The loaded library has no tool for this {} assignment.",
                cutter.heading.to_lowercase()
            ));
            return;
        }
        let prefix = cutter.prefix;
        // The combos below borrow the library's own tool list, so the chosen
        // ids live in locals until the command is emitted.
        let old = self.resources.picker_tools[index].clone();
        let mut chosen_tool = old.clone();
        let mut chosen_preset = self.resources.picker_profiles[index].clone();
        let tool_combo = egui::ComboBox::from_id_salt(("library-tool", index))
            .selected_text(
                tools
                    .iter()
                    .find(|tool| tool.id == chosen_tool)
                    .map_or("Select tool…", |tool| tool.name.as_str()),
            )
            .show_ui(ui, |ui| {
                for tool in &tools {
                    let r = ui.selectable_value(&mut chosen_tool, tool.id.clone(), &tool.name);
                    observe_control(&format!("{prefix} library tool {}", tool.id), r.rect);
                }
            });
        observe_control(&format!("{prefix} library tool"), tool_combo.response.rect);
        if chosen_tool != old {
            chosen_preset.clear();
        }
        self.resources.picker_tools[index] = chosen_tool.clone();
        self.resources.picker_profiles[index] = chosen_preset.clone();
        let Some(tool) = tools.iter().find(|tool| tool.id == chosen_tool) else {
            return;
        };
        let profile_combo = egui::ComboBox::from_id_salt(("library-profile", index))
            .selected_text(
                tool.cutting_presets
                    .iter()
                    .find(|cut| cut.id == chosen_preset)
                    .map_or("Tool only (no profile)", |cut| cut.name.as_str()),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut chosen_preset, String::new(), "Tool only (no profile)");
                for cut in &tool.cutting_presets {
                    let r = ui.selectable_value(&mut chosen_preset, cut.id.clone(), &cut.name);
                    observe_control(&format!("{prefix} library profile {}", cut.id), r.rect);
                }
            });
        observe_control(
            &format!("{prefix} library profile"),
            profile_combo.response.rect,
        );
        self.resources.picker_profiles[index] = chosen_preset.clone();
        let valid = chosen_preset.is_empty()
            || tool
                .cutting_presets
                .iter()
                .any(|cut| cut.id == chosen_preset);
        let apply = ui.add_enabled(
            valid && self.active.is_none() && self.io.is_none(),
            egui::Button::new(if chosen_preset.is_empty() {
                "Use library tool"
            } else {
                "Apply tool & profile"
            }),
        );
        observe_control(&format!("Apply {prefix} library selection"), apply.rect);
        if apply.clicked() {
            let action = if chosen_preset.is_empty() {
                R::SelectLibraryTool {
                    catalog: catalog.clone(),
                    tool: tool.id.clone(),
                    operation: cutter.operation.clone(),
                    role: cutter.role,
                }
            } else {
                R::ApplyToolProfile {
                    catalog: catalog.clone(),
                    tool: tool.id.clone(),
                    preset: chosen_preset.clone(),
                    operation: cutter.operation.clone(),
                    role: cutter.role,
                }
            };
            self.resource_command(action, ctx);
            self.operation_picker = None;
        }
        if self.resources.dirty {
            ui.small("Selections use the saved library revision. Save library edits to use the new values.");
        }
    }

    /// The copied cutting profile, its provenance and its baseline actions.
    fn cutting_profile(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        cutter: &Cutter,
        state: &core::AssignmentStatus,
    ) {
        match &state.applied {
            Some(applied) => {
                ui.label(format!("Profile: {}", applied.name_at_application));
                ui.small(match state.status {
                    ProfileStatus::Applied => "From library",
                    _ => "Modified for this job",
                });
            }
            None => {
                ui.small("Custom cutting values · no profile applied");
            }
        }
        let reviewed = self.resources.base.as_ref().map(|s| s.snapshot.clone());
        ui.horizontal_wrapped(|ui| {
            let profiles = button(ui, "Change profile…", self.active.is_none());
            observe_control(&cutter.profiles_probe, profiles.rect);
            if profiles.clicked() {
                self.resources.role = cutter.role;
                if let Some(applied) = &state.applied {
                    self.resources.tool = applied.library_tool_id.clone();
                    self.resources.preset = applied.preset_id.clone();
                }
                self.resources.machines_view = false;
                self.resources.open = true;
                if !self.resources.ready {
                    self.request_resources(ResourceIntent::Load, ctx);
                }
            }
            let menu = ui.menu_button("More…", |ui| {
                if button(
                    ui,
                    &cutter.reset_label,
                    state.status != ProfileStatus::Custom && self.active.is_none(),
                )
                .clicked()
                {
                    self.resource_command(
                        R::Reset {
                            operation: cutter.operation.clone(),
                            role: cutter.role,
                        },
                        ctx,
                    );
                    ui.close();
                }
                if button(
                    ui,
                    "Reapply reviewed profile",
                    reviewed.is_some()
                        && self.active.is_none()
                        && state.status != ProfileStatus::Custom,
                )
                .clicked()
                {
                    self.resource_command(
                        R::Reapply {
                            operation: cutter.operation.clone(),
                            role: cutter.role,
                            catalog: reviewed.clone().unwrap(),
                        },
                        ctx,
                    );
                    ui.close();
                }
            });
            observe_control(&cutter.actions_probe, menu.response.rect);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_has_its_own_picker_slot() {
        let slots = [
            role_index(Role::Endmill),
            role_index(Role::Vbit),
            role_index(Role::Milling),
            role_index(Role::Knife),
        ];
        let unique: std::collections::BTreeSet<_> = slots.iter().collect();
        assert_eq!(unique.len(), slots.len(), "one slot per role: {slots:?}");
    }

    #[test]
    fn the_cutters_keep_the_established_probe_names() {
        let endmill = Cutter::endmill("carving-1");
        assert_eq!(endmill.change_probe, "Change endmill tool");
        assert_eq!(endmill.assignment_probe, "Endmill assignment tool");
        assert_eq!(endmill.clear_label, "Clear endmill cutting values");
        assert_eq!(endmill.reset_label, "Reset roughing overrides");
        assert_eq!(endmill.profiles_probe, "Roughing profiles");
        assert_eq!(endmill.actions_probe, "Roughing profile actions");
        let vbit = Cutter::vbit("carving-1");
        assert_eq!(vbit.change_probe, "Change V-bit tool");
        assert_eq!(vbit.assignment_probe, "V-bit assignment tool");
        assert_eq!(vbit.reset_label, "Reset finish overrides");
        let profile = Cutter::milling("profile-1", "Profile");
        assert_eq!(profile.change_probe, "Change cutter tool");
        assert_eq!(profile.assignment_probe, "Profile assignment tool");
        assert_eq!(profile.reset_label, "Reset profile overrides");
        assert_eq!(profile.profiles_probe, "Profile profiles");
        let knife = Cutter::knife("knife-1");
        assert_eq!(knife.change_probe, "Change knife tool");
        assert_eq!(knife.reset_label, "Reset knife overrides");
    }

    /// The whole point of this module: every operation renders the same picker.
    #[test]
    fn every_operation_renders_the_same_picker() {
        use crate::operation_authoring::{self, Kind};

        fn app_with(kind: Option<Kind>, operation_tab: usize) -> (App, usize) {
            let empty = operation_authoring::empty_job();
            let job = match kind {
                Some(kind) => {
                    operation_authoring::apply(&empty, operation_authoring::add(kind, &empty))
                        .unwrap()
                }
                // The established carving fixture, so the Flat V-carve stages
                // have a tool and an assignment to address.
                None => {
                    let artwork = crate::authoring::import_svg(
                        "letters.svg".into(),
                        include_str!("../../../fixtures/gui3/lettering.svg").into(),
                    )
                    .unwrap();
                    crate::operation_authoring::apply(
                        &artwork,
                        crate::operation_authoring::add(
                            crate::operation_authoring::Kind::FlatVcarve,
                            &artwork,
                        ),
                    )
                    .unwrap()
                }
            };
            let index = match kind {
                None | Some(Kind::FlatVcarve) => {
                    if operation_tab == 2 {
                        role_index(Role::Vbit)
                    } else {
                        role_index(Role::Endmill)
                    }
                }
                Some(Kind::Face) | Some(Kind::Profile) => role_index(Role::Milling),
                Some(Kind::DragKnife) => role_index(Role::Knife),
            };
            (
                App {
                    document: Some(Document::new(job)),
                    inspector_tab: 2,
                    operation_tab,
                    operation_picker: Some(index),
                    ..Default::default()
                },
                index,
            )
        }

        fn render(
            app: &mut App,
            ctx: &egui::Context,
        ) -> std::collections::BTreeMap<String, [f32; 4]> {
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1200., 900.),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.inspector(ctx),
                );
            }
            CONTROLS.with(|c| c.borrow().clone())
        }

        let ctx = egui::Context::default();
        struct Case {
            kind: Option<Kind>,
            operation_tab: usize,
            noun: &'static str,
            change: &'static str,
            profiles: &'static str,
            assignment: &'static str,
        }
        let cases = [
            Case {
                kind: None,
                operation_tab: 1,
                noun: "Endmill",
                change: "Change endmill tool",
                profiles: "Roughing profiles",
                assignment: "Endmill assignment tool",
            },
            Case {
                kind: None,
                operation_tab: 2,
                noun: "V-bit",
                change: "Change V-bit tool",
                profiles: "Finish profiles",
                assignment: "V-bit assignment tool",
            },
            Case {
                kind: Some(Kind::Face),
                operation_tab: 0,
                noun: "Cutter",
                change: "Change cutter tool",
                profiles: "Face profiles",
                assignment: "Face assignment tool",
            },
            Case {
                kind: Some(Kind::Profile),
                operation_tab: 0,
                noun: "Cutter",
                change: "Change cutter tool",
                profiles: "Profile profiles",
                assignment: "Profile assignment tool",
            },
            Case {
                kind: Some(Kind::DragKnife),
                operation_tab: 0,
                noun: "Knife",
                change: "Change knife tool",
                profiles: "Knife profiles",
                assignment: "Knife assignment tool",
            },
        ];
        for Case {
            kind,
            operation_tab,
            noun,
            change,
            profiles,
            assignment,
        } in cases
        {
            let (mut app, _) = app_with(kind, operation_tab);
            let controls = render(&mut app, &ctx);
            for label in [format!("{noun} picker"), change.into(), profiles.into()] {
                assert!(
                    controls.contains_key(&label),
                    "{kind:?} is missing {label}: {:?}",
                    controls.keys().collect::<Vec<_>>()
                );
            }
            // Opening the picker shows the same revealed controls everywhere.
            for label in [assignment, "Browse library…"] {
                assert!(
                    controls.contains_key(label),
                    "{kind:?} is missing {label} once its picker is open"
                );
            }
        }
    }
}
