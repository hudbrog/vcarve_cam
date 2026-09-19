//! The applied machine's job-wide tool mapping. Drafts follow tool IDs, so a
//! shared cutter has one T/H editor regardless of the selected operation.
use super::*;

fn mapping_key(tool: &str, length: bool) -> String {
    format!("mapping/{tool}/{}", FIELDS[if length { 33 } else { 32 }])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> Document {
        let mut job =
            CamJobV5::from_json(include_str!("../../../fixtures/gui4/lettering.job.json")).unwrap();
        let mut second = job.operations[0].clone();
        second.id = "second-carve".into();
        job.operations.push(second);
        let profile =
            cam_core::post::sequence::SequenceProfile::from_json(engine::PROFILE).unwrap();
        job =
            cam_core::project::v5::machine::apply_machine_configuration(&job, &profile, "Workshop")
                .unwrap()
                .job;
        job.machine_configuration
            .as_mut()
            .unwrap()
            .length_compensation = Some(cam_core::post::LengthCompensation::ToolTable);
        Document::new(job)
    }

    #[test]
    fn mapping_drafts_follow_shared_tools_and_recover_without_an_operation() {
        let mut doc = document();
        doc.edit_mapping("endmill", false, "7".into()).unwrap();
        assert!(doc.edit_mapping("endmill", true, "-".into()).is_err());
        doc.raw.operation = "second-carve".into();
        assert_eq!(doc.text(32), "7");
        assert_eq!(doc.text(33), "-");
        assert_eq!(doc.mapping_value("vbit", false), Some(2.));
        assert!(doc.pending());
        doc.job.operations.clear();
        doc.raw.operation.clear();
        let snapshot = doc.snapshot();
        snapshot.validate().unwrap();
        let draft = Draft::recover(&serde_json::to_string(&snapshot.draft).unwrap()).unwrap();
        draft.validate_job(&doc.job).unwrap();
        doc.raw = draft;
        assert_eq!(doc.mapping_text("endmill", true), "-");
        assert!(doc.pending());
        doc.job
            .machine_configuration
            .as_mut()
            .unwrap()
            .length_compensation = Some(cam_core::post::LengthCompensation::MacroManaged);
        assert!(!doc.pending());
        for invalid in ["0", "1.5", "1.", "4294967296"] {
            assert!(doc.edit_mapping("endmill", false, invalid.into()).is_err());
            assert_eq!(doc.mapping_value("endmill", false), Some(7.));
            assert_eq!(doc.mapping_text("endmill", false), invalid);
            assert!(doc.pending());
        }
    }

    #[test]
    fn legacy_mapping_drafts_are_visible_and_replaced_only_for_the_same_tool_and_column() {
        let mut doc = document();
        doc.raw.operation = "second-carve".into();
        let old = doc.raw.key(32);
        doc.raw.raw.insert(old.clone(), "-".into());
        doc.raw.raw.insert(doc.raw.key(33), "3.".into());
        doc.raw.raw.insert(doc.raw.key(38), "4.".into());
        doc.raw.operation = doc.job.operations[0].id.clone();
        assert_eq!(doc.text(32), "-");
        assert!(doc.pending());
        doc.edit_mapping("endmill", false, "9".into()).unwrap();
        assert!(!doc.raw.raw.contains_key(&old));
        assert_eq!(doc.mapping_text("endmill", true), "3.");
        assert_eq!(doc.mapping_text("vbit", false), "4.");
        doc.clear_tool_mapping_draft("endmill", true);
        assert_eq!(doc.mapping_text("vbit", false), "4.");
        doc.clear_mapping_drafts(false);
        assert!(doc.raw.raw.is_empty());
    }

    #[test]
    fn applying_machine_clears_all_mapping_drafts_preserves_datum_and_undo_restores_them() {
        let ctx = egui::Context::default();
        let mut doc = document();
        doc.edit_mapping("endmill", false, "-".into()).unwrap_err();
        doc.raw.operation = "second-carve".into();
        doc.raw.raw.insert(doc.raw.key(39), "2.".into());
        doc.raw.operation = doc.job.operations[0].id.clone();
        let prior = doc.raw.clone();
        let datum = doc.job.setup.work_zero.clone();
        let scene = engine::run(Command::ApplyProfile {
            job: doc.job.to_json().unwrap(),
            json: engine::PROFILE.into(),
        })
        .unwrap();
        let mut app = App {
            document: Some(doc),
            active: Some((1, 0)),
            ..Default::default()
        };
        app.accept(1, Ok(scene), &ctx);
        let applied = app.document.as_ref().unwrap();
        assert_eq!(applied.job.setup.work_zero, datum);
        assert!(applied.raw.raw.is_empty());
        assert!(!applied.pending());
        app.undo(&ctx);
        assert_eq!(app.document.as_ref().unwrap().raw, prior);
        let recovery = app.recovery_snapshot().unwrap();
        let mut restored = App::default();
        restored.restore(recovery, &ctx);
        assert_eq!(restored.document.as_ref().unwrap().raw, prior);
        restored.redo(&ctx);
        assert!(restored.document.as_ref().unwrap().raw.raw.is_empty());
    }
}

impl Document {
    pub(crate) fn mapping_value(&self, tool: &str, length: bool) -> Option<f64> {
        let row = self
            .job
            .machine_configuration
            .as_ref()?
            .tools
            .iter()
            .find(|row| row.job_tool_id == tool)?;
        if length {
            row.length_offset_number
        } else {
            row.tool_number
        }
        .map(f64::from)
    }

    pub(crate) fn legacy_mapping_tool(&self, key: &str, length: bool) -> Option<&str> {
        let (scope, operation, label) = crate::state::scope_of(key)?;
        let field = FIELDS.iter().position(|name| *name == label)?;
        if scope != "job"
            || !matches!(field, 32 | 33 | 38 | 39)
            || matches!(field, 33 | 39) != length
        {
            return None;
        }
        authoring::tool_in(&self.job, operation, field >= 38).map(|t| t.id.as_str())
    }

    pub(crate) fn mapping_text(&self, tool: &str, length: bool) -> String {
        self.raw
            .raw
            .get(&mapping_key(tool, length))
            .cloned()
            // Older recoveries keep these values under operation field keys.
            // Read them without rewriting the user's retained spelling.
            .or_else(|| {
                self.raw
                    .raw
                    .iter()
                    .find(|(key, _)| self.legacy_mapping_tool(key, length) == Some(tool))
                    .map(|(_, text)| text.clone())
            })
            .unwrap_or_else(|| {
                self.mapping_value(tool, length)
                    .map(|n| n.to_string())
                    .unwrap_or_default()
            })
    }

    pub(crate) fn edit_mapping(
        &mut self,
        tool: &str,
        length: bool,
        text: String,
    ) -> Result<(), String> {
        if !self.job.tools.iter().any(|t| t.id == tool) {
            return Err("This job tool no longer exists".into());
        }
        let legacy = self
            .raw
            .raw
            .keys()
            .filter(|key| self.legacy_mapping_tool(key, length) == Some(tool))
            .cloned()
            .collect::<Vec<_>>();
        for key in legacy {
            self.raw.raw.remove(&key);
        }
        self.raw.raw.insert(mapping_key(tool, length), text.clone());
        let number = Draft::parse(&text)
            .map_err(str::to_string)?
            .map(|n| {
                if n >= 0. && n <= u32::MAX as f64 && n.fract() == 0. {
                    Ok(n as u32)
                } else {
                    Err("Mapping must be a whole nonnegative number".to_string())
                }
            })
            .transpose()?;
        let (t, h) = if length {
            (self.mapping_value(tool, false).map(|n| n as u32), number)
        } else {
            (number, self.mapping_value(tool, true).map(|n| n as u32))
        };
        self.job = cam_core::project::v5::machine::set_tool_mapping(&self.job, tool, t, h)
            .map_err(|e| e.to_string())?
            .job;
        Ok(())
    }

    pub(crate) fn clear_mapping_drafts(&mut self, length_only: bool) {
        self.raw.raw.retain(|key, _| {
            let Some((scope, _, label)) = crate::state::scope_of(key) else {
                return true;
            };
            let mapping = matches!(
                label,
                "Tool number" | "Length offset" | "V-bit tool number" | "V-bit length offset"
            );
            !((scope == "mapping" || scope == "job")
                && mapping
                && (!length_only || label.contains("offset")))
        });
    }

    pub(crate) fn clear_tool_mapping_draft(&mut self, tool: &str, length: bool) {
        let keys = self
            .raw
            .raw
            .keys()
            .filter(|key| {
                **key == mapping_key(tool, length)
                    || self.legacy_mapping_tool(key, length) == Some(tool)
            })
            .cloned()
            .collect::<Vec<_>>();
        for key in keys {
            self.raw.raw.remove(&key);
        }
    }
}

impl App {
    pub(super) fn applied_machine_contract(
        &self,
        ui: &mut egui::Ui,
        machine: &cam_core::project::v5::AppliedMachineConfiguration,
    ) {
        use cam_core::post::M6Return;
        ui.separator();
        ui.strong("Startup & tool change");
        ui.small(
            machine
                .program_start_position_mm
                .map(|p| format!("Start XYZ: {}, {}, {} mm", p.x, p.y, p.z))
                .unwrap_or_else(|| "Startup position: owned by the machine macro".into()),
        );
        if let Some(m6) = &machine.m6 {
            ui.small(format!(
                "M6 contract: {} · {}",
                m6.reference,
                if m6.reviewed {
                    "reviewed"
                } else {
                    "not reviewed"
                }
            ));
            ui.small(match m6.return_position {
                M6Return::CallerPosition => "M6 returns to the caller's position".into(),
                M6Return::FixedPosition { position_mm: p } => {
                    format!("M6 return XYZ: {}, {}, {} mm", p.x, p.y, p.z)
                }
                M6Return::SafeRetract {
                    z_mm,
                    transit_xy_mm: p,
                } => format!(
                    "M6 safe retract: Z {z_mm} mm, transit X {}, Y {} mm",
                    p.x, p.y
                ),
            });
            for (label, guaranteed) in [
                ("Preserves work datum", m6.preserves_work_datum),
                ("No local offsets", m6.local_offsets_unused),
                ("Tool offsets are Z only", m6.tool_offsets_z_only),
            ] {
                ui.small(format!(
                    "{label}: {}",
                    if guaranteed { "stated" } else { "not stated" }
                ));
            }
        } else {
            ui.colored_label(crate::ui_theme::WARNING, "M6 contract is not stated.");
        }
        ui.separator();
        ui.strong("Simulation assumptions");
        ui.small(
            machine
                .rapid_rate_mm_min
                .map(|rate| format!("Rapid rate: {rate} mm/min"))
                .unwrap_or_else(|| "Rapid rate: unstated; simulation reports its fallback".into()),
        );
        if let Some(holder) = &machine.holder {
            ui.small(format!("Holder: {}", holder.id));
            if !holder.segments.is_empty() {
                ui.small(format!(
                    "{} custom segments · read-only here",
                    holder.segments.len()
                ));
                for (i, segment) in holder.segments.iter().enumerate() {
                    ui.small(format!(
                        "{}: height {} mm · lower Ø {} mm · upper Ø {} mm",
                        i + 1,
                        segment.height_mm,
                        segment.lower_diameter_mm,
                        segment.upper_diameter_mm
                    ));
                }
            }
        } else {
            ui.small("Holder: unstated");
        }
        ui.small("Review startup, M6, rapid rate and holder in the reusable machine settings, then explicitly apply the reviewed profile.");
    }

    pub(super) fn machine_mapping_table(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let doc = self.document.as_ref().unwrap();
        let tools = doc.job.tools.clone();
        let table = doc.job.machine_configuration.as_ref().is_some_and(|m| {
            m.length_compensation == Some(cam_core::post::LengthCompensation::ToolTable)
        });
        let uses = cam_core::project::v5::resources::assignment_statuses(&doc.job);
        let active = [false, true].map(|finish| {
            authoring::tool_in(&doc.job, &doc.raw.operation, finish).map(|t| t.id.clone())
        });
        ui.small("T: controller tool number · H: measured length table entry");
        if !table {
            ui.small("H is inactive unless compensation uses the tool table.");
        }
        let name_width = (ui.available_width() - 140.).max(80.);
        ui.horizontal(|ui| {
            ui.add_sized(
                [name_width, 24.],
                egui::Label::new(RichText::new("Job tool").strong()),
            );
            ui.add_sized([62., 24.], egui::Label::new(RichText::new("T").strong()));
            ui.add_sized([62., 24.], egui::Label::new(RichText::new("H").strong()));
        });
        let mapping_list = egui::ScrollArea::vertical().id_salt("machine-tool-mappings")
            .max_height(220.).auto_shrink([false, true]).show(ui, |ui| {
        for (index, tool) in tools.iter().enumerate() {
            if !self.search.is_empty()
                && !tool
                    .name
                    .to_lowercase()
                    .contains(&self.search.to_lowercase())
                && ![32, 33, 38, 39].iter().any(|f| {
                    FIELDS[*f]
                        .to_lowercase()
                        .contains(&self.search.to_lowercase())
                })
                && !format!("Mapping {} T Mapping {} H", tool.id, tool.id).to_lowercase().contains(&self.search.to_lowercase())
            {
                continue;
            }
            ui.push_id(("machine-mapping", &tool.id), |ui| {
                ui.horizontal(|ui| {
                    ui.add_sized([name_width, 28.], egui::Label::new(&tool.name).truncate()).on_hover_text(&tool.id);
                    for length in [false, true] {
                        let doc = self.document.as_ref().unwrap();
                        let mut raw = doc.mapping_text(&tool.id, length);
                        let pending = Draft::parse(&raw).ok() != Some(doc.mapping_value(&tool.id, length));
                        let response = ui.add_enabled_ui(!length || table, |ui| ui.add_sized([62., 28.],
                            egui::TextEdit::singleline(&mut raw).id(egui::Id::new(("mapping-field", &tool.id, length)))
                                .desired_width(62.).char_limit(128).hint_text(if length && !table { "N/A" } else { "Unset" })
                                .text_color(if pending { crate::ui_theme::ERROR } else { crate::ui_theme::TEXT })
                        )).inner.on_hover_text(if length { "Measured tool-length table entry (H), not a length in mm" } else { "Controller tool number (T)" });
                        let label = format!("Mapping {} {}", tool.id, if length { "H" } else { "T" });
                        observe_control(&label, response.rect);
                        for (finish, assigned) in active.iter().enumerate() {
                            if assigned.as_deref() == Some(&tool.id) {
                                let field = if finish == 1 { 38 } else { 32 } + usize::from(length);
                                observe_control(FIELDS[field], response.rect);
                                if self.issue_focus.as_deref() == Some(FIELDS[field]) {
                                    response.scroll_to_me(Some(egui::Align::Center));
                                    response.request_focus();
                                    self.issue_focus = None;
                                }
                            }
                        }
                        if response.changed() {
                            // Transaction grouping only; widget/recovery IDs
                            // above use the stable tool, never this row index.
                            let group = FIELDS.len() + index * 2 + usize::from(length);
                            if self.edit_group != Some(group) { self.remember(); self.edit_group = Some(group); }
                            let result = self.document.as_mut().unwrap().edit_mapping(&tool.id, length, raw);
                            self.changed(ctx);
                            self.status = result.err().unwrap_or_else(|| "Tool mapping changed for this job. Regenerate before checked output.".into());
                        }
                        if response.lost_focus() { self.edit_group = None; }
                    }
                });
                let count = uses.iter().filter(|u| u.tool_id == tool.id).count();
                ui.small(if count == 0 { "Not assigned to an operation".into() } else { format!("Used by {count} assignment{}", if count == 1 { "" } else { "s" }) });
            });
        }
        });
        observe_control("Machine mapping viewport", mapping_list.inner_rect);
        if tools.is_empty() {
            ui.label("Add or assign a job tool to map its controller number.");
        }
        if let Some(machine) = &self.document.as_ref().unwrap().job.machine_configuration {
            for row in &machine.tools {
                if !tools.iter().any(|t| t.id == row.job_tool_id) {
                    ui.colored_label(
                        crate::ui_theme::WARNING,
                        format!("Unresolved mapping: {}", row.job_tool_id),
                    );
                }
            }
        }
    }
}
