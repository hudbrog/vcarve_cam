use super::*;
#[path = "knife_ui.rs"]
mod knife_ui;
#[path = "operation_ui.rs"]
mod operation_ui;
use crate::authoring::{self, settings_mut};
use cam_core::project::{FlatVcarveMode, SpindleDirection, WorkZeroXY, WorkZeroZ};

impl App {
    pub(super) fn edit_job(
        &mut self,
        ctx: &egui::Context,
        clear: &[usize],
        edit: impl FnOnce(&mut CamJobV5) -> Result<(), String>,
    ) {
        let Some(doc) = &self.document else { return };
        let mut job = doc.job.clone();
        let cached_finish = engine::carving(&job)
            .and_then(|s| s.finish.clone())
            .or(doc.finish_draft.clone());
        let old_mode = engine::carving(&job).map(|s| s.mode);
        if let Err(error) = edit(&mut job) {
            self.status = error;
            return;
        }
        if old_mode != engine::carving(&job).map(|s| s.mode)
            && engine::carving(&job).is_some_and(|s| s.mode == FlatVcarveMode::Combined)
            && cached_finish.is_some()
        {
            settings_mut(&mut job).finish = cached_finish.clone();
        }
        if let Err(error) = job.validate_structure().map_err(|e| e.to_string()) {
            self.status = error;
            return;
        }
        self.remember();
        let doc = self.document.as_mut().unwrap();
        doc.finish_draft = engine::carving(&job)
            .and_then(|s| s.finish.clone())
            .or(cached_finish);
        doc.job = job;
        for &field in clear {
            doc.raw.raw.remove(&doc.raw.key(field));
        }
        self.edit_group = None;
        self.changed(ctx);
        self.status = "Job changed. Generate to update the cutting result.".into();
    }
    fn numbers(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, fields: &[usize]) {
        for &field in fields {
            if !FIELDS[field]
                .to_lowercase()
                .contains(&self.search.to_lowercase())
            {
                continue;
            }
            let Some(doc) = &self.document else {
                return;
            };
            let mut text = doc.text(field);
            let operation = self.inspector_tab == 2;
            let response = ui
                .with_layout(
                    if operation {
                        egui::Layout::top_down(egui::Align::Min)
                    } else {
                        egui::Layout::left_to_right(egui::Align::Center)
                    },
                    |ui| {
                        let name = match field {
                            2 | 3 if operation => "Cutting feed",
                            21 if operation => "Plunge feed",
                            20 if operation => "Stepdown",
                            46 if operation => "Stepover",
                            16 if operation => "Included angle",
                            32 => "Tool number (T)",
                            33 => "Length entry (H)",
                            38 => "V-bit tool (T)",
                            39 => "V-bit entry (H)",
                            _ => FIELDS[field],
                        };
                        let label = if operation {
                            ui.horizontal_wrapped(|ui| {
                                let response = ui.label(name);
                                help::icon(ui, FIELDS[field]);
                                response
                            })
                            .inner
                        } else {
                            ui.add_sized([116., 20.], egui::Label::new(name))
                        };
                        ui.horizontal(|ui| {
                            let response = ui
                                .add(
                                    egui::TextEdit::singleline(&mut text)
                                        .id(egui::Id::new((
                                            "carving-field",
                                            doc.raw.key(field),
                                            &doc.raw.operation,
                                            field,
                                        )))
                                        .desired_width(if operation {
                                            (ui.available_width() - 58.).clamp(65., 160.)
                                        } else {
                                            72.
                                        })
                                        .char_limit(128)
                                        .hint_text("Unset"),
                                )
                                .labelled_by(label.id);
                            ui.small(match field {
                                2 | 3 | 10 | 21 | 51 | 63..=65 => "mm/min",
                                11 | 22 => "RPM",
                                14 | 16 | 28 | 69 | 72 => "deg",
                                29 => "×",
                                34 | 37 => "s",
                                35 => "digits",
                                32 | 33 | 38 | 39 | 48..=50 | 52..=56 | 58..=60 => "",
                                _ => "mm",
                            });
                            if !operation {
                                help::icon(ui, FIELDS[field]);
                            }
                            response
                        })
                        .inner
                    },
                )
                .inner;
            observe_control(FIELDS[field], response.rect);
            if self.issue_focus.as_deref() == Some(FIELDS[field]) {
                response.scroll_to_me(Some(egui::Align::Center));
                response.request_focus();
                self.issue_focus = None;
            }
            if response.changed() {
                if self.edit_group != Some(field) {
                    self.remember();
                    self.edit_group = Some(field);
                }
                let result = self.document.as_mut().unwrap().edit(field, text.clone());
                self.changed(ctx);
                self.status = match result {
                    Ok(()) => "Setting changed; generate to update simulation.".into(),
                    Err(error) => {
                        self.issues = vec![cam_core::operations::LocatedDiagnostic {
                            code: "EDITOR_VALUE".into(),
                            message: error.clone(),
                            operation_id: self
                                .document
                                .as_ref()
                                .unwrap()
                                .job
                                .operations
                                .first()
                                .map(|o| o.id.clone()),
                            tool_id: None,
                            field_path: Some(format!("editor.fields.{}", FIELDS[field])),
                        }];
                        error
                    }
                };
            }
            if response.lost_focus() {
                self.edit_group = None;
            }
            if let Err(error) = Draft::parse(&text) {
                ui.colored_label(Color32::from_rgb(176, 42, 35), error);
            }
        }
    }
    pub(super) fn inspector(&mut self, ctx: &egui::Context) {
        let panel=egui::SidePanel::right("gui2-inspector").default_width(self.inspector_width).width_range(300.0..=480.0).resizable(true).show(ctx,|ui|{
            ui.add_space(8.);
            if self.inspector_tab != 2 {
                ui.strong(["ARTWORK", "STOCK & WORK ZERO", "OPERATION", "MACHINE", "JOB TOOL · ENDMILL", "JOB TOOL · V-BIT", "RESULT INSPECTION", "JOB SETTINGS"][self.inspector_tab]);
                ui.separator();
            }
            if self.inspector_tab == 2 && self.document.is_some() { self.operation_header(ui,ctx); }
            if self.inspector_tab != 6 {
            let r=ui.add(egui::TextEdit::singleline(&mut self.search).id(egui::Id::new("gui2-search")).char_limit(512).hint_text("Filter fields"));observe_control("Filter fields",r.rect);
            if r.changed(){self.scroll[self.inspector_tab]=0.;if self.inspector_tab==2 {self.operation_scroll[self.operation_tab]=0.;}}
            }
            let operation = self.inspector_tab == 2;
            let offset = if operation { self.operation_scroll[self.operation_tab] } else { self.scroll[self.inspector_tab] };
            let area=egui::ScrollArea::vertical().id_salt(("inspector-scroll",self.inspector_tab,if operation {self.operation_tab} else {0})).auto_shrink([false,!operation]).max_height(if operation { (ui.available_height()-55.).max(100.) } else {ui.available_height()}).vertical_scroll_offset(offset).show(ui,|ui|{
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                if operation { ui.spacing_mut().interact_size.y = 22.; }
                if self.document.is_none(){ui.label("Import an SVG or open a saved job to begin.");return;}
                if self.document.as_ref().unwrap().job.operations.is_empty() {
                    match self.inspector_tab {
                        1 => self.setup_panel(ui,ctx),
                        7 => self.job_settings_panel(ui,ctx),
                        _ => { ui.heading("No operations"); ui.label("Add an operation from the Operations list. Your artwork, stock, tools and machine settings are retained."); if self.inspector_tab == 0 {self.numbers(ui,ctx,&[26,27,28,29]);} }
                    }
                } else if crate::knife::settings(&self.document.as_ref().unwrap().job).is_some() && matches!(self.inspector_tab,0|2|4|5|6) {
                    self.knife_panel(ui,ctx);
                } else { match self.inspector_tab {0=>self.artwork_panel(ui,ctx),1=>self.setup_panel(ui,ctx),2=>self.cutting_panel(ui,ctx),3=>self.machine_panel(ui,ctx),6=>self.view.inspection_controls(ui),7=>self.job_settings_panel(ui,ctx),index=>{
                    let finish=index==5;
                    ui.heading(if finish {"V-bit geometry"}else{"Endmill geometry"});
                    self.numbers(ui,ctx,if finish {&[16,17,18,19]}else{&[12,13]});
                    if button(ui, if finish {"Unset V-bit geometry"} else {"Unset endmill geometry"}, true).clicked() {
                        self.edit_job(ctx, if finish {&[16,17,18,19]} else {&[12,13]}, |job| { authoring::tool_mut(job, finish)?.geometry = None; Ok(()) });
                    }
                    ui.separator();ui.label("Used by Flat V-carve");
                    ui.small(if finish {"Defines the V-shaped target in both modes; executes finishing in Combined mode."}else{"Endmill clearing stage"});
                    if button(ui,"Edit cutting assignment",true).clicked(){self.navigate(2);}
                    if button(ui,"Edit controller mapping",true).clicked(){self.navigate(3);}
                }}}
                if let Some(label) = self.issue_focus.clone() {
                    let rect = CONTROLS.with(|c| c.borrow().get(&label).copied());
                    if let Some([x0,y0,x1,y1]) = rect { ui.scroll_to_rect(egui::Rect::from_min_max(egui::pos2(x0,y0),egui::pos2(x1,y1)), Some(egui::Align::Center)); self.issue_focus = None; }
                }
                if self.document.as_ref().is_some_and(Document::pending){ui.colored_label(Color32::from_rgb(164,83,12),"Complete partial fields before generation or job save. Recovery keeps the raw text.");}
            });
            observe_control("Inspector viewport",area.inner_rect);
            if operation {
                self.operation_scroll[self.operation_tab]=area.state.offset.y;
                ui.separator();
                ui.horizontal(|ui| {
                    ui.small("Changes apply to this job");
                    let ready = !self.operation_ramp_draft && self.document.as_ref().is_some_and(|d| !d.pending() && !d.job.operations.is_empty());
                    let generate = ui.add_enabled(ready && self.active.is_none() && self.io.is_none(),egui::Button::new("Generate").fill(Color32::from_rgb(49,190,195)));
                    observe_control("Generate operation",generate.rect);
                    if generate.clicked() {
                        self.submit(Command::Generate {job:self.document.as_ref().unwrap().job.to_json().unwrap()},ctx);
                    }
                });
            } else { self.scroll[self.inspector_tab]=area.state.offset.y; }
        });
        self.inspector_width = panel.response.rect.width();
    }
    fn artwork_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Artwork & placement");
        for rejection in &self.artwork_rejections {
            ui.colored_label(Color32::from_rgb(176, 42, 35), rejection);
        }
        let item = self.document.as_ref().unwrap().active_artwork().cloned();
        let idle = self.active.is_none() && self.io.is_none();
        ui.horizontal_wrapped(|ui| {
            if button(ui, "Add SVG", idle).clicked() {
                self.open(IoKind::AddSvg, ctx);
            }
            if button(ui, "Replace SVG", idle && item.is_some()).clicked() {
                self.open(IoKind::ReplaceSvg, ctx);
            }
            if button(ui, "Delete artwork", idle && item.is_some()).clicked() {
                self.artwork_command(
                    engine::ArtworkCommand::Delete {
                        item: item.as_ref().unwrap().id.clone(),
                    },
                    ctx,
                );
            }
        });
        if let Some(item) = item {
            ui.horizontal_wrapped(|ui| {
                let mut hidden = self.view.artwork.hidden.contains(&item.id.0);
                let r = ui.checkbox(&mut hidden, "Hide artwork");
                observe_control("Hide artwork", r.rect);
                if r.changed() {
                    if hidden {
                        self.view.artwork.hidden.insert(item.id.0.clone());
                    } else {
                        self.view.artwork.hidden.remove(&item.id.0);
                    }
                }
                let mut locked = self.view.artwork.locked.contains(&item.id.0);
                let r = ui.checkbox(&mut locked, "Lock artwork");
                observe_control("Lock artwork", r.rect);
                if r.changed() {
                    if locked {
                        self.view.artwork.locked.insert(item.id.0.clone());
                    } else {
                        self.view.artwork.locked.remove(&item.id.0);
                    }
                }
            });
            ui.small("Hide/Lock affect this workspace only. Assigned hidden artwork still cuts; numeric edits remain available when locked.");
            ui.horizontal_wrapped(|ui| {
                if button(ui, "Duplicate artwork", idle).clicked() {
                    self.artwork_command(
                        engine::ArtworkCommand::Duplicate {
                            item: item.id.clone(),
                        },
                        ctx,
                    );
                }
                let ids: Vec<_> = self
                    .document
                    .as_ref()
                    .unwrap()
                    .job
                    .artwork
                    .iter()
                    .map(|i| i.id.clone())
                    .collect();
                let at = ids.iter().position(|id| id == &item.id).unwrap();
                for (label, next) in [
                    ("Move row up", at.checked_sub(1)),
                    ("Move row down", (at + 1 < ids.len()).then_some(at + 1)),
                ] {
                    if button(ui, label, idle && next.is_some()).clicked() {
                        let mut ordered = ids.clone();
                        ordered.swap(at, next.unwrap());
                        self.artwork_command(
                            engine::ArtworkCommand::Reorder { items: ordered },
                            ctx,
                        );
                    }
                }
            });
            ui.label(format!("{} · {}", item.name, item.id.0));
            ui.small("SVG units become mm. Origin is in the SVG page: placement = scale × rotate(page − origin).");
            self.numbers(ui, ctx, &[26, 27, 28, 29]);
        } else {
            ui.label("Add an SVG to this project. Unresolved assignments can be repaired after adding artwork or with Undo.");
        }
        ui.separator();
        ui.label("Viewport selection");
        ui.small(format!(
            "{} picked · orange outline. Picking does not change the cut.",
            self.view.artwork.selected.len()
        ));
        ui.horizontal_wrapped(|ui| {
            for (label, action) in [("Use picked", 0), ("Add picked", 1), ("Remove picked", 2)] {
                if button(ui, label, !self.view.artwork.selected.is_empty()).clicked() {
                    let picked = self.view.artwork.selected.clone();
                    // References come from the currently displayed catalogue, never
                    // inferred from names or nearest positions.
                    if picked
                        .iter()
                        .all(|r| self.components.iter().any(|c| &c.reference == r))
                    {
                        self.edit_job(ctx, &[], |job| {
                            let refs = &mut settings_mut(job).components;
                            match action {
                                0 => *refs = picked,
                                1 => {
                                    for r in picked {
                                        if !refs.contains(&r) {
                                            refs.push(r);
                                        }
                                    }
                                }
                                _ => refs.retain(|r| !picked.contains(r)),
                            }
                            Ok(())
                        });
                    }
                }
            }
        });
        ui.separator();
        ui.label("Carving assignment · filled components");
        let count = engine::settings(&self.document.as_ref().unwrap().job)
            .components
            .len();
        ui.small(format!(
            "{count} selected. Cyan = selected; gray = excluded."
        ));
        if button(
            ui,
            "Select all filled components",
            !self.components.is_empty(),
        )
        .clicked()
        {
            let refs = self
                .components
                .iter()
                .map(|c| c.reference.clone())
                .collect();
            self.edit_job(ctx, &[], |job| {
                settings_mut(job).components = refs;
                Ok(())
            });
        }
        if button(ui, "Clear component selection", count > 0).clicked() {
            self.edit_job(ctx, &[], |job| {
                settings_mut(job).components.clear();
                Ok(())
            });
        }
        let unresolved: Vec<_> = engine::settings(&self.document.as_ref().unwrap().job)
            .components
            .iter()
            .filter(|r| !self.components.iter().any(|c| &c.reference == *r))
            .cloned()
            .collect();
        if !unresolved.is_empty() {
            let response = ui.strong("Unresolved assignments");
            observe_control("Unresolved assignments", response.rect);
            ui.small("Pick one current component, then explicitly replace a reference. Reused local IDs do not repair changed sources.");
            for (index, reference) in unresolved.into_iter().enumerate() {
                ui.label(format!(
                    "{} / {} · revision {}",
                    reference.artwork_item_id.0,
                    reference.local_geometry_id,
                    &reference.source_revision.content_digest[..12]
                ));
                ui.horizontal_wrapped(|ui| {
                    if button(
                        ui,
                        &format!("Repair reference {} with picked", index + 1),
                        idle && self.view.artwork.selected.len() == 1,
                    )
                    .clicked()
                    {
                        self.artwork_command(
                            engine::ArtworkCommand::Repair {
                                expected: reference.clone(),
                                replacement: self.view.artwork.selected[0].clone(),
                            },
                            ctx,
                        );
                    }
                    if button(
                        ui,
                        &format!("Remove unresolved reference {}", index + 1),
                        idle,
                    )
                    .clicked()
                    {
                        self.edit_job(ctx, &[], |job| {
                            settings_mut(job).components.retain(|r| r != &reference);
                            Ok(())
                        });
                    }
                });
            }
        }
        for component in self.components.clone() {
            let id = &component.reference.local_geometry_id;
            let mut selected = engine::settings(&self.document.as_ref().unwrap().job)
                .components
                .contains(&component.reference);
            let label = format!("{} / {}", component.reference.artwork_item_id.0, id);
            let r = ui
                .push_id(&label, |ui| ui.checkbox(&mut selected, &label))
                .inner;
            observe_control(&format!("Component {id}"), r.rect);
            observe_control(&format!("Component {label}"), r.rect);
            if r.changed() {
                self.edit_job(ctx, &[], |job| {
                    let refs = &mut settings_mut(job).components;
                    refs.retain(|r| r != &component.reference);
                    if selected {
                        refs.push(component.reference.clone());
                    }
                    Ok(())
                });
            }
            ui.small(format!(
                "[{:.2}, {:.2}] – [{:.2}, {:.2}] mm",
                component.bounds[0], component.bounds[1], component.bounds[2], component.bounds[3]
            ));
        }
    }
    fn setup_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Physical stock");
        if button(
            ui,
            "Stock XY from SVG page",
            self.active.is_none()
                && self
                    .document
                    .as_ref()
                    .is_some_and(|d| d.active_artwork().is_some()),
        )
        .clicked()
        {
            let item = self
                .document
                .as_ref()
                .unwrap()
                .active_artwork()
                .unwrap()
                .id
                .clone();
            self.resource_command(crate::resources::ResourceCommand::StockPage { item }, ctx);
        }
        if let Some(item) = self.document.as_ref().and_then(|d| d.active_artwork()) {
            ui.small(format!(
                "Page source: {} (includes placement and scale)",
                item.name
            ));
        }
        self.numbers(ui, ctx, &[6, 40, 41, 42, 43]);
        if button(ui, "Unset stock XY", true).clicked() {
            self.edit_job(ctx, &[40, 41, 42, 43], |job| {
                job.setup.stock.xy = None;
                Ok(())
            });
        }
        if crate::knife::settings(&self.document.as_ref().unwrap().job).is_some() {
            ui.small("Set actual stock thickness and clearance. Page capture changes only XY.");
        } else {
            ui.small("New SVG jobs use the page size, 18 mm thickness and 5 mm clearance. Adjust these to your actual stock. Page capture changes only XY.");
        }
        ui.separator();
        ui.heading("Work zero");
        help::icon(ui, "Work zero");
        let custom = matches!(
            self.document.as_ref().unwrap().job.setup.work_zero.xy,
            WorkZeroXY::CustomPoint { .. }
        );
        ui.horizontal(|ui| {
            for (label, is_custom) in [("Setup origin", false), ("Custom XY", true)] {
                let r = ui.selectable_label(
                    if is_custom {
                        custom
                    } else {
                        matches!(
                            self.document.as_ref().unwrap().job.setup.work_zero.xy,
                            WorkZeroXY::SetupOrigin
                        )
                    },
                    label,
                );
                observe_control(label, r.rect);
                if r.clicked() {
                    self.edit_job(ctx, &[44, 45], |job| {
                        job.setup.work_zero.xy = if is_custom {
                            WorkZeroXY::CustomPoint { x_mm: 0., y_mm: 0. }
                        } else {
                            WorkZeroXY::SetupOrigin
                        };
                        Ok(())
                    });
                }
            }
        });
        if custom {
            self.numbers(ui, ctx, &[44, 45]);
        }
        ui.horizontal(|ui| {
            for (label, z) in [
                ("Z: stock top", WorkZeroZ::StockTop),
                ("Z: stock bottom", WorkZeroZ::StockBottom),
            ] {
                let r = ui.selectable_label(
                    self.document.as_ref().unwrap().job.setup.work_zero.z == z,
                    label,
                );
                observe_control(label, r.rect);
                if r.clicked() {
                    self.edit_job(ctx, &[], |job| {
                        job.setup.work_zero.z = z;
                        Ok(())
                    });
                }
            }
        });
        ui.small("Work zero affects output coordinates; simulation stays in setup coordinates. Applying a machine does not change this datum.");
        self.numbers(ui, ctx, &[7, 30, 31]);
        if button(ui, "Use default start XY", true).clicked() {
            self.edit_job(ctx, &[30, 31], |job| {
                job.setup.start_xy_mm = Some(cam_core::geometry::Point::new(0., 0.));
                Ok(())
            });
        }
        if button(ui, "Unset start XY", true).clicked() {
            self.edit_job(ctx, &[30, 31], |job| {
                job.setup.start_xy_mm = None;
                Ok(())
            });
        }
    }
    fn job_settings_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Planning tolerances");
        ui.small("Saved with this job and used by all its operations. Changing these values requires regenerating paths; they are independent of machine output precision.");
        self.numbers(ui, ctx, &[23, 25]);
        if button(ui, "Use default planning tolerances", true).clicked() {
            self.edit_job(ctx, &[23, 25], |job| {
                job.tolerances.motion_tolerance_mm = Some(0.01);
                job.tolerances.verification_tolerance_mm = Some(0.05);
                Ok(())
            });
        }
    }
    fn entry_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use cam_core::pocket::EntryStrategy;
        let ramp = matches!(
            engine::settings(&self.document.as_ref().unwrap().job)
                .rough
                .as_ref()
                .unwrap()
                .entry,
            EntryStrategy::Ramp { .. }
        );
        let showing_ramp = ramp || self.operation_ramp_draft;
        help::label(ui, "Endmill entry");
        ui.horizontal(|ui| {
            let r = ui.selectable_label(!showing_ramp, "Plunge entry");
            observe_control("Plunge entry", r.rect);
            if r.clicked() && showing_ramp {
                self.operation_ramp_draft = false;
                // Keep the last ramp numbers as inactive raw fields for recovery
                // and switching back; Plunge does not replace them with defaults.
                let doc = self.document.as_mut().unwrap();
                for f in [14, 51] {
                    let text = doc.text(f);
                    doc.raw.raw.insert(doc.raw.key(f), text);
                }
                self.edit_job(ctx, &[], |job| {
                    settings_mut(job).rough.as_mut().unwrap().entry = EntryStrategy::Plunge;
                    Ok(())
                });
            }
            let r = ui.selectable_label(showing_ramp, "Ramp entry");
            observe_control("Ramp entry", r.rect);
            if r.clicked() && !ramp {
                self.operation_ramp_draft = true;
                self.changed(ctx);
            }
        });
        if showing_ramp || self.operation_ramp_draft {
            self.numbers(ui, ctx, &[14, 51]);
            if self.operation_ramp_draft {
                let doc = self.document.as_ref().unwrap();
                let values = [14, 51].map(|f| Draft::parse(&doc.text(f)).ok().flatten());
                if let [Some(angle), Some(feed)] = values {
                    let mut candidate = doc.job.clone();
                    match authoring::set_group(&mut candidate, 14, &[angle, feed]) {
                        Ok(()) => {
                            self.operation_ramp_draft = false;
                            self.edit_job(ctx, &[], |job| {
                                authoring::set_group(job, 14, &[angle, feed])
                            });
                        }
                        Err(error) => {
                            ui.colored_label(Color32::from_rgb(164, 83, 12), error);
                        }
                    }
                }
            }
            if self.operation_ramp_draft && !ramp {
                ui.colored_label(
                    Color32::from_rgb(164, 83, 12),
                    "Enter ramp angle and feed to complete ramp entry.",
                );
            }
        }
    }
    fn assignment_tool(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, finish: bool) {
        help::label(ui, "Assigned job tool");
        use cam_core::project::ToolGeometry;
        let job = &self.document.as_ref().unwrap().job;
        let assignment = if finish {
            &engine::settings(job).vbit
        } else {
            &engine::settings(job).endmill
        };
        let mut selected = assignment.tool_id.clone();
        let before = selected.clone();
        let current = job
            .tools
            .iter()
            .find(|t| t.id == selected)
            .map(|t| t.name.as_str())
            .unwrap_or("Missing tool");
        let response = egui::ComboBox::from_id_salt(("assignment-tool", finish))
            .selected_text(current)
            .show_ui(ui, |ui| {
                for tool in &job.tools {
                    if matches!(
                        (&tool.geometry, finish),
                        (None, _)
                            | (Some(ToolGeometry::Endmill(_)), false)
                            | (Some(ToolGeometry::Vbit(_)), true)
                    ) {
                        let r = ui.selectable_value(
                            &mut selected,
                            tool.id.clone(),
                            format!("{} · {}", tool.name, tool.id),
                        );
                        observe_control(
                            &format!(
                                "Assign {} {}",
                                if finish { "V-bit" } else { "endmill" },
                                tool.id
                            ),
                            r.rect,
                        );
                    }
                }
            });
        observe_control(
            if finish {
                "V-bit assignment tool"
            } else {
                "Endmill assignment tool"
            },
            response.response.rect,
        );
        if selected != before {
            self.edit_job(
                ctx,
                if finish {
                    &[16, 17, 18, 19, 3, 20, 21, 22, 46, 38, 39]
                } else {
                    &[12, 13, 2, 8, 9, 10, 11, 32, 33]
                },
                |job| authoring::assign_tool(job, finish, &selected),
            );
        }
        let label = if finish {
            "Clear V-bit cutting values"
        } else {
            "Clear endmill cutting values"
        };
        if button(ui, label, true).clicked() {
            self.edit_job(
                ctx,
                if finish {
                    &[3, 20, 21, 22, 46]
                } else {
                    &[2, 8, 9, 10, 11]
                },
                |job| {
                    authoring::clear_assignment(job, finish);
                    Ok(())
                },
            );
        }
        ui.small("Choosing another job tool clears this assignment’s cutting values; enter values for the chosen cutter.");
    }
    fn direction(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, finish: bool) {
        help::label(
            ui,
            if finish {
                "V-bit direction"
            } else {
                "Endmill direction"
            },
        );
        let s = engine::settings(&self.document.as_ref().unwrap().job);
        let mut direction = if finish {
            s.vbit.spindle_direction
        } else {
            s.endmill.spindle_direction
        };
        let before = direction;
        ui.horizontal(|ui| {
            for (name, v) in [
                ("unset", None),
                ("CW", Some(SpindleDirection::Clockwise)),
                ("CCW", Some(SpindleDirection::Counterclockwise)),
            ] {
                let label = format!("{} {name}", if finish { "V-bit" } else { "Endmill" });
                let r = ui.selectable_value(&mut direction, v, &label);
                observe_control(&label, r.rect);
            }
        });
        if direction != before {
            self.edit_job(ctx, &[], |job| {
                let s = settings_mut(job);
                if finish {
                    s.vbit.spindle_direction = direction
                } else {
                    s.endmill.spindle_direction = direction
                };
                Ok(())
            });
        }
    }
    fn machine_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.heading("Applied machine");
        if button(ui, "Create or choose machine profile", true).clicked() {
            self.resources.machines_view = true;
            self.resources.open = true;
            if !self.resources.ready {
                self.request_resources(ResourceIntent::Load, ctx);
            }
        }
        let idle = self.active.is_none() && self.io.is_none();
        if button(ui, "Load machine profile", idle).clicked() {
            self.open(IoKind::Profile, ctx);
        }
        let example = ui.collapsing("Example machine (review fixture)", |ui| {
            if button(ui, "Apply flower machine profile", idle).clicked() {
                self.submit(
                    Command::ApplyProfile {
                        job: self.document.as_ref().unwrap().job.to_json().unwrap(),
                        json: engine::PROFILE.into(),
                    },
                    ctx,
                );
            }
        });
        observe_control("Example machine", example.header_response.rect);
        if let Some(machine) = &self.document.as_ref().unwrap().job.machine_configuration {
            ui.label(&machine.origin.name);
            ui.small(format!(
                "Work offset: {}",
                machine.work_offset.as_deref().unwrap_or("unset")
            ));
            ui.small("T = controller tool number. H = measured tool-length table entry, not a length in mm. A reusable machine profile may not yet map the cutters selected for this job.");
            let table =
                machine.length_compensation == Some(cam_core::post::LengthCompensation::ToolTable);
            self.numbers(ui, ctx, &[7]);
            let job = &self.document.as_ref().unwrap().job;
            let tool_id = crate::knife::settings(job)
                .map(|s| &s.assignment.tool_id)
                .unwrap_or_else(|| &engine::settings(job).endmill.tool_id);
            ui.label(format!(
                "Tool: {}",
                job.tools
                    .iter()
                    .find(|t| &t.id == tool_id)
                    .map(|t| t.name.as_str())
                    .unwrap_or(tool_id)
            ));
            self.numbers(ui, ctx, if table { &[32, 33] } else { &[32] });
            if engine::carving(&self.document.as_ref().unwrap().job)
                .is_some_and(|s| s.mode == FlatVcarveMode::Combined)
            {
                self.numbers(ui, ctx, if table { &[38, 39] } else { &[38] });
            }
            if table && button(ui, "Use T numbers for H entries", true).clicked() {
                self.edit_job(ctx, &[33, 39], |job| {
                    let used = cam_core::project::v5::resources::assignment_statuses(job)
                        .into_iter()
                        .map(|s| s.tool_id)
                        .collect::<Vec<_>>();
                    if let Some(m) = &mut job.machine_configuration {
                        for row in &mut m.tools {
                            if used.contains(&row.job_tool_id) {
                                row.length_offset_number = row.tool_number;
                            }
                        }
                    }
                    Ok(())
                });
            }
            if table {
                ui.small("Use matching T/H only if your controller stores each tool's measured length at that same table number.");
            }
            ui.small("Setup owns the datum. Clearance is shared with Setup.");
            self.applied_machine_options(ui, ctx);
            self.numbers(ui, ctx, &[34, 35]);
            if crate::authoring::active(&self.document.as_ref().unwrap().job, 36) {
                self.numbers(ui, ctx, &[36]);
            }
        } else {
            ui.colored_label(
                Color32::from_rgb(164, 83, 12),
                "Create or choose a machine profile before checked export.",
            );
        }
    }
}
