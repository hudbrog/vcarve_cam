use super::*;
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
    ui.horizontal(|ui| {
        ui.label(label);
        let r = ui.add(
            egui::TextEdit::singleline(value)
                .desired_width(240.)
                .char_limit(1000),
        );
        observe_control(label, r.rect);
        r.changed()
    })
    .inner
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
fn capability(ui: &mut egui::Ui, label: &str, value: &mut Option<bool>) -> bool {
    let before = *value;
    ui.horizontal(|ui| {
        ui.label(label);
        for (name, v) in [("Unset", None), ("Yes", Some(true)), ("No", Some(false))] {
            let r = ui.selectable_value(value, v, name);
            observe_control(&format!("{label} {name}"), r.rect);
        }
    });
    before != *value
}
fn tool_form(
    ui: &mut egui::Ui,
    tool: &mut LibraryTool,
    e: &mut crate::resources::Editor,
    prefix: &str,
) {
    e.dirty |= text(ui, &format!("{prefix} tool name"), &mut tool.name);
    let key = tool.id.clone();
    match &mut tool.geometry {
        LibraryGeometry::Endmill(g) => {
            required(
                ui,
                &format!("{prefix} diameter"),
                &format!("{key}/diameter"),
                &mut g.diameter_mm,
                e,
            );
            required(
                ui,
                &format!("{prefix} cutting length"),
                &format!("{key}/length"),
                &mut g.cutting_length_mm,
                e,
            );
        }
        LibraryGeometry::Vbit(g) => {
            required(
                ui,
                &format!("{prefix} V-bit angle"),
                &format!("{key}/angle"),
                &mut g.included_angle_deg,
                e,
            );
            required(
                ui,
                &format!("{prefix} tip diameter"),
                &format!("{key}/tip"),
                &mut g.tip_diameter_mm,
                e,
            );
            required(
                ui,
                &format!("{prefix} cutting diameter"),
                &format!("{key}/diameter"),
                &mut g.max_cutting_diameter_mm,
                e,
            );
            required(
                ui,
                &format!("{prefix} cutting height"),
                &format!("{key}/height"),
                &mut g.cutting_height_mm,
                e,
            );
        }
        LibraryGeometry::DragKnife(g) => {
            required(
                ui,
                &format!("{prefix} blade offset"),
                &format!("{key}/offset"),
                &mut g.blade_offset_mm,
                e,
            );
            required(
                ui,
                &format!("{prefix} cut depth"),
                &format!("{key}/depth"),
                &mut g.max_cut_depth_mm,
                e,
            );
        }
    }
    e.dirty |= capability(ui, &format!("{prefix} plunge"), &mut tool.plunge_capable);
    if let LibraryGeometry::Endmill(g) = &mut tool.geometry
        && let Some(plunge) = tool.plunge_capable
    {
        g.plunge_capable = plunge;
    }
    e.dirty |= capability(ui, &format!("{prefix} ramp"), &mut tool.ramp_capable);
}
fn preset_form(
    ui: &mut egui::Ui,
    tool: &str,
    preset: &mut CuttingPreset,
    e: &mut crate::resources::Editor,
) {
    e.dirty |= text(ui, "Profile name", &mut preset.name);
    for (label, slot) in [
        ("Profile material", &mut preset.material),
        ("Profile machine context", &mut preset.machine),
    ] {
        let mut value = slot.clone().unwrap_or_default();
        if text(ui, label, &mut value) {
            *slot = (!value.is_empty()).then_some(value);
            e.dirty = true;
        }
    }
    for (label, field, value) in [
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
    ] {
        number(
            ui,
            label,
            &format!("{tool}/{}/{field}", preset.id),
            value,
            false,
            e,
        );
    }
}
impl App {
    pub(super) fn resource_stamp(&self) -> String {
        crate::compute::hash(
            &serde_json::to_vec(&(&self.resources.draft, &self.resources.raw)).unwrap(),
        )
    }
    fn resource_command(&mut self, action: R, ctx: &egui::Context) {
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
        let save = if matches!(intent, ResourceIntent::Save) {
            if !self.resources.invalid.is_empty() {
                self.resources.status = "Complete partial library fields before Save.".into();
                return;
            }
            if let Err(e) = self.resources.draft.validate() {
                self.resources.status = e;
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
            Err(e) => self.resources.status = format!("{e} Your library edits are preserved."),
            Ok(stored) if matches!(intent, ResourceIntent::Compare) => {
                self.resources.conflict = stored;
                self.resources.status =
                    "Stored revision loaded for comparison; your edits are unchanged.".into();
            }
            Ok(stored) => {
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
                // Imported content is only a buffer until explicit conditional Save.
                catalog.library.revision = self.resources.base.as_ref().map_or(0, |s| s.revision);
                self.resources.draft = catalog;
                self.resources.dirty = true;
                self.resources.raw.clear();
                self.resources.invalid.clear();
                self.resources.status =
                    "Imported into the edit buffer. Review and Save library explicitly.".into();
            }
            Err(e) => self.resources.status = e,
        }
    }
    pub(super) fn import_machine(&mut self, json: &str) {
        match cam_core::post::sequence::SequenceProfile::from_json(json) {
            Ok(mut machine) => {
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
            Err(e) => self.resources.status = e.to_string(),
        }
    }
    pub(super) fn resource_windows(&mut self, ctx: &egui::Context) {
        if self.resources.open {
            let mut open = true;
            egui::Window::new("Tool library & machine configurations")
                .id(egui::Id::new("resource-editor"))
                .open(&mut open)
                .default_width(650.)
                .default_height(530.)
                .resizable(true)
                .show(ctx, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.horizontal_wrapped(|ui| {
                        if button(ui, "Close library", true).clicked() {
                            self.resources.open = false;
                        }
                        if button(
                            ui,
                            "Load library",
                            !self.resources.busy && !self.resources.dirty,
                        )
                        .clicked()
                        {
                            self.request_resources(ResourceIntent::Load, ctx);
                        }
                        if button(
                            ui,
                            "Save library",
                            self.resources.ready && !self.resources.busy && self.resources.dirty,
                        )
                        .clicked()
                        {
                            self.request_resources(ResourceIntent::Save, ctx);
                        }
                        if button(
                            ui,
                            "Compare stored revision",
                            self.resources.ready && !self.resources.busy,
                        )
                        .clicked()
                        {
                            self.request_resources(ResourceIntent::Compare, ctx);
                        }
                    });
                    ui.label(&self.resources.status);
                    if self.document.is_some() {
                        ui.small(format!("Job: {}", self.status));
                    }
                    ui.small(format!(
                        "Revision {} · {}",
                        self.resources.base.as_ref().map_or(0, |s| s.revision),
                        if self.resources.dirty {
                            "Library has unsaved edits"
                        } else {
                            "Library unchanged"
                        }
                    ));
                    self.resource_role(ui);
                    if let Some(other) = self.resources.conflict.clone() {
                        ui.collapsing(
                            format!("Compare with stored revision {}", other.revision),
                            |ui| {
                                ui.label("Stored content");
                                ui.monospace(
                                    serde_json::to_string_pretty(&other.snapshot).unwrap(),
                                );
                                ui.label("Your edit buffer");
                                ui.monospace(
                                    serde_json::to_string_pretty(&self.resources.draft).unwrap(),
                                );
                            },
                        );
                        ui.horizontal_wrapped(|ui| {
                            if button(ui, "Reload stored library", !self.resources.busy).clicked() {
                                self.resources.draft = other.snapshot.clone();
                                self.resources.base = Some(other.clone());
                                self.resources.dirty = false;
                                self.resources.raw.clear();
                                self.resources.invalid.clear();
                                self.resources.conflict = None;
                            }
                            if button(ui, "Overwrite reviewed revision", !self.resources.busy)
                                .clicked()
                            {
                                self.resources.base = Some(other);
                                self.request_resources(ResourceIntent::Save, ctx);
                            }
                        });
                    }
                    if !self.resources.ready {
                        ui.label("Load the local library before editing or importing.");
                        return;
                    }
                    ui.add_enabled_ui(!self.resources.busy, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            if button(ui, "Import library", self.io.is_none()).clicked() {
                                self.open(IoKind::LibraryImport, ctx);
                            }
                            if button(
                                ui,
                                "Export library",
                                self.io.is_none() && self.resources.invalid.is_empty(),
                            )
                            .clicked()
                            {
                                match self.resources.draft.validate() {
                                    Ok(()) => self.save(
                                        "cam-library.json".into(),
                                        serde_json::to_vec_pretty(&self.resources.draft).unwrap(),
                                        None,
                                        ctx,
                                    ),
                                    Err(e) => self.resources.status = e,
                                }
                            }
                            if button(ui, "Import machine configuration", self.io.is_none())
                                .clicked()
                            {
                                self.open(IoKind::MachineImport, ctx);
                            }
                        });
                        let area = egui::ScrollArea::vertical()
                            .id_salt("resources-content")
                            .max_height(430.)
                            .show(ui, |ui| {
                                self.library_tools(ui, ctx);
                                ui.separator();
                                self.library_machines(ui, ctx);
                            });
                        observe_control("Resource viewport", area.inner_rect);
                    });
                });
            self.resources.open &= open;
        }
        if self.resources.jobs_open {
            self.job_tools_window(ctx);
        }
    }
    fn library_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Library tools");
        let tools: Vec<_> = self
            .resources
            .draft
            .library
            .tools
            .iter()
            .map(|t| (t.id.clone(), t.name.clone()))
            .collect();
        ui.horizontal_wrapped(|ui| {
            for (id, name) in tools {
                let r = ui.selectable_value(&mut self.resources.tool, id.clone(), name);
                observe_control(&format!("Library tool {id}"), r.rect);
            }
        });
        ui.horizontal_wrapped(|ui| {
            for (label, finish) in [("New endmill", false), ("New V-bit", true)] {
                if button(ui, label, true).clicked() {
                    let id = unique(
                        "tool",
                        self.resources
                            .draft
                            .library
                            .tools
                            .iter()
                            .map(|t| t.id.clone()),
                    );
                    let geometry = if finish {
                        LibraryGeometry::Vbit(cam_core::model::VBitSpec {
                            included_angle_deg: 0.,
                            tip_diameter_mm: 0.,
                            max_cutting_diameter_mm: 0.,
                            cutting_height_mm: 0.,
                        })
                    } else {
                        LibraryGeometry::Endmill(cam_core::model::EndmillSpec {
                            diameter_mm: 0.,
                            cutting_length_mm: 0.,
                            plunge_capable: false,
                        })
                    };
                    self.resources.draft.library.tools.push(LibraryTool {
                        id: id.clone(),
                        name: label.into(),
                        geometry,
                        ramp_capable: None,
                        plunge_capable: None,
                        cutting_presets: vec![],
                        knife_cutting_presets: vec![],
                    });
                    self.resources.tool = id;
                    self.resources.dirty = true;
                }
            }
            if button(ui, "Capture job geometry", self.document.is_some()).clicked() {
                let job = &self.document.as_ref().unwrap().job;
                let source = crate::authoring::tool(job, self.resources.role == Role::Vbit);
                if let Some(source) = source {
                    let id = unique(
                        "tool",
                        self.resources
                            .draft
                            .library
                            .tools
                            .iter()
                            .map(|t| t.id.clone()),
                    );
                    match crate::resources::capture_tool(source, id.clone(), source.name.clone()) {
                        Ok(tool) => {
                            self.resources.draft.library.tools.push(tool);
                            self.resources.tool = id;
                            self.resources.dirty = true;
                        }
                        Err(e) => self.resources.status = e,
                    }
                }
            }
        });
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
        ui.push_id(tool.id.clone(), |ui| {
            tool_form(ui, &mut tool, &mut self.resources, "Library");
        });
        self.resources.draft.library.tools[index] = tool.clone();
        if button(ui, "Duplicate library tool", true).clicked() {
            let mut copy = tool.clone();
            copy.id = unique(
                "tool",
                self.resources
                    .draft
                    .library
                    .tools
                    .iter()
                    .map(|t| t.id.clone()),
            );
            copy.name.push_str(" copy");
            self.resources.tool = copy.id.clone();
            self.resources.draft.library.tools.push(copy);
            self.resources.dirty = true;
        }
        let reviewed = self
            .resources
            .base
            .as_ref()
            .filter(|_| !self.resources.dirty && self.resources.invalid.is_empty())
            .map(|s| s.snapshot.clone());
        if button(
            ui,
            "Add geometry to job",
            reviewed.is_some() && self.document.is_some() && self.active.is_none(),
        )
        .clicked()
        {
            self.resource_command(
                R::AddTool {
                    catalog: reviewed.clone().unwrap(),
                    tool: tool.id.clone(),
                },
                ctx,
            );
        }
        ui.small(
            "Add geometry copies a physical tool. Choose its assignment separately in Job tools.",
        );
        ui.separator();
        ui.heading("Cutting profiles");
        ui.horizontal_wrapped(|ui| {
            for p in &tool.cutting_presets {
                let r = ui.selectable_value(&mut self.resources.preset, p.id.clone(), &p.name);
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
                let id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
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
                self.document.is_some() && !matches!(tool.geometry, LibraryGeometry::DragKnife(_)),
            )
            .clicked()
            {
                let id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
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
                copy.id = unique("profile", tool.cutting_presets.iter().map(|p| p.id.clone()));
                copy.name.push_str(" copy");
                self.resources.preset = copy.id.clone();
                tool.cutting_presets.push(copy);
                self.resources.dirty = true;
            }
            let usable = reviewed.is_some()
                && !self.resources.dirty
                && self.document.is_some()
                && self.active.is_none();
            if button(ui, "Apply cutting profile", usable).clicked() {
                self.resource_command(
                    R::Apply {
                        operation: self.document.as_ref().unwrap().job.operations[0].id.clone(),
                        role: self.resources.role,
                        catalog: reviewed.clone().unwrap(),
                        tool: tool.id.clone(),
                        preset: self.resources.preset.clone(),
                    },
                    ctx,
                );
            }
        }
        self.resources.draft.library.tools[index] = tool;
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
    }
    fn resource_role(&mut self, ui: &mut egui::Ui) {
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
    ui.label(format!("Configuration ID: {}", m.id));
    e.dirty |= choice(
        ui,
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
        ui,
        "Configuration clearance",
        &format!("{key}/clearance"),
        &mut m.clearance_z_mm,
        e,
    );
    let mut precision = m.decimal_places as f64;
    required(
        ui,
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
        ui,
        "Configuration spinup seconds",
        &format!("{key}/spinup"),
        &mut m.spindle_spinup_seconds,
        e,
    );
    e.dirty |= choice(
        ui,
        "Length compensation",
        &mut m.length_compensation,
        &[
            ("Macro managed", LengthCompensation::MacroManaged),
            ("Tool table", LengthCompensation::ToolTable),
        ],
    );
    e.dirty |= choice(
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
    if choice(
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
    let mut startup = m.program_start_position_mm.is_some();
    if ui
        .checkbox(&mut startup, "Explicit program start position")
        .changed()
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
    ui.separator();
    ui.label("M6 contract");
    e.dirty |= text(ui, "Contract reference", &mut m.m6.reference);
    for (label, value) in [
        ("Contract reviewed", &mut m.m6.reviewed),
        ("Preserves work datum", &mut m.m6.preserves_work_datum),
        ("Local offsets unused", &mut m.m6.local_offsets_unused),
        ("Z-only tool offsets", &mut m.m6.tool_offsets_z_only),
    ] {
        let r = ui.checkbox(value, label);
        observe_control(label, r.rect);
        e.dirty |= r.changed();
    }
    let mut mode = match m.m6.return_position {
        M6Return::CallerPosition => 0,
        M6Return::FixedPosition { .. } => 1,
        M6Return::SafeRetract { .. } => 2,
    };
    if choice(
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
        number(
            ui,
            &format!("Configuration H {}", row.tool_id),
            &format!("{key}/H/{}", row.tool_id),
            &mut h,
            false,
            e,
        );
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
}
impl App {
    fn library_machines(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Machine configurations");
        ui.horizontal_wrapped(|ui| {
            for m in &self.resources.draft.machines {
                let r = ui.selectable_value(&mut self.resources.machine, m.id.clone(), &m.id);
                observe_control(&format!("Library machine {}", m.id), r.rect);
            }
        });
        if button(
            ui,
            "Capture applied machine",
            self.document
                .as_ref()
                .is_some_and(|d| d.job.machine_configuration.is_some()),
        )
        .clicked()
        {
            match cam_core::project::v5::machine::resolve_sequence_profile(
                &self.document.as_ref().unwrap().job,
                &cam_core::project::v5::ReadinessScope::AllEnabled,
            ) {
                Ok(mut m) => {
                    m.id = unique(
                        "machine",
                        self.resources.draft.machines.iter().map(|m| m.id.clone()),
                    );
                    self.resources.machine = m.id.clone();
                    self.resources.draft.machines.push(m);
                    self.resources.dirty = true;
                }
                Err(e) => self.resources.status = e.to_string(),
            }
        }
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
        self.resources.machine = machine.id.clone();
        self.resources.draft.machines[index] = machine.clone();
        if button(ui, "Duplicate machine configuration", true).clicked() {
            machine.id = unique(
                "machine",
                self.resources.draft.machines.iter().map(|m| m.id.clone()),
            );
            self.resources.machine = machine.id.clone();
            self.resources.draft.machines.push(machine.clone());
            self.resources.dirty = true;
        }
        let ready = !self.resources.dirty
            && self.resources.invalid.is_empty()
            && self.document.is_some()
            && self.active.is_none();
        if button(ui, "Apply reviewed machine", ready).clicked() {
            self.resource_command(R::Machine { profile: machine }, ctx);
        }
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
                let users:Vec<_>=statuses.iter().filter(|s|s.tool_id==tool.id).map(|s|format!("{} / {:?}",s.operation_id,s.role)).collect();
                ui.horizontal(|ui|{let r=ui.selectable_value(&mut self.resources.job_tool,tool.id.clone(),format!("{} · {}",tool.name,tool.id));observe_control(&format!("Job tool {}",tool.id),r.rect);ui.label(if users.is_empty(){"Unused".into()}else{users.join(", ")});});
            }
            if let Some(tool)=job.tools.iter().find(|t|t.id==self.resources.job_tool){
                if button(ui,"Use tool in assignment",self.active.is_none()).clicked(){self.resource_command(R::UseTool{operation:job.operations[0].id.clone(),role:self.resources.role,tool:tool.id.clone()},ctx);}
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
            for status in statuses{ui.label(format!("{} / {:?}: {:?}",status.operation_id,status.role,status.status));}
            });observe_control("Job tools viewport",area.inner_rect);
        });
        self.resources.jobs_open &= open;
    }
}

impl App {
    pub(super) fn assignment_profiles(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        finish: bool,
    ) {
        let role = if finish { Role::Vbit } else { Role::Endmill };
        let Some(doc) = &self.document else {
            return;
        };
        let state = core::assignment_statuses(&doc.job)
            .into_iter()
            .find(|s| s.role == role)
            .unwrap();
        ui.label(format!("Cutting profile: {:?}", state.status));
        if let Some(p) = &state.applied {
            ui.small(format!(
                "{} · copied revision {}",
                p.name_at_application, p.revision_at_application
            ));
        }
        let operation = doc.job.operations[0].id.clone();
        ui.horizontal_wrapped(|ui| {
            if button(
                ui,
                if finish {
                    "Finish profiles"
                } else {
                    "Roughing profiles"
                },
                true,
            )
            .clicked()
            {
                self.resources.role = role;
                self.resources.open = true;
                if !self.resources.ready {
                    self.request_resources(ResourceIntent::Load, ctx);
                }
            }
            if button(
                ui,
                if finish {
                    "Reset finish overrides"
                } else {
                    "Reset roughing overrides"
                },
                state.status != core::ProfileStatus::Custom && self.active.is_none(),
            )
            .clicked()
            {
                self.resource_command(R::Reset { operation, role }, ctx);
            }
        });
    }
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
            self.edit_job(ctx, &[36], |job| {
                job.machine_configuration = Some(machine);
                Ok(())
            });
        }
        if button(ui, "Reusable machine settings", true).clicked() {
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
