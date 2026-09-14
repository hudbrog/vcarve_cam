//! How one piece of source geometry is named, grouped and coloured in the
//! pickers.
//!
//! Two shapes that differ only in colour or layer describe the same cutting
//! geometry, so neither is ever used to *decide* anything. They are how the
//! person recognises their own drawing: a carving and the outline of the part
//! arrive from one file, and the pickers have to let the two be told apart and
//! taken in one action.
use super::*;
use cam_core::svg::SourcePaint;
use std::collections::BTreeSet;

/// One row a picker offers: the value it selects, and how the drawing named it.
pub struct SourceRow<'a, T> {
    pub value: T,
    /// The element's own `inkscape:label`.
    pub label: Option<&'a str>,
    /// The nearest enclosing named layer or group.
    pub group: Option<&'a str>,
    /// The colour the element is drawn in.
    pub paint: Option<SourcePaint>,
    /// The stable local id, used when the drawing names nothing.
    pub id: &'a str,
}

/// What a row is called: the element's own label, else its layer, else its id.
/// The id is always shown afterwards, so a name is never a substitute for the
/// stable address a saved selection uses.
pub fn source_name(row: &SourceRow<'_, impl Sized>) -> String {
    let name = row
        .label
        .or(row.group)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(row.id);
    name.to_owned()
}

/// The colour swatch for a row: drawn only when the source has a single
/// colour, so a gradient or an unstroked shape simply shows none.
pub fn paint_swatch(ui: &mut egui::Ui, paint: Option<SourcePaint>) {
    let Some(paint) = paint else {
        return;
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(10., 10.), egui::Sense::hover());
    let [red, green, blue, _] = paint.rgba_unit();
    ui.painter().rect_filled(
        rect,
        egui::CornerRadius::same(2),
        Color32::from_rgb(
            (red * 255.).round() as u8,
            (green * 255.).round() as u8,
            (blue * 255.).round() as u8,
        ),
    );
    response.on_hover_text(format!("drawn in {}", paint.hex()));
}

/// One control per layer and one per colour, so a whole group of geometry can
/// be taken in one action. Returns the values the user asked to select.
pub fn select_all<T: Clone>(ui: &mut egui::Ui, rows: &[SourceRow<'_, T>]) -> Option<Vec<T>> {
    let groups: BTreeSet<&str> = rows
        .iter()
        .filter_map(|row| row.group)
        .map(str::trim)
        .filter(|group| !group.is_empty())
        .collect();
    let mut colours: Vec<SourcePaint> = vec![];
    for row in rows {
        if let Some(paint) = row.paint
            && !colours.contains(&paint)
        {
            colours.push(paint);
        }
    }
    if groups.is_empty() && colours.len() < 2 {
        // Nothing to group by: no point offering one button for everything.
        return None;
    }
    let mut chosen: Option<Vec<T>> = None;
    ui.horizontal_wrapped(|ui| {
        ui.small("Select all in");
        for group in groups {
            let label = format!("Select all in {group}");
            if button(ui, &label, true).clicked() {
                chosen = Some(
                    rows.iter()
                        .filter(|row| row.group.map(str::trim) == Some(group))
                        .map(|row| row.value.clone())
                        .collect(),
                );
            }
        }
        if colours.len() > 1 {
            for paint in colours {
                let label = format!("Select all {}", paint.hex());
                let (rect, response) =
                    ui.allocate_exact_size(egui::vec2(14., 14.), egui::Sense::click());
                let [red, green, blue, _] = paint.rgba_unit();
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::same(2),
                    Color32::from_rgb(
                        (red * 255.).round() as u8,
                        (green * 255.).round() as u8,
                        (blue * 255.).round() as u8,
                    ),
                );
                observe_control(&label, rect);
                if response.on_hover_text(label).clicked() {
                    chosen = Some(
                        rows.iter()
                            .filter(|row| row.paint == Some(paint))
                            .map(|row| row.value.clone())
                            .collect(),
                    );
                }
            }
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row<'a>(
        value: &'a str,
        label: Option<&'a str>,
        group: Option<&'a str>,
    ) -> SourceRow<'a, &'a str> {
        SourceRow {
            value,
            label,
            group,
            paint: None,
            id: value,
        }
    }

    #[test]
    fn a_row_is_named_by_its_label_then_its_layer_then_its_id() {
        assert_eq!(
            source_name(&row("path1::0", Some("Outline"), Some("Cut"))),
            "Outline"
        );
        assert_eq!(source_name(&row("path1::0", None, Some("Cut"))), "Cut");
        assert_eq!(source_name(&row("path1::0", None, None)), "path1::0");
        assert_eq!(source_name(&row("path1::0", Some("  "), None)), "path1::0");
    }

    #[test]
    fn the_colour_is_shown_as_the_hex_the_file_wrote() {
        let paint = SourcePaint {
            red: 0x80,
            green: 0x00,
            blue: 0x2a,
            alpha: 255,
        };
        assert_eq!(paint.hex(), "#80002a");
        assert_eq!(paint.rgba_unit()[0], 128. / 255.);
    }
}
