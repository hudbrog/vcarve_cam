use super::*;
#[path = "face_ui.rs"]
mod face_ui;
#[path = "knife_ui.rs"]
mod knife_ui;
#[path = "operation_ui.rs"]
mod operation_ui;
#[path = "profile_ui.rs"]
mod profile_ui;
#[path = "tool_picker.rs"]
mod tool_picker;
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
            let unit = match field {
                2 | 3 | 10 | 21 | 51 | 63..=65 => "mm/min",
                11 | 22 => "RPM",
                14 | 16 | 28 | 69 | 72 => "deg",
                29 => "×",
                34 | 37 => "s",
                35 => "digits",
                32 | 33 | 38 | 39 | 48..=50 | 52..=56 | 58..=60 => "",
                _ => "mm",
            };
            let response = crate::ui_widgets::number_row(
                ui,
                egui::Id::new((
                    "carving-field",
                    doc.raw.key(field),
                    &doc.raw.operation,
                    field,
                )),
                name,
                &mut text,
                unit,
                FIELDS[field],
            );
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
                let stock = crate::app::StockEditContext {
                    anchor: self.stock_anchor,
                    artwork: self.artwork_bounds(),
                };
                let result = self
                    .document
                    .as_mut()
                    .unwrap()
                    .edit_with(field, text.clone(), stock);
                self.changed(ctx);
                self.status = match result {
                    Ok(()) => "Setting changed; generate to update simulation.".into(),
                    Err(error) => {
                        self.issues = vec![cam_core::operations::LocatedDiagnostic {
                            code: "EDITOR_VALUE".into(),
                            message: error.clone(),
                            operation_id: {
                                let selected = &self.document.as_ref().unwrap().raw.operation;
                                (!selected.is_empty()).then(|| selected.clone())
                            },
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
        if self.inspector_collapsed {
            return;
        }
        let panel=egui::SidePanel::right("gui2-inspector").default_width(self.inspector_width).width_range(320.0..=440.0).resizable(true).show(ctx,|ui|{
            ui.add_space(8.);
            if self.inspector_tab != 2 {
                let title = self.inspector_heading();
                let heading = ui.strong(title.clone());
                // The text itself is the probe: what a job-tool tab is called
                // is exactly what a reviewer needs to check.
                observe_control(&format!("Inspector heading {title}"), heading.rect);
                ui.separator();
            }
            if self.inspector_tab == 2 && self.document.is_some() { self.operation_header(ui,ctx); }
            if self.inspector_tab != 6 {
            let r=ui.add(egui::TextEdit::singleline(&mut self.search).id(egui::Id::new("gui2-search")).char_limit(512).hint_text("Filter fields"));observe_control("Filter fields",r.rect);
            if r.changed(){self.scroll[self.inspector_tab]=0.;if self.inspector_tab==2 {self.operation_scroll[self.operation_tab]=0.;}}
            }
            let operation = self.inspector_tab == 2;
            let offset = if operation { self.operation_scroll[self.operation_tab] } else { self.scroll[self.inspector_tab] };
            let area=egui::ScrollArea::vertical().min_scrolled_height(32.).id_salt(("inspector-scroll",self.inspector_tab,if operation {self.operation_tab} else {0})).auto_shrink([false,!operation]).max_height(if operation { (ui.available_height()-80.).max(48.) } else {ui.available_height()}).vertical_scroll_offset(offset).show(ui,|ui|{
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                if operation { ui.spacing_mut().interact_size.y = 22.; }
                if self.document.is_none(){ui.label("Import an SVG or open a saved job to begin.");return;}
                if self.document.as_ref().unwrap().job.operations.is_empty() {
                    match self.inspector_tab {
                        0 => self.artwork_panel(ui,ctx),
                        1 => self.setup_panel(ui,ctx),
                        3 => self.machine_panel(ui,ctx),
                        7 => self.job_settings_panel(ui,ctx),
                        _ => { ui.heading("No operations"); ui.label("Add an operation from the Operations list. Your artwork, stock, tools and machine settings are retained."); if self.inspector_tab == 0 {self.numbers(ui,ctx,&[26,27,28,29]);} }
                    }
                } else if self.operation_kind() == Some(crate::session::OperationKind::DragKnife) && matches!(self.inspector_tab,0|2|4|5|6) {
                    self.knife_panel(ui,ctx);
                } else if self.operation_kind() == Some(crate::session::OperationKind::Profile) && matches!(self.inspector_tab,0|2|4|5|6) {
                    // One profile editor owns its contours, passes, tabs,
                    // finishing and entries; the artwork, tool-geometry and
                    // machine panels stay the shared ones.
                    self.profile_panel(ui,ctx);
                } else { match self.inspector_tab {0=>self.artwork_panel(ui,ctx),1=>self.setup_panel(ui,ctx),2=>self.cutting_panel(ui,ctx),3=>self.machine_panel(ui,ctx),6=>self.view.inspection_controls(ui),7=>self.job_settings_panel(ui,ctx),index=>{
                    let finish=index==5;
                    self.job_tool_panel(ui, ctx, finish);
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
                ui.small("Changes apply to this job");
                ui.horizontal(|ui| {
                    let ready = !self.operation_ramp_draft && self.document.as_ref().is_some_and(|d| !d.pending() && !d.job.operations.is_empty());
                    let generate = ui.add_enabled(ready && self.active.is_none() && self.io.is_none(),egui::Button::new("Generate all"));
                    observe_control("Generate operation",generate.rect);
                    if generate.clicked() {
                        self.generate(crate::session::GenerateScope::AllEnabled, ctx);
                    }
                    let id = self.operation_id();
                    let through = ui.add_enabled(ready && self.active.is_none() && self.io.is_none(), egui::Button::new("Generate through here"));
                    observe_control("Generate through here", through.rect);
                    if through.clicked() && !id.is_empty() {
                        self.generate(crate::session::GenerateScope::ThroughOperation { operation_id: id }, ctx);
                    }
                });
            } else { self.scroll[self.inspector_tab]=area.state.offset.y; }
        });
        self.inspector_width = panel.response.rect.width();
    }
    /// The inspector's heading. A job-tool tab names the tool the selected
    /// operation actually addresses, so a library copy reports its own name
    /// and a fresh operation reports that nothing is chosen yet.
    fn inspector_heading(&self) -> String {
        if !matches!(self.inspector_tab, 4 | 5) {
            return [
                "ARTWORK",
                "STOCK & WORK ZERO",
                "OPERATION",
                "MACHINE",
                "",
                "",
                "RESULT INSPECTION",
                "JOB SETTINGS",
            ][self.inspector_tab]
                .into();
        }
        let Some(tab) = crate::resources::tool_tab(self.operation_kind(), self.inspector_tab == 5)
        else {
            // Only a Flat V-carve operation owns a V-bit stage.
            return "JOB TOOL · V-BIT".into();
        };
        match self.assigned_tool(tab.role) {
            Some(tool) => format!("JOB TOOL · {}", tool.label()),
            None => format!("JOB TOOL · {}", crate::resources::role_word(tab.role)),
        }
    }

    /// The geometry tab of one job tool. The assignment belongs to an
    /// operation, so the panel names the operation that uses it and edits only
    /// that operation's tool snapshot.
    fn job_tool_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, finish: bool) {
        let id = self.operation_id();
        let kind = self.operation_kind();
        let label = crate::session::kind_label(&self.document.as_ref().unwrap().job, &id);
        if finish && kind != Some(crate::session::OperationKind::FlatVcarve) {
            ui.heading("V-bit geometry");
            self.numbers(ui, ctx, &[16, 17, 18, 19]);
            ui.separator();
            ui.label(format!("{label} does not use a V-bit."));
            return;
        }
        // A Flat V-carve spends an endmill and a V-bit; a Face operation spends
        // one cutter. Name the tool this assignment actually addresses rather
        // than the placeholder it was created with.
        let tab = crate::resources::tool_tab(kind, finish).expect("a cutter tab exists here");
        let assigned = self.assigned_tool(tab.role);
        ui.heading(match &assigned {
            Some(tool) => format!("{} geometry · {}", tab.noun, tool.tool_label()),
            None => format!("{} geometry", tab.noun),
        });
        if let Some(tool) = &assigned {
            ui.small(format!("Profile: {}", tool.profile_label()));
            if let Some(origin) = tool.origin_label() {
                ui.small(origin);
            }
        }
        self.numbers(ui, ctx, if finish { &[16, 17, 18, 19] } else { &[12, 13] });
        if button(
            ui,
            if finish {
                "Unset V-bit geometry"
            } else {
                "Unset endmill geometry"
            },
            true,
        )
        .clicked()
        {
            self.edit_job(
                ctx,
                if finish { &[16, 17, 18, 19] } else { &[12, 13] },
                move |job| {
                    authoring::tool_mut_in(job, &id, finish)?.geometry = None;
                    Ok(())
                },
            );
        }
        ui.separator();
        ui.label(format!("Used by {label}"));
        ui.small(if finish {
            "Defines the V-shaped target in both modes; executes finishing in Combined mode."
        } else if kind == Some(crate::session::OperationKind::DragKnife) {
            "Passive blade geometry: offset and maximum cutting depth."
        } else if kind == Some(crate::session::OperationKind::Profile) {
            "Cutter used by the profile's rough and finishing passes."
        } else {
            "Clearing and facing cutter"
        });
        if button(ui, "Edit cutting assignment", true).clicked() {
            self.navigate(2);
        }
        if button(ui, "Edit controller mapping", true).clicked() {
            self.navigate(3);
        }
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
            ui.small("SVG units become mm. The page is flipped once about its physical height, so the page's top edge is the stock's maximum Y and the page's bottom-left corner is the setup origin; after that, placement = scale × rotate(artwork − origin).");
            self.numbers(ui, ctx, &[26, 27, 28, 29]);
        } else {
            ui.label("Add an SVG to this project. Unresolved selections can be repaired after adding artwork or with Undo.");
        }
        ui.separator();
        ui.label("Geometry selection");
        ui.small("Which geometry gets cut belongs to the operation, not to the artwork. Select filled components under Cutting → Geometry to carve, or click them in the viewport. This panel keeps placement, hide/lock and source management only.");
        if button(ui, "Open operation geometry", self.document.is_some()).clicked() {
            self.operation_tab = 0;
            self.navigate(2);
        }
    }
    /// Name every artwork item that now reaches past the stock rectangle. The
    /// fix is always the stock or the item's placement, chosen by the user:
    /// nothing here moves anything.
    fn stock_outside_notice(&mut self, ui: &mut egui::Ui) {
        let Some(stock) = self.document.as_ref().and_then(|d| d.job.setup.stock.xy) else {
            return;
        };
        let outside: Vec<String> = self
            .placed_artwork_bounds()
            .into_iter()
            .filter_map(|(name, bounds)| {
                cam_core::project::v5::commands::stock_overhang(bounds, stock)
                    .map(|(side, overhang)| format!("{name} ({overhang:.2} mm past {side})"))
            })
            .collect();
        if outside.is_empty() {
            return;
        }
        let r = ui.colored_label(
            Color32::from_rgb(176, 42, 35),
            format!(
                "Outside the stock: {}. Nothing was moved; use Stock from artwork bounds, or a resize anchor that re-centres the stock.",
                outside.join(", ")
            ),
        );
        observe_control("Stock artwork outside", r.rect);
    }
    /// The W5 resize anchor: which point of the stock rectangle a width or
    /// length edit keeps still. Artwork never moves, whatever is chosen here.
    fn stock_anchor_controls(&mut self, ui: &mut egui::Ui) {
        use cam_core::project::v5::commands::StockAnchor;
        help::label(ui, "Stock resize anchor");
        ui.horizontal_wrapped(|ui| {
            for anchor in StockAnchor::ALL {
                let r = ui.selectable_label(self.stock_anchor == anchor, anchor.label());
                observe_control(&format!("Stock anchor {}", anchor.label()), r.rect);
                if r.clicked() {
                    self.stock_anchor = anchor;
                }
            }
        });
        ui.small(self.stock_anchor.help()).on_hover_text(
            "Width and length edits keep this anchor fixed. Artwork is never moved.",
        );
    }
    fn setup_panel(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        crate::ui_widgets::scope(ui, "This job · setup coordinates");
        crate::ui_widgets::section(ui, "Dimensions", Some(crate::ui_icons::Icon::Stock));
        self.numbers(ui, ctx, &[42, 43, 6]);
        crate::ui_widgets::section(ui, "Position", None);
        self.numbers(ui, ctx, &[40, 41]);
        self.stock_anchor_controls(ui);
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
        let has_artwork = self
            .document
            .as_ref()
            .is_some_and(|d| !d.job.artwork.is_empty());
        if button(
            ui,
            "Stock from artwork bounds",
            self.active.is_none() && has_artwork,
        )
        .clicked()
        {
            self.resource_command(crate::resources::ResourceCommand::StockArtworkBounds, ctx);
        }
        if let Some(item) = self.document.as_ref().and_then(|d| d.active_artwork()) {
            ui.small(format!(
                "Page source: {} (includes placement and scale)",
                item.name
            ));
        }
        self.stock_outside_notice(ui);
        if button(ui, "Unset stock XY", true).clicked() {
            self.edit_job(ctx, &[40, 41, 42, 43], |job| {
                job.setup.stock.xy = None;
                Ok(())
            });
        }
        crate::ui_widgets::section(ui, "Work zero", None);
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
        // The Z datum decides the surface the first plunge is measured from.
        // It is a single choice with a physical consequence, so it is stated
        // here in the machine's terms instead of left to the operator to
        // reconstruct (field-test finding 1.2).
        help::label(ui, "Z datum");
        ui.horizontal(|ui| {
            for (label, z) in [
                ("Z0: stock top", WorkZeroZ::StockTop),
                ("Z0: stock bottom", WorkZeroZ::StockBottom),
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
        let setup = &self.document.as_ref().unwrap().job.setup;
        crate::ui_widgets::stock_datum(
            ui,
            setup.stock.thickness_mm,
            setup.clearance_above_stock_mm,
            setup.work_zero.z == WorkZeroZ::StockBottom,
        );
        ui.small("Touch off the machine on the selected Z0 surface. Work zero changes output coordinates; simulation uses setup coordinates.");
        crate::ui_widgets::section(ui, "Clearance & start", None);
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
            self.open_resource(ResourcePage::MachineLibrary);
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
            let operation_id = self
                .document
                .as_ref()
                .map(|d| d.raw.operation.clone())
                .unwrap_or_default();
            // The mapping shown belongs to the selected operation's own tool,
            // whatever kind that operation is.
            let tool_id = crate::knife::settings_in(job, &operation_id)
                .map(|s| &s.assignment.tool_id)
                .or_else(|| {
                    crate::authoring::tool_in(job, &operation_id, false).map(|tool| &tool.id)
                });
            ui.label(format!(
                "Tool: {}",
                tool_id
                    .and_then(|id| job.tools.iter().find(|t| &t.id == id))
                    .map(|t| format!("{} · {}", t.name, t.id))
                    .unwrap_or_else(|| "no cutter assigned".into())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::operation_authoring::{self, Kind};
    use cam_core::project::v5::resources::AssignmentRole;

    fn profile_app() -> App {
        let empty = operation_authoring::empty_job();
        let job =
            operation_authoring::apply(&empty, operation_authoring::add(Kind::Profile, &empty))
                .unwrap();
        App {
            document: Some(Document::new(job)),
            // The Profile operation's cutter tab.
            inspector_tab: 4,
            ..Default::default()
        }
    }

    /// The rendered job-tool heading, read back from the input probe.
    fn headings(app: &mut App, ctx: &egui::Context) -> Vec<String> {
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
        CONTROLS.with(|c| {
            c.borrow()
                .keys()
                .filter(|key| key.starts_with("Inspector heading "))
                .cloned()
                .collect()
        })
    }

    #[test]
    fn the_job_tool_tab_names_the_tool_and_profile_the_assignment_uses() {
        let ctx = egui::Context::default();
        let mut app = profile_app();
        assert_eq!(
            headings(&mut app, &ctx),
            ["Inspector heading JOB TOOL · no tool chosen · custom"]
        );
        // Apply a library tool and cutting profile exactly as the picker does.
        let operation = app.document.as_ref().unwrap().job.operations[0].id.clone();
        let library =
            crate::resources::Catalog::decode(include_str!("../../../fixtures/gui5/library.json"))
                .unwrap();
        let applied = crate::resources::ResourceCommand::ApplyToolProfile {
            catalog: library,
            tool: "endmill".into(),
            preset: "rough".into(),
            operation,
            role: AssignmentRole::Milling,
        }
        .execute(&app.document.as_ref().unwrap().job)
        .unwrap();
        app.document = Some(Document::new(applied));
        assert_eq!(
            headings(&mut app, &ctx),
            ["Inspector heading JOB TOOL · Endmill · Lettering rough"]
        );
    }

    #[test]
    fn the_setup_tab_shows_the_z_datum_as_its_own_documented_choice() {
        // Field-test finding 1.2: the active Z datum must be a named control
        // with help, not a pair of labels buried under the stock numbers.
        let empty = operation_authoring::empty_job();
        let job = operation_authoring::apply(&empty, operation_authoring::add(Kind::Face, &empty))
            .unwrap();
        let mut app = App {
            document: Some(Document::new(job)),
            inspector_tab: 1,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        for label in ["Z datum", "Z0: stock top", "Z0: stock bottom"] {
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
            let present = CONTROLS.with(|c| {
                let c = c.borrow();
                c.contains_key(label) || c.contains_key(&format!("Help {label}"))
            });
            assert!(present, "{label} missing from the setup tab");
        }
        assert!(help::explanation("Z datum").is_some_and(|s| s.len() > 30));
        let labels = controls(&mut app, &ctx);
        assert!(!labels.contains("Z: stock top"));
        assert!(!labels.contains("Z: stock bottom"));
        assert!(labels.contains("Stock datum diagram"));
    }

    #[test]
    fn stock_number_rows_stay_compact_at_different_window_heights() {
        for height in [800., 900., 1080.] {
            let mut app = App {
                document: Some(Document::new(artwork_job())),
                inspector_tab: 1,
                ..Default::default()
            };
            let ctx = egui::Context::default();
            App::theme(&ctx);
            for _ in 0..3 {
                CONTROLS.with(|c| c.borrow_mut().clear());
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280., height),
                        )),
                        ..Default::default()
                    },
                    |ctx| app.inspector(ctx),
                );
            }
            CONTROLS.with(|c| {
                let controls = c.borrow();
                let width = controls["Stock width"];
                let length = controls["Stock length"];
                let thickness = controls["Stock thickness"];
                assert!(width[1] < 180., "first input pushed down: {width:?}");
                for row in [width, length, thickness] {
                    assert!(row[3] - row[1] <= 30., "unbounded input: {row:?}");
                    assert!(row[0] >= 1280. - crate::ui_theme::INSPECTOR && row[2] < 1280.);
                }
                assert!((28.0..=40.).contains(&(length[1] - width[1])));
                assert!((28.0..=40.).contains(&(thickness[1] - length[1])));
            });
        }
    }

    /// Every control label the last few rendered frames published.
    fn controls(app: &mut App, ctx: &egui::Context) -> std::collections::BTreeSet<String> {
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
        CONTROLS.with(|c| c.borrow().keys().cloned().collect())
    }

    /// A job with one imported artwork item and its page-sized stock.
    fn artwork_job() -> CamJobV5 {
        crate::authoring::import_svg(
            "letters.svg".into(),
            include_str!("../../../fixtures/gui3/lettering.svg").into(),
        )
        .unwrap()
    }

    #[test]
    fn the_setup_tab_names_the_resize_anchor_and_offers_the_stock_fit() {
        // W5 finding 3.3: a stock resize needs a stated anchor, and the fix
        // for geometry left outside it has to be next to the stock numbers.
        let mut app = App {
            document: Some(Document::new(artwork_job())),
            inspector_tab: 1,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let labels = controls(&mut app, &ctx);
        for label in [
            "Stock resize anchor",
            "Stock anchor Min corner",
            "Stock anchor Stock centre",
            "Stock anchor Artwork bounds",
            "Stock from artwork bounds",
        ] {
            assert!(
                labels.contains(label) || labels.contains(&format!("Help {label}")),
                "{label} missing from the setup tab: {labels:?}"
            );
        }
        assert!(help::explanation("Stock resize anchor").is_some_and(|s| s.len() > 30));
        // Artwork inside the stock says nothing.
        assert!(!labels.contains("Stock artwork outside"));
    }

    #[test]
    fn artwork_left_outside_a_shrunk_stock_is_named_next_to_the_numbers() {
        let mut app = App {
            document: Some(Document::new(artwork_job())),
            inspector_tab: 1,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        // Shrink the stock to a tenth of its width and length, min-corner
        // anchored: the artwork no longer fits, and nothing moves it.
        let document = app.document.as_mut().unwrap();
        document
            .edit(42, "20".into())
            .expect("a size edit is legal");
        document
            .edit(43, "10".into())
            .expect("a size edit is legal");
        // The workspace resolves the artwork bounds from the last artwork
        // reply; this test supplies the same bounds the importer measured.
        let bounds = crate::authoring::catalogue_components(
            &cam_core::project::v5::inspect_artwork(&document.job).unwrap(),
        );
        assert!(!bounds.is_empty());
        app.components = bounds;
        let labels = controls(&mut app, &ctx);
        assert!(
            labels.contains("Stock artwork outside"),
            "the outside-stock notice must be visible: {labels:?}"
        );
        // The notice is a report: the artwork is exactly where it was.
        let placement = app.document.as_ref().unwrap().job.artwork[0]
            .placement
            .clone();
        assert_eq!(placement, artwork_job().artwork[0].placement);
    }

    #[test]
    fn a_stock_size_edit_keeps_the_chosen_anchor_and_never_the_artwork() {
        use cam_core::project::v5::commands::StockAnchor;
        let job = artwork_job();
        let page = job.setup.stock.xy.unwrap();
        let placement = job.artwork[0].placement.clone();
        // The default is today's behaviour: the minimum corner never moves.
        let mut default = Document::new(job.clone());
        default.edit(42, "50".into()).unwrap();
        let rect = default.job.setup.stock.xy.unwrap();
        assert_eq!(
            (rect.min_x_mm, rect.min_y_mm),
            (page.min_x_mm, page.min_y_mm)
        );
        assert_eq!(rect.width_mm, 50.);
        // Centre: the rectangle's own centre stays, so the corner follows.
        let mut centred = Document::new(job.clone());
        centred
            .edit_with(
                42,
                "50".into(),
                StockEditContext {
                    anchor: StockAnchor::Centre,
                    artwork: None,
                },
            )
            .unwrap();
        let rect = centred.job.setup.stock.xy.unwrap();
        assert!((rect.min_x_mm - (page.min_x_mm + (page.width_mm - 50.) / 2.)).abs() < 1e-9);
        // Artwork bounds: the placed artwork stays centred in the stock.
        let artwork = cam_core::project::v5::SetupBounds {
            min_x_mm: 10.,
            min_y_mm: 20.,
            max_x_mm: 30.,
            max_y_mm: 40.,
        };
        let mut anchored = Document::new(job.clone());
        anchored
            .edit_with(
                43,
                "50".into(),
                StockEditContext {
                    anchor: StockAnchor::ArtworkBounds,
                    artwork: Some(artwork),
                },
            )
            .unwrap();
        let rect = anchored.job.setup.stock.xy.unwrap();
        assert!((rect.min_y_mm - (30. - 25.)).abs() < 1e-9);
        // An explicit corner edit is taken as written, whatever the anchor is.
        let mut corner = Document::new(job.clone());
        corner
            .edit_with(
                40,
                "5".into(),
                StockEditContext {
                    anchor: StockAnchor::ArtworkBounds,
                    artwork: Some(artwork),
                },
            )
            .unwrap();
        assert_eq!(corner.job.setup.stock.xy.unwrap().min_x_mm, 5.);
        // Every anchor moved the rectangle and left the artwork alone.
        for document in [&default, &centred, &anchored, &corner] {
            assert_eq!(document.job.artwork[0].placement, placement);
        }
        // The field being typed into keeps its own spelling, so a partially
        // typed number is not rewritten under the cursor.
        let mut typed = Document::new(job.clone());
        typed.edit(42, "70.50".into()).unwrap();
        assert_eq!(typed.text(42), "70.50");
        assert_eq!(typed.job.setup.stock.xy.unwrap().width_mm, 70.5);
    }
}
