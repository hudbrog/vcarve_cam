use super::{Result, error, number, user_length};
use crate::geometry::WindingRule;
use roxmltree::Node;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone)]
pub(super) struct Style {
    pub fill: String,
    pub color: String,
    pub stroke: String,
    pub stroke_width: f64,
    pub stroke_opacity: f64,
    pub fill_opacity: f64,
    pub opacity: f64,
    local_opacity: f64,
    pub visible: bool,
    pub suppressed: bool,
    pub rule: WindingRule,
}
impl Default for Style {
    fn default() -> Self {
        Self {
            fill: "black".into(),
            color: "black".into(),
            stroke: "none".into(),
            stroke_width: 1.,
            stroke_opacity: 1.,
            fill_opacity: 1.,
            opacity: 1.,
            local_opacity: 1.,
            visible: true,
            suppressed: false,
            rule: WindingRule::Nonzero,
        }
    }
}
impl Style {
    pub fn resolve(node: Node<'_, '_>, parent: &Self) -> Result<Self> {
        let mut props = BTreeMap::new();
        const PROPS: &[&str] = &[
            "fill",
            "fill-rule",
            "fill-opacity",
            "stroke",
            "stroke-width",
            "stroke-opacity",
            "color",
            "visibility",
            "display",
            "opacity",
            "filter",
            "mask",
            "clip-path",
            "marker",
            "marker-start",
            "marker-mid",
            "marker-end",
            "transform-origin",
        ];
        for &p in PROPS {
            if let Some(v) = node.attribute(p) {
                props.insert(p.to_owned(), v.trim().to_owned());
            }
        }
        if node.attribute("href").is_some()
            || node
                .attribute(("http://www.w3.org/1999/xlink", "href"))
                .is_some()
        {
            return Err(error(
                "SVG_REFERENCE",
                "references are unsupported; unlink clones and embed closed paths",
            ));
        }
        if let Some(style) = node.attribute("style") {
            let mut important = BTreeSet::new();
            for declaration in style.split(';').filter(|s| !s.trim().is_empty()) {
                let Some((key, value)) = declaration.split_once(':') else {
                    return Err(error("SVG_STYLE", "malformed inline style"));
                };
                let key = key.trim();
                let value = value.trim();
                let harmless = key.starts_with("font-")
                    || key.starts_with("-inkscape-")
                    || matches!(
                        key,
                        "stroke-linecap"
                            | "stroke-linejoin"
                            | "stroke-miterlimit"
                            | "stroke-dasharray"
                            | "stroke-dashoffset"
                            | "paint-order"
                            | "shape-rendering"
                            | "vector-effect"
                            | "stop-color"
                            | "stop-opacity"
                    );
                if !PROPS.contains(&key) && !harmless {
                    return Err(error(
                        "SVG_STYLE_UNSUPPORTED",
                        format!("unsupported CSS property {key}"),
                    ));
                }
                let is_important = value.ends_with("!important");
                if important.contains(key) && !is_important {
                    continue;
                }
                if is_important {
                    important.insert(key.to_owned());
                }
                let value = value.strip_suffix("!important").unwrap_or(value).trim();
                props.insert(key.to_owned(), value.to_owned());
            }
        }
        let mut s = parent.clone();
        s.local_opacity = 1.;
        for (key, value) in &props {
            if value == "inherit" {
                if key == "opacity" {
                    s.local_opacity = parent.local_opacity;
                }
                continue;
            }
            match key.as_str() {
                "fill" => s.fill = value.clone(),
                "color" => s.color = value.clone(),
                "stroke" => s.stroke = value.clone(),
                "fill-rule" => {
                    s.rule = match value.as_str() {
                        "evenodd" => WindingRule::Evenodd,
                        "nonzero" => WindingRule::Nonzero,
                        _ => return Err(error("SVG_FILL_RULE", "expected evenodd or nonzero")),
                    }
                }
                "stroke-width" => {
                    s.stroke_width = user_length(value)?;
                    if s.stroke_width < 0. {
                        return Err(error("SVG_STYLE", "stroke width cannot be negative"));
                    }
                }
                "fill-opacity" => s.fill_opacity = opacity(value)?,
                "stroke-opacity" => s.stroke_opacity = opacity(value)?,
                "opacity" => s.local_opacity = opacity(value)?,
                "visibility" => {
                    s.visible = match value.as_str() {
                        "visible" => true,
                        "hidden" | "collapse" => false,
                        _ => return Err(error("SVG_VISIBILITY", "unsupported visibility value")),
                    }
                }
                "display" => match value.as_str() {
                    "none" => s.suppressed = true,
                    "inline" | "block" => {}
                    _ => {
                        return Err(error(
                            "SVG_DISPLAY",
                            "supported display values are none, inline and block",
                        ));
                    }
                },
                "filter" | "mask" | "clip-path" | "marker" | "marker-start" | "marker-mid"
                | "marker-end"
                    if value != "none" =>
                {
                    return Err(error(
                        "SVG_RENDERING_FEATURE",
                        format!("{key} is unsupported; convert its visible result to plain paths"),
                    ));
                }
                "transform-origin" => {
                    return Err(error(
                        "SVG_STYLE_UNSUPPORTED",
                        "CSS transform-origin is unsupported; use the SVG transform attribute",
                    ));
                }
                _ => {}
            }
        }
        s.opacity = parent.opacity * s.local_opacity;
        if s.opacity == 0. {
            s.suppressed = true;
        }
        Ok(s)
    }
    pub fn paint_alpha(&self) -> Result<u8> {
        let paint = if self.fill == "currentColor" {
            &self.color
        } else {
            &self.fill
        };
        paint.parse::<svgtypes::Color>().map(|c|c.alpha).map_err(|_|error("SVG_PAINT","only solid fills/currentColor are supported; gradients and paint servers need conversion"))
    }
}
fn opacity(value: &str) -> Result<f64> {
    let value = number(value)?;
    if !(0.0..=1.0).contains(&value) {
        return Err(error("SVG_OPACITY", "opacity must be between 0 and 1"));
    }
    Ok(value)
}
