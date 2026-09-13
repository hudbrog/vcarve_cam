use super::*;
#[path = "knife_library.rs"]
mod knife_library;
#[path = "library_modal.rs"]
mod library_modal;
use crate::resources::{Catalog, ResourceCommand as R, StoredCatalog};
use cam_core::{
    project::v5::resources::{self as core, AssignmentRole as Role},
    tool_library::{CuttingPreset, LibraryGeometry, LibraryTool},
};

fn unique(base: &str, used: impl Iterator<Item = String>) -> String {
    let used: std::collections::BTreeSet<_> = used.collect();
    (1..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !used.contains(id))
        .unwrap()
}
fn text(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.label(field_label(label));
            help::icon(ui, label);
        });
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(ui.available_width())
                .char_limit(1000),
        );
        observe_control(label, r.rect);
        r.changed()
    })
    .inner
}
fn field_label(label: &str) -> &str {
    let short = label
        .strip_prefix("Library ")
        .or_else(|| label.strip_prefix("Copied "))
        .or_else(|| label.strip_prefix("Profile "))
        .unwrap_or(label);
    match short {
        "tool name" | "name" => "Name",
        "diameter" => "Diameter (mm)",
        "cutting length" => "Cutting length (mm)",
        "V-bit angle" => "Included angle (°)",
        "tip diameter" => "Tip diameter (mm)",
        "cutting diameter" => "Cutting diameter (mm)",
        "cutting height" => "Cutting height (mm)",
        "blade offset" => "Blade offset (mm)",
        "cut depth" => "Cut depth (mm)",
        "plunge" => "Can plunge",
        "ramp" => "Can ramp",
        "rotation" => "Rotation",
        "material" => "Material",
        "machine context" => "Machine context",
        "spindle RPM" => "Spindle speed (RPM)",
        "cutting feed" => "Cutting feed (mm/min)",
        "plunge feed" => "Plunge feed (mm/min)",
        "stepdown" => "Stepdown (mm)",
        "stepover" => "Stepover (mm)",
        "New machine ID" => "Machine name / ID",
        "Contract reference" => "Tool-change notes / reference",
        "Configuration clearance" => "Clearance (mm)",
        "Configuration precision" => "Output precision (decimals)",
        "Configuration spinup seconds" => "Spindle spin-up (s)",
        "Configuration blend tolerance" => "Blend tolerance · G64 P (mm)",
        "Configuration naive CAM tolerance" => "Line simplification · G64 Q (mm)",
        _ => short,
    }
}

fn section(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    title: &str,
    open: bool,
    contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(Color32::from_rgb(245, 248, 250))
        .stroke(egui::Stroke::new(1., Color32::from_rgb(209, 219, 225)))
        .corner_radius(5)
        .inner_margin(12.)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let r = egui::CollapsingHeader::new(RichText::new(title).size(16.).strong())
                .id_salt(id)
                .default_open(open)
                .show(ui, |ui| {
                    ui.add_space(6.);
                    contents(ui);
                });
            observe_control(title, r.header_response.rect);
        });
    ui.add_space(4.);
}

fn select<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(&str, T)],
) -> bool {
    let before = value.clone();
    ui.horizontal(|ui| {
        ui.label(field_label(label));
        help::icon(ui, label);
    });
    let name = options
        .iter()
        .find(|(_, v)| v == value)
        .map(|(name, _)| *name)
        .unwrap_or("Unset");
    let r = egui::ComboBox::from_id_salt(label)
        .selected_text(name)
        .width(ui.available_width())
        .show_ui(ui, |ui| {
            for (name, v) in options {
                let r = ui.selectable_value(value, v.clone(), *name);
                observe_control(&format!("{label} {name}"), r.rect);
            }
        });
    observe_control(label, r.response.rect);
    before != *value
}
fn number(
    ui: &mut egui::Ui,
    label: &str,
    key: &str,
    value: &mut Option<f64>,
    required: bool,
    editor: &mut crate::resources::Editor,
) {
    let mut raw = editor
        .raw
        .get(key)
        .cloned()
        .unwrap_or_else(|| value.map(|v| v.to_string()).unwrap_or_default());
    if text(ui, label, &mut raw) {
        editor.dirty = true;
        editor.raw.insert(key.into(), raw.clone());
        match Draft::parse(&raw) {
            Ok(v) if !required || v.is_some() => {
                *value = v;
                editor.invalid.remove(key);
            }
            _ => {
                editor.invalid.insert(key.into());
            }
        }
    }
    if editor.invalid.contains(key) {
        ui.colored_label(Color32::DARK_RED, "Complete this numeric value");
    }
}
fn required(
    ui: &mut egui::Ui,
    label: &str,
    key: &str,
    value: &mut f64,
    editor: &mut crate::resources::Editor,
) {
    let mut optional = Some(*value);
    number(ui, label, key, &mut optional, true, editor);
    if let Some(v) = optional {
        *value = v;
    }
}
fn tool_form(
    ui: &mut egui::Ui,
    tool: &mut LibraryTool,
    e: &mut crate::resources::Editor,
    prefix: &str,
) {
    e.dirty |= text(ui, &format!("{prefix} tool name"), &mut tool.name);
    let key = tool.id.clone();
    ui.columns(2, |cols| match &mut tool.geometry {
        LibraryGeometry::Endmill(g) => {
            required(
                &mut cols[0],
                &format!("{prefix} diameter"),
                &format!("{key}/diameter"),
                &mut g.diameter_mm,
                e,
            );
            required(
                &mut cols[1],
                &format!("{prefix} cutting length"),
                &format!("{key}/length"),
                &mut g.cutting_length_mm,
                e,
            );
        }
        LibraryGeometry::Vbit(g) => {
            required(
                &mut cols[0],
                &format!("{prefix} V-bit angle"),
                &format!("{key}/angle"),
                &mut g.included_angle_deg,
                e,
            );
            required(
                &mut cols[1],
                &format!("{prefix} tip diameter"),
                &format!("{key}/tip"),
                &mut g.tip_diameter_mm,
                e,
            );
            required(
                &mut cols[0],
                &format!("{prefix} cutting diameter"),
                &format!("{key}/diameter"),
                &mut g.max_cutting_diameter_mm,
                e,
            );
            required(
                &mut cols[1],
                &format!("{prefix} cutting height"),
                &format!("{key}/height"),
                &mut g.cutting_height_mm,
                e,
            );
        }
        LibraryGeometry::DragKnife(g) => {
            required(
                &mut cols[0],
                &format!("{prefix} blade offset"),
                &format!("{key}/offset"),
                &mut g.blade_offset_mm,
                e,
            );
            required(
                &mut cols[1],
                &format!("{prefix} cut depth"),
                &format!("{key}/depth"),
                &mut g.max_cut_depth_mm,
                e,
            );
        }
    });
    if !matches!(tool.geometry, LibraryGeometry::DragKnife(_)) {
        ui.columns(2, |cols| {
            e.dirty |= select(
                &mut cols[0],
                &format!("{prefix} plunge"),
                &mut tool.plunge_capable,
                &[("Unset", None), ("Yes", Some(true)), ("No", Some(false))],
            );
            e.dirty |= select(
                &mut cols[1],
                &format!("{prefix} ramp"),
                &mut tool.ramp_capable,
                &[("Unset", None), ("Yes", Some(true)), ("No", Some(false))],
            );
        });
    }
    if let LibraryGeometry::Endmill(g) = &mut tool.geometry
        && let Some(plunge) = tool.plunge_capable
    {
        g.plunge_capable = plunge;
    }
    if prefix == "Library" && !matches!(tool.geometry, LibraryGeometry::DragKnife(_)) {
        use cam_core::project::SpindleDirection::{Clockwise, Counterclockwise};
        e.dirty |= select(
            ui,
            &format!("{prefix} rotation"),
            &mut tool.spindle_direction,
            &[
                ("Unset", None),
                ("CW", Some(Clockwise)),
                ("CCW", Some(Counterclockwise)),
            ],
        );
    }
}
fn preset_form(
    ui: &mut egui::Ui,
    tool: &str,
    preset: &mut CuttingPreset,
    e: &mut crate::resources::Editor,
) {
    e.dirty |= text(ui, "Profile name", &mut preset.name);
    ui.columns(2, |cols| {
        for (i, (label, slot)) in [
            ("Profile material", &mut preset.material),
            ("Profile machine context", &mut preset.machine),
        ]
        .into_iter()
        .enumerate()
        {
            let mut value = slot.clone().unwrap_or_default();
            if text(&mut cols[i], label, &mut value) {
                *slot = (!value.is_empty()).then_some(value);
                e.dirty = true;
            }
        }
    });
    let mut fields = [
        ("Profile spindle RPM", "rpm", &mut preset.spindle_rpm),
        (
            "Profile cutting feed",
            "feed",
            &mut preset.cutting_feed_mm_min,
        ),
        (
            "Profile plunge feed",
            "plunge",
            &mut preset.plunge_feed_mm_min,
        ),
        ("Profile stepdown", "stepdown", &mut preset.max_stepdown_mm),
        ("Profile stepover", "stepover", &mut preset.stepover_mm),
    ];
    for row in fields.chunks_mut(2) {
        ui.columns(2, |cols| {
            for (i, (label, field, value)) in row.iter_mut().enumerate() {
                number(
                    &mut cols[i],
                    label,
                    &format!("{tool}/{}/{field}", preset.id),
                    value,
                    false,
                    e,
                );
            }
        });
    }
}
impl App {
    pub(super) fn resource_stamp(&self) -> String {
        crate::compute::hash(
            &serde_json::to_vec(&(&self.resources.draft, &self.resources.raw)).unwrap(),
        )
    }
    pub(super) fn resource_command(&mut self, action: R, ctx: &egui::Context) {
        if let Some(doc) = &self.document {
            self.submit(
                Command::Resource {
                    job: doc.job.to_json().unwrap(),
                    action: Box::new(action),
                },
                ctx,
            );
        }
    }
    pub(super) fn request_resources(&mut self, intent: ResourceIntent, ctx: &egui::Context) {
        if self.resources.busy {
            return;
        }
        self.resources.error = None;
        let save = if matches!(intent, ResourceIntent::Save) {
            if !self.resources.invalid.is_empty() {
                self.resources.status = "Complete partial library fields before Save.".into();
                self.resources.error = Some(self.resources.status.clone());
                return;
            }
            if let Err(e) = self.resources.draft.validate() {
                self.resources.status = e.clone();
                self.resources.error = Some(e);
                return;
            }
            Some((
                self.resources.base.as_ref().map(|s| s.revision),
                self.resources.draft.clone(),
            ))
        } else {
            None
        };
        let id = self.id();
        self.resource_request = Some((id, intent));
        self.resources.busy = true;
        self.port.resources(id, save, ctx.clone());
    }
    pub(super) fn accept_resources(
        &mut self,
        id: u64,
        result: Result<Option<StoredCatalog>, String>,
    ) {
        let Some((expected, intent)) = self.resource_request else {
            return;
        };
        if expected != id {
            return;
        }
        self.resource_request = None;
        self.resources.busy = false;
        let result = result.and_then(|s| {
            if let Some(stored) = &s {
                stored.validate()?;
            }
            Ok(s)
        });
        match result {
            Err(e) => {
                self.resources.status = format!("{e} Your library edits are preserved.");
                self.resources.error = Some(self.resources.status.clone());
            }
            Ok(stored) if matches!(intent, ResourceIntent::Compare) => {
                self.resources.error = None;
                self.resources.conflict = stored;
                self.resources.status =
                    "Stored revision loaded for comparison; your edits are unchanged.".into();
            }
            Ok(stored) => {
                self.resources.error = None;
                if let Some(s) = &stored {
                    self.resources.draft = s.snapshot.clone();
                }
                self.resources.base = stored;
                self.resources.ready = true;
                self.resources.dirty = false;
                self.resources.raw.clear();
                self.resources.invalid.clear();
                self.resources.conflict = None;
                self.resources.status = if matches!(intent, ResourceIntent::Save) {
                    "Library saved. Jobs keep their existing copies."
                } else {
                    "Local library loaded."
                }
                .into();
            }
        }
    }
    pub(super) fn import_resources(&mut self, json: &str) {
        match Catalog::decode(json) {
            Ok(mut catalog) => {
                self.resources.error = None;
                // Imported content is only a buffer until explicit conditional Save.
                catalog.library.revision = self.resources.base.as_ref().map_or(0, |s| s.revision);
                self.resources.draft = catalog;
                self.resources.dirty = true;
                self.resources.raw.clear();
                self.resources.invalid.clear();
                self.resources.status =
                    "Imported into the edit buffer. Review and Save library explicitly.".into();
            }
            Err(e) => {
                self.resources.status = e.clone();
                self.resources.error = Some(e);
            }
        }
    }
    pub(super) fn import_machine(&mut self, json: &str) {
        match cam_core::post::sequence::SequenceProfile::from_json(json) {
            Ok(mut machine) => {
                self.resources.error = None;
                if self
                    .resources
                    .draft
                    .machines
                    .iter()
                    .any(|m| m.id == machine.id)
                {
                    machine.id = unique(
                        "machine",
                        self.resources.draft.machines.iter().map(|m| m.id.clone()),
                    );
                }
                self.resources.machine = machine.id.clone();
                self.resources.draft.machines.push(machine);
                self.resources.dirty = true;
                self.resources.status =
                    "Machine imported into library buffer. Review and Save library.".into();
            }
            Err(e) => {
                self.resources.status = e.to_string();
                self.resources.error = Some(e.to_string());
            }
        }
    }
    pub(super) fn resource_windows(&mut self, ctx: &egui::Context) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| d.job.operations.is_empty())
        {
            self.resources.jobs_open = false;
            if self.resources.open {
                egui::Window::new("Tool library").open(&mut self.resources.open).show(ctx, |ui| { ui.label("Add an operation before choosing its tools and cutting profiles. Existing job tools are retained."); });
            }
            return;
        }
        if let Some(doc) = &self.document {
            if crate::knife::settings(&doc.job).is_some() {
                self.resources.role = Role::Knife;
            } else if self.resources.role == Role::Knife {
                self.resources.role = Role::Endmill;
            }
        }
        if self.resources.open {
            self.library_window(ctx);
        }
        if self.resources.jobs_open {
            self.job_tools_window(ctx);
        }
    }
    fn library_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let Some(index) = self
            .resources
            .draft
            .library
            .tools
            .iter()
            .position(|t| t.id == self.resources.tool)
        else {
            return;
        };
        let mut tool = self.resources.draft.library.tools[index].clone();
        section(
            ui,
            (tool.id.clone(), "geometry"),
            "Geometry & capabilities",
            true,
            |ui| {
                tool_form(ui, &mut tool, &mut self.resources, "Library");
            },
        );
        self.resources.draft.library.tools[index] = tool.clone();
        let reviewed = self
            .resources
            .base
            .as_ref()
            .filter(|_| !self.resources.dirty && self.resources.invalid.is_empty())
            .map(|s| s.snapshot.clone());
        if matches!(tool.geometry, LibraryGeometry::DragKnife(_)) {
            self.knife_library_profiles(ui, &mut tool);
        } else {
            section(
                ui,
                (tool.id.clone(), "profiles"),
                "Cutting profiles",
                true,
                |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let r = ui.selectable_value(
                            &mut self.resources.preset,
                            String::new(),
                            "Tool only",
                        );
                        observe_control("Library profile none", r.rect);
                        for p in &tool.cutting_presets {
                            let r = ui.selectable_value(
                                &mut self.resources.preset,
                                p.id.clone(),
                                &p.name,
                            );
                            observe_control(&format!("Library profile {}", p.id), r.rect);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        if button(
                            ui,
                            "New cutting profile",
                            !matches!(tool.geometry, LibraryGeometry::DragKnife(_)),
                        )
                        .clicked()
                        {
                            let id = unique(
                                "profile",
                                tool.cutting_presets.iter().map(|p| p.id.clone()),
                            );
                            tool.cutting_presets.push(CuttingPreset {
                                id: id.clone(),
                                name: "New profile".into(),
                                material: None,
                                machine: None,
                                spindle_rpm: None,
                                cutting_feed_mm_min: None,
                                plunge_feed_mm_min: None,
                                max_stepdown_mm: None,
                                stepover_mm: None,
                            });
                            self.resources.preset = id;
                            self.resources.dirty = true;
                        }
                        if button(
                            ui,
                            "Capture assignment as profile",
                            self.document.is_some()
                                && !matches!(tool.geometry, LibraryGeometry::DragKnife(_)),
                        )
                        .clicked()
                        {
                            let id = unique(
                                "profile",
                                tool.cutting_presets.iter().map(|p| p.id.clone()),
                            );
                            match crate::resources::capture_assignment(
                                &self.document.as_ref().unwrap().job,
                                self.resources.role,
                                id.clone(),
                                "Captured cutting values".into(),
                            ) {
                                Ok(p) => {
                                    tool.cutting_presets.push(p);
                                    self.resources.preset = id;
                                    self.resources.dirty = true;
                                }
                                Err(e) => self.resources.status = e,
                            }
                        }
                    });
                    if let Some(pindex) = tool
                        .cutting_presets
                        .iter()
                        .position(|p| p.id == self.resources.preset)
                    {
                        let p = &mut tool.cutting_presets[pindex];
                        ui.push_id((tool.id.clone(), p.id.clone()), |ui| {
                            preset_form(ui, &tool.id, p, &mut self.resources)
                        });
                        if button(ui, "Duplicate cutting profile", true).clicked() {
                            let mut copy = tool.cutting_presets[pindex].clone();
                            copy.id = unique(
                                "profile",
                                tool.cutting_presets.iter().map(|p| p.id.clone()),
                            );
                            copy.name.push_str(" copy");
                            self.resources.preset = copy.id.clone();
                            tool.cutting_presets.push(copy);
                            self.resources.dirty = true;
                        }
                    }
                },
            );
        }
        self.resources.draft.library.tools[index] = tool;
        section(
            ui,
            ("library-job-assignment", self.resources.role == Role::Vbit),
            "Applied job values",
            false,
            |ui| {
                if let Some(doc) = &self.document {
                    let status = core::assignment_statuses(&doc.job)
                        .into_iter()
                        .find(|s| s.role == self.resources.role)
                        .unwrap();
                    ui.label(format!(
                        "Selected assignment: {:?} · {:?}",
                        status.role, status.status
                    ));
                    if let Some(applied) = status.applied {
                        ui.small(format!(
                            "{} · copied revision {}",
                            applied.name_at_application, applied.revision_at_application
                        ));
                    }
                    let operation = doc.job.operations[0].id.clone();
                    ui.horizontal_wrapped(|ui| {
                        if button(
                            ui,
                            "Reset assignment overrides",
                            status.status != core::ProfileStatus::Custom && self.active.is_none(),
                        )
                        .clicked()
                        {
                            self.resource_command(
                                R::Reset {
                                    operation: operation.clone(),
                                    role: self.resources.role,
                                },
                                ctx,
                            );
                        }
                        if button(
                            ui,
                            "Reapply reviewed profile",
                            reviewed.is_some()
                                && self.active.is_none()
                                && status.status != core::ProfileStatus::Custom,
                        )
                        .clicked()
                        {
                            self.resource_command(
                                R::Reapply {
                                    operation,
                                    role: self.resources.role,
                                    catalog: reviewed.unwrap(),
                                },
                                ctx,
                            );
                        }
                    });
                }
            },
        );
    }
    fn resource_role(&mut self, ui: &mut egui::Ui) {
        if self
            .document
            .as_ref()
            .is_some_and(|d| crate::knife::settings(&d.job).is_some())
        {
            self.resources.role = Role::Knife;
            ui.label("Target assignment: Drag knife");
            return;
        }
        // A Face or Profile operation has one milling assignment, so there is
        // no rough/finish choice to make for it.
        if self.document.as_ref().is_some_and(|d| {
            matches!(
                crate::session::kind(&d.job, &d.raw.operation),
                Some(crate::session::OperationKind::Face | crate::session::OperationKind::Profile)
            )
        }) {
            self.resources.role = Role::Milling;
            ui.label("Target assignment: Milling (the selected operation's cutter)");
            return;
        }
        ui.horizontal(|ui| {
            ui.label("Target assignment");
            for (label, role) in [
                ("Roughing assignment", Role::Endmill),
                ("Finishing assignment", Role::Vbit),
            ] {
                let r = ui.selectable_value(&mut self.resources.role, role, label);
                observe_control(label, r.rect);
            }
        });
    }
}

fn choice<T: PartialEq + Clone>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut T,
    options: &[(&str, T)],
) -> bool {
    let before = value.clone();
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        help::icon(ui, label);
        for (name, v) in options {
            let r = ui.selectable_value(value, v.clone(), *name);
            observe_control(&format!("{label} {name}"), r.rect);
        }
    });
    before != *value
}
fn machine_form(
    ui: &mut egui::Ui,
    m: &mut cam_core::post::sequence::SequenceProfile,
    e: &mut crate::resources::Editor,
) {
    use cam_core::post::{Coolant, LengthCompensation, M6Return, PathControl};
    let key = format!("machine/{}", m.id);
    section(ui, (&key, "general"), "General", true, |ui| {
        ui.columns(2, |cols| {
            e.dirty |= select(
                &mut cols[0],
                "Work offset",
                &mut m.work_offset,
                &[
                    ("G54", "G54".into()),
                    ("G55", "G55".into()),
                    ("G56", "G56".into()),
                    ("G57", "G57".into()),
                    ("G58", "G58".into()),
                    ("G59", "G59".into()),
                    ("G59.1", "G59.1".into()),
                    ("G59.2", "G59.2".into()),
                    ("G59.3", "G59.3".into()),
                ],
            );
            required(
                &mut cols[1],
                "Configuration clearance",
                &format!("{key}/clearance"),
                &mut m.clearance_z_mm,
                e,
            );
        });
        ui.columns(2, |cols| {
            let mut precision = m.decimal_places as f64;
            required(
                &mut cols[0],
                "Configuration precision",
                &format!("{key}/precision"),
                &mut precision,
                e,
            );
            if precision.fract() == 0. && (0.0..=9.).contains(&precision) {
                m.decimal_places = precision as usize;
            } else {
                e.invalid.insert(format!("{key}/precision"));
            }
            required(
                &mut cols[1],
                "Configuration spinup seconds",
                &format!("{key}/spinup"),
                &mut m.spindle_spinup_seconds,
                e,
            );
        });
    });
    section(
        ui,
        (&key, "tool-change"),
        "Tool change & compensation",
        true,
        |ui| {
            let compensation_changed = select(
                ui,
                "Length compensation",
                &mut m.length_compensation,
                &[
                    ("Macro managed", LengthCompensation::MacroManaged),
                    ("Tool table", LengthCompensation::ToolTable),
                ],
            );
            e.dirty |= compensation_changed;
            if compensation_changed && m.length_compensation == LengthCompensation::MacroManaged {
                for row in &mut m.tools {
                    row.length_offset_number = None;
                    e.raw.remove(&format!("{key}/H/{}", row.tool_id));
                    e.invalid.remove(&format!("{key}/H/{}", row.tool_id));
                }
            }

            if m.m6.reviewed
                && m.m6.preserves_work_datum
                && m.m6.local_offsets_unused
                && m.m6.tool_offsets_z_only
                && !m.m6.reference.trim().is_empty()
            {
                ui.colored_label(Color32::from_rgb(38, 125, 90), "Reviewed");
            }
            help::label(ui, "M6 contract");
            ui.label("Tool-change behavior: describe what your controller or manual change procedure guarantees after changing and compensating the tool.");
            ui.small("Reference = your notes, manual section, or macro filename/revision. Review the guarantees below against that procedure.");
            e.dirty |= text(ui, "Contract reference", &mut m.m6.reference);
            for (label, value) in [
                ("Contract reviewed", &mut m.m6.reviewed),
                ("Preserves work datum", &mut m.m6.preserves_work_datum),
                ("Local offsets unused", &mut m.m6.local_offsets_unused),
                ("Z-only tool offsets", &mut m.m6.tool_offsets_z_only),
            ] {
                e.dirty |= ui
                    .horizontal(|ui| {
                        let r = ui.checkbox(value, label);
                        observe_control(label, r.rect);
                        help::icon(ui, label);
                        r.changed()
                    })
                    .inner;
            }
            let mut mode = match m.m6.return_position {
                M6Return::CallerPosition => 0,
                M6Return::FixedPosition { .. } => 1,
                M6Return::SafeRetract { .. } => 2,
            };
            if select(
                ui,
                "M6 return",
                &mut mode,
                &[("Caller", 0), ("Fixed", 1), ("Safe retract", 2)],
            ) {
                m.m6.return_position = match mode {
                    0 => M6Return::CallerPosition,
                    1 => M6Return::FixedPosition {
                        position_mm: cam_core::motion::Position {
                            x: 0.,
                            y: 0.,
                            z: 0.,
                        },
                    },
                    _ => M6Return::SafeRetract {
                        z_mm: 0.,
                        transit_xy_mm: cam_core::geometry::Point::new(0., 0.),
                    },
                };
                e.dirty = true;
            }
            match &mut m.m6.return_position {
                M6Return::CallerPosition => {}
                M6Return::FixedPosition { position_mm: p } => {
                    for (label, field, v) in [
                        ("M6 fixed X", "return-x", &mut p.x),
                        ("M6 fixed Y", "return-y", &mut p.y),
                        ("M6 fixed Z", "return-z", &mut p.z),
                    ] {
                        required(ui, label, &format!("{key}/{field}"), v, e);
                    }
                }
                M6Return::SafeRetract {
                    z_mm,
                    transit_xy_mm: p,
                } => {
                    for (label, field, v) in [
                        ("M6 retract Z", "return-z", z_mm),
                        ("M6 transit X", "return-x", &mut p.x),
                        ("M6 transit Y", "return-y", &mut p.y),
                    ] {
                        required(ui, label, &format!("{key}/{field}"), v, e);
                    }
                }
            }
        },
    );
    section(ui, (&key, "motion"), "Motion & coolant", false, |ui| {
        e.dirty |= select(
            ui,
            "Coolant",
            &mut m.coolant,
            &[
                ("Off", Coolant::Off),
                ("Flood", Coolant::Flood),
                ("Mist", Coolant::Mist),
            ],
        );
        let mut blend = matches!(m.path_control, PathControl::Blend { .. });
        if select(
            ui,
            "Path control",
            &mut blend,
            &[("Exact path", false), ("Blend", true)],
        ) {
            m.path_control = if blend {
                PathControl::Blend {
                    tolerance_mm: 0.,
                    naive_cam_tolerance_mm: None,
                }
            } else {
                PathControl::ExactPath
            };
            e.dirty = true;
        }
        if let PathControl::Blend {
            tolerance_mm,
            naive_cam_tolerance_mm,
        } = &mut m.path_control
        {
            required(
                ui,
                "Configuration blend tolerance",
                &format!("{key}/blend"),
                tolerance_mm,
                e,
            );
            number(
                ui,
                "Configuration naive CAM tolerance",
                &format!("{key}/naive"),
                naive_cam_tolerance_mm,
                false,
                e,
            );
        }
    });
    section(
        ui,
        (&key, "advanced"),
        "Program start & tool mappings",
        false,
        |ui| {
            let mut startup = m.program_start_position_mm.is_some();
            if ui
                .horizontal(|ui| {
                    let r = ui.checkbox(&mut startup, "Explicit program start position");
                    observe_control("Explicit program start position", r.rect);
                    help::icon(ui, "Explicit program start position");
                    r.changed()
                })
                .inner
            {
                m.program_start_position_mm = startup.then_some(cam_core::motion::Position {
                    x: 0.,
                    y: 0.,
                    z: 0.,
                });
                e.dirty = true;
            }
            if let Some(p) = &mut m.program_start_position_mm {
                for (label, field, v) in [
                    ("Program start X", "start-x", &mut p.x),
                    ("Program start Y", "start-y", &mut p.y),
                    ("Program start Z", "start-z", &mut p.z),
                ] {
                    required(ui, label, &format!("{key}/{field}"), v, e);
                }
            }

            ui.small("Mappings below identify job tool IDs explicitly. Only existing IDs are copied when applied.");
            for row in &mut m.tools {
                ui.label(format!("Mapping {}", row.tool_id));
                let mut t = Some(row.tool_number as f64);
                let mut h = row.length_offset_number.map(|v| v as f64);
                number(
                    ui,
                    &format!("Configuration T {}", row.tool_id),
                    &format!("{key}/T/{}", row.tool_id),
                    &mut t,
                    true,
                    e,
                );
                if m.length_compensation == LengthCompensation::ToolTable {
                    number(
                        ui,
                        &format!("Configuration H {}", row.tool_id),
                        &format!("{key}/H/{}", row.tool_id),
                        &mut h,
                        false,
                        e,
                    );
                }
                if let Some(t) = t {
                    if t.fract() == 0. && t > 0. && t <= u32::MAX as f64 {
                        row.tool_number = t as u32;
                    } else {
                        e.invalid.insert(format!("{key}/T/{}", row.tool_id));
                    }
                }
                if h.is_none_or(|h| h.fract() == 0. && h >= 0. && h <= u32::MAX as f64) {
                    row.length_offset_number = h.map(|h| h as u32);
                } else {
                    e.invalid.insert(format!("{key}/H/{}", row.tool_id));
                }
            }
            if m.tools.is_empty() {
                ui.small("Set tool numbers for the job's selected cutters in the job Machine panel after applying this profile.");
            }
        },
    );
}
impl App {
    fn library_machines(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context) {
        let Some(index) = self
            .resources
            .draft
            .machines
            .iter()
            .position(|m| m.id == self.resources.machine)
        else {
            return;
        };
        let mut machine = self.resources.draft.machines[index].clone();
        machine_form(ui, &mut machine, &mut self.resources);
        for error in crate::resources::machine_problems(&machine) {
            ui.colored_label(Color32::DARK_RED, error);
        }
        self.resources.machine = machine.id.clone();
        self.resources.draft.machines[index] = machine.clone();
    }
    /// The tool the selected operation's assignment currently addresses,
    /// resolved for display. `None` when there is no such assignment.
    pub(super) fn assigned_tool(&self, role: Role) -> Option<crate::resources::AssignedTool> {
        let document = self.document.as_ref()?;
        crate::resources::AssignedTool::of(&document.job, &document.raw.operation, role)
    }

    fn job_tools_window(&mut self, ctx: &egui::Context) {
        let mut open = true;
        egui::Window::new("Job tools and assignments").id(egui::Id::new("job-tools-editor")).open(&mut open).default_width(640.).show(ctx,|ui|{
            ui.style_mut().wrap_mode=Some(egui::TextWrapMode::Wrap);
            if button(ui,"Close job tools",true).clicked(){self.resources.jobs_open=false;}
            self.resource_role(ui);
            let area=egui::ScrollArea::vertical().max_height(430.).id_salt("job-tool-fields").show(ui,|ui|{
            let Some(job)=self.document.as_ref().map(|d|d.job.clone())else{return;};
            let statuses=core::assignment_statuses(&job);
            ui.small("These are copied physical tools. Editing geometry affects every listed assignment; cutting profiles remain assignment-specific.");
            for tool in &job.tools{
                let users:Vec<_>=statuses.iter().filter(|s|s.tool_id==tool.id).map(|s|format!("{} / {}",s.operation_id,crate::resources::role_word(s.role))).collect();
                // A row names the physical tool and where it came from: the
                // operation's placeholder is not a chosen cutter, and a
                // library copy reports the library it was copied from.
                let mut notes=vec![];
                if tool.geometry.is_none(){notes.push("never configured".to_string());}
                if let Some(origin)=&tool.library_origin{
                    let renamed=origin.name_at_copy!=tool.name;
                    notes.push(format!("from library '{}' · '{}' r{}{}",origin.library_id,origin.tool_id,origin.copied_revision,if renamed{" (renamed here)"}else{""}));
                }
                notes.push(if users.is_empty(){"unused".into()}else{users.join(", ")});
                ui.horizontal(|ui|{let r=ui.selectable_value(&mut self.resources.job_tool,tool.id.clone(),format!("{} · {}",tool.name,tool.id));observe_control(&format!("Job tool {}",tool.id),r.rect);ui.label(notes.join(" · "));});
            }
            if let Some(tool)=job.tools.iter().find(|t|t.id==self.resources.job_tool){
                // Address the operation the workspace has selected, not
                // whichever operation happens to be first.
                let operation = job
                    .operations
                    .iter()
                    .find(|operation| operation.id == self.document.as_ref().unwrap().raw.operation)
                    .map(|operation| operation.id.clone())
                    .unwrap_or_else(|| job.operations[0].id.clone());
                if button(ui,"Use tool in assignment",self.active.is_none()).clicked(){self.resource_command(R::UseTool{operation,role:self.resources.role,tool:tool.id.clone()},ctx);}
                if button(ui,"Edit copied geometry",true).clicked(){
                    match crate::resources::capture_tool(tool,tool.id.clone(),tool.name.clone()){
                        Ok(t)=>{self.resources.job_tool_draft=Some((self.revision,t));self.resources.job_raw.clear();self.resources.job_invalid.clear();},Err(e)=>self.status=e,
                    }
                }
            }
                if let Some((revision,mut tool))=self.resources.job_tool_draft.clone(){
                    let mut form=crate::resources::Editor{raw:std::mem::take(&mut self.resources.job_raw),invalid:std::mem::take(&mut self.resources.job_invalid),..Default::default()};
                    ui.push_id(tool.id.clone(),|ui|tool_form(ui,&mut tool,&mut form,"Copied"));
                    self.resources.job_raw=form.raw;self.resources.job_invalid=form.invalid;
                    self.resources.job_tool_draft=Some((revision,tool.clone()));
                    let ready=revision==self.revision&&self.active.is_none()&&self.resources.job_invalid.is_empty();
                    if revision!=self.revision{ui.colored_label(Color32::DARK_RED,"Job changed. Choose Edit copied geometry again to review the current tool.");}
                    if button(ui,"Apply copied geometry",ready).clicked()
                        && let Some(mut target)=job.tools.iter().find(|t|t.id==tool.id).cloned(){
                            target.name=tool.name.clone();target.capabilities.plunge_capable=tool.plunge_capable;target.capabilities.ramp_capable=tool.ramp_capable;
                            target.geometry=Some(match tool.geometry{LibraryGeometry::Endmill(g)=>cam_core::project::ToolGeometry::Endmill(cam_core::project::EndmillGeometry{diameter_mm:g.diameter_mm,cutting_length_mm:g.cutting_length_mm}),LibraryGeometry::Vbit(g)=>cam_core::project::ToolGeometry::Vbit(g),LibraryGeometry::DragKnife(g)=>cam_core::project::ToolGeometry::DragKnife(g)});
                            self.resource_command(R::EditTool{tool:target},ctx);
                    }
                }
            for status in statuses{
                let assignment=crate::resources::AssignedTool::of(&job,&status.operation_id,status.role);
                let role=crate::resources::role_word(status.role);
                ui.label(match assignment{
                    Some(tool)=>format!("{} · {}: {} · {}",status.operation_id,role,tool.tool_label(),tool.profile_label()),
                    None=>format!("{} · {}: no assignment",status.operation_id,role),
                });
            }
            });observe_control("Job tools viewport",area.inner_rect);
        });
        self.resources.jobs_open &= open;
    }
}

impl App {
    pub(super) fn applied_machine_options(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use cam_core::post::{Coolant, LengthCompensation, PathControl};
        let Some(mut machine) = self
            .document
            .as_ref()
            .and_then(|d| d.job.machine_configuration.clone())
        else {
            return;
        };
        let before = machine.clone();
        choice(
            ui,
            "Job work offset",
            &mut machine.work_offset,
            &[
                ("Unset", None),
                ("G54", Some("G54".into())),
                ("G55", Some("G55".into())),
                ("G56", Some("G56".into())),
                ("G57", Some("G57".into())),
                ("G58", Some("G58".into())),
                ("G59", Some("G59".into())),
                ("G59.1", Some("G59.1".into())),
                ("G59.2", Some("G59.2".into())),
                ("G59.3", Some("G59.3".into())),
            ],
        );
        choice(
            ui,
            "Job compensation",
            &mut machine.length_compensation,
            &[
                ("Unset", None),
                ("Macro managed", Some(LengthCompensation::MacroManaged)),
                ("Tool table", Some(LengthCompensation::ToolTable)),
            ],
        );
        choice(
            ui,
            "Job coolant",
            &mut machine.coolant,
            &[
                ("Unset", None),
                ("Off", Some(Coolant::Off)),
                ("Flood", Some(Coolant::Flood)),
                ("Mist", Some(Coolant::Mist)),
            ],
        );
        let mut blend = matches!(machine.path_control, Some(PathControl::Blend { .. }));
        if choice(
            ui,
            "Job path control",
            &mut blend,
            &[("Exact path", false), ("Blend", true)],
        ) {
            machine.path_control = Some(if blend {
                PathControl::Blend {
                    tolerance_mm: 0.,
                    naive_cam_tolerance_mm: None,
                }
            } else {
                PathControl::ExactPath
            });
        }
        if machine != before {
            let compensation_changed = machine.length_compensation != before.length_compensation;
            if compensation_changed
                && machine.length_compensation == Some(LengthCompensation::MacroManaged)
            {
                for row in &mut machine.tools {
                    row.length_offset_number = None;
                }
            }
            self.edit_job(
                ctx,
                if compensation_changed {
                    &[33, 39, 36]
                } else {
                    &[36]
                },
                |job| {
                    job.machine_configuration = Some(machine);
                    Ok(())
                },
            );
        }
        if button(ui, "Reusable machine settings", true).clicked() {
            self.resources.machines_view = true;
            self.resources.open = true;
            if !self.resources.ready {
                self.request_resources(ResourceIntent::Load, ctx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn import_failure_is_visible_in_the_library_and_preserves_its_buffer() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.resources.draft = catalog();
        a.resources.dirty = true;
        let before = a.resource_stamp();
        a.io = Some((7, IoKind::LibraryImport));
        a.event(
            Event::Io {
                id: 7,
                result: Err("Cannot read library file".into()),
            },
            &ctx,
        );
        assert_eq!(
            a.resources.error.as_deref(),
            Some("Cannot read library file")
        );
        assert_eq!(a.resource_stamp(), before);
        assert!(a.resources.dirty);
    }
    #[test]
    fn combined_library_action_fills_blank_geometry_capabilities_direction_and_raw_fields_atomically()
     {
        let ctx = egui::Context::default();
        let mut a = app();
        let d = a.document.as_mut().unwrap();
        d.job.tools[0].geometry = None;
        d.job.tools[0].capabilities.plunge_capable = None;
        assert!(d.edit(12, String::new()).is_err());
        assert!(d.edit(13, String::new()).is_err());
        let before = d.job.clone();
        let mut library = catalog();
        library.library.tools[0].spindle_direction =
            Some(cam_core::project::SpindleDirection::Counterclockwise);
        let action = R::ApplyToolProfile {
            catalog: library,
            tool: "endmill".into(),
            preset: "rough".into(),
            operation: "carving".into(),
            role: Role::Endmill,
        };
        a.active = Some((1, a.revision));
        let result = engine::execute(
            &mut cam_service::retained::Retained::new(),
            Command::Resource {
                job: before.to_json().unwrap(),
                action: Box::new(action),
            },
        );
        a.accept(1, result, &ctx);
        let d = a.document.as_ref().unwrap();
        assert_eq!(d.text(12), "2.5");
        assert_eq!(d.text(13), "15");
        let t = crate::authoring::tool(&d.job, false).unwrap();
        assert_eq!(t.capabilities.plunge_capable, Some(true));
        assert_eq!(t.capabilities.ramp_capable, Some(true));
        assert_eq!(
            engine::settings(&d.job).endmill.spindle_direction,
            Some(cam_core::project::SpindleDirection::Counterclockwise)
        );
        assert_eq!(d.text(2), "1200");
        assert_eq!(
            engine::settings(&d.job).vbit,
            engine::settings(&before).vbit
        );
        assert_eq!(a.undo.len(), 1);
        a.undo(&ctx);
        assert_eq!(a.document.as_ref().unwrap().job, before);
        assert_eq!(a.document.as_ref().unwrap().text(12), "");
    }
    fn app() -> App {
        App {
            document: Some(Document::new(
                engine::open(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap(),
            )),
            ..Default::default()
        }
    }
    fn catalog() -> Catalog {
        Catalog::decode(include_str!("../../../fixtures/gui5/library.json")).unwrap()
    }
    #[test]
    fn committed_library_revision_survives_job_undo_and_conflict_keeps_partial_buffer() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.resources.draft = catalog();
        a.resources.dirty = true;
        a.resources.busy = true;
        a.resource_request = Some((1, ResourceIntent::Save));
        let saved = StoredCatalog::next(None, catalog()).unwrap();
        a.accept_resources(1, Ok(Some(saved.clone())));
        assert_eq!(a.revision, 0);
        assert!(a.undo.is_empty());
        assert_eq!(a.resources.base.as_ref().unwrap().revision, 1);
        let command = R::Apply {
            operation: "carving".into(),
            role: Role::Endmill,
            catalog: catalog(),
            tool: "endmill".into(),
            preset: "rough".into(),
        };
        a.active = Some((2, a.revision));
        let result = engine::execute(
            &mut cam_service::retained::Retained::new(),
            Command::Resource {
                job: a.document.as_ref().unwrap().job.to_json().unwrap(),
                action: Box::new(command),
            },
        );
        a.accept(2, result, &ctx);
        assert_eq!(a.undo.len(), 1);
        assert_eq!(
            core::assignment_statuses(&a.document.as_ref().unwrap().job)[0].status,
            core::ProfileStatus::Applied
        );
        a.undo(&ctx);
        assert_eq!(a.resources.base.as_ref().unwrap().revision, 1);
        assert_eq!(
            core::assignment_statuses(&a.document.as_ref().unwrap().job)[0].status,
            core::ProfileStatus::Custom
        );
        a.resources.raw.insert("partial".into(), "-".into());
        a.resources.dirty = true;
        a.resources.busy = true;
        a.resource_request = Some((3, ResourceIntent::Save));
        a.accept_resources(3, Err("Revision conflict".into()));
        assert_eq!(a.resources.raw["partial"], "-");
        assert!(a.resources.dirty);
        assert!(!a.resources.busy);
        a.resource_request = Some((4, ResourceIntent::Compare));
        a.accept_resources(
            4,
            Ok(Some(StoredCatalog::next(Some(1), catalog()).unwrap())),
        );
        assert_eq!(a.resources.raw["partial"], "-");
        assert_eq!(a.resources.base.unwrap().revision, 1);
        assert_eq!(a.resources.conflict.unwrap().revision, 2);
    }
    #[test]
    fn stale_resource_job_completion_and_import_cannot_replace_new_edits() {
        let ctx = egui::Context::default();
        let mut a = app();
        a.active = Some((1, a.revision));
        let result = engine::execute(
            &mut cam_service::retained::Retained::new(),
            Command::Resource {
                job: a.document.as_ref().unwrap().job.to_json().unwrap(),
                action: Box::new(R::Apply {
                    operation: "carving".into(),
                    role: Role::Endmill,
                    catalog: catalog(),
                    tool: "endmill".into(),
                    preset: "rough".into(),
                }),
            },
        );
        a.document.as_mut().unwrap().edit(2, "777".into()).unwrap();
        a.changed(&ctx);
        a.accept(1, result, &ctx);
        assert_eq!(a.document.as_ref().unwrap().text(2), "777");
        assert_eq!(
            core::assignment_statuses(&a.document.as_ref().unwrap().job)[0].status,
            core::ProfileStatus::Custom
        );
        a.io = Some((2, IoKind::LibraryImport));
        a.resource_import_stamp = Some(a.resource_stamp());
        a.resources.draft.id = "newer-edits".into();
        a.event(
            Event::Io {
                id: 2,
                result: Ok(IoValue::Job(serde_json::to_string(&catalog()).unwrap())),
            },
            &ctx,
        );
        assert_eq!(a.resources.draft.id, "newer-edits");
        assert!(a.resources.status.contains("changed while choosing"));
    }
}
