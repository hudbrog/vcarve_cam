//! Supported CSS subset for the SVG importer.
//!
//! One element's paint and visibility state comes from three author sources,
//! cascaded in CSS order: presentation attributes first, then the matching
//! rules of the document's `<style>` elements, then the inline `style`
//! attribute, with `!important` beating its own specificity. Rules are matched
//! by element name, `.class` and `#id` selectors. Anything this subset reads
//! but cannot use is reported as a diagnostic instead of being silently
//! dropped, and a property that changes the drawn geometry is refused rather
//! than approximated.
use super::{Result, error, number, user_length};
use crate::geometry::{Diagnostic, WindingRule};
use roxmltree::Node;
use std::collections::BTreeSet;

/// Properties the importer resolves into the element's state.
const APPLIED: &[&str] = &[
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
];

/// Properties that change what geometry is drawn. They are refused rather
/// than approximated: dropping them would carve the wrong shape, and the
/// importer has no renderer to evaluate them with. The CSS `transform`
/// property is refused in the same way, but it is deliberately *not* listed
/// here: the `transform` attribute is the supported SVG one and is read by
/// the transform parser, not by this cascade.
const REJECTED: &[&str] = &[
    "filter",
    "mask",
    "clip-path",
    "marker",
    "marker-start",
    "marker-mid",
    "marker-end",
    "transform-origin",
];

/// Every property this module reads, used to collect presentation attributes.
fn presentation_properties() -> impl Iterator<Item = &'static str> {
    APPLIED.iter().chain(REJECTED).copied()
}

#[derive(Clone, Debug, Default)]
pub(super) struct Stylesheet {
    rules: Vec<Rule>,
}

#[derive(Clone, Debug)]
struct Rule {
    selector: Selector,
    specificity: u64,
    declarations: Vec<Declaration>,
}

#[derive(Clone, Debug)]
struct Declaration {
    name: String,
    value: String,
    important: bool,
    order: usize,
}

/// A simple CSS selector: an optional element name, at most one `#id` and any
/// number of `.class` parts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Selector {
    element: Option<String>,
    id: Option<String>,
    classes: Vec<String>,
}

impl Selector {
    /// CSS specificity (ids, classes, element names) packed into one key so
    /// two selectors compare with `>`.
    fn specificity(&self) -> u64 {
        (self.id.is_some() as u64) * 1_000_000
            + (self.classes.len() as u64) * 1_000
            + (self.element.is_some() as u64)
    }
    fn matches(&self, node: Node<'_, '_>) -> bool {
        if let Some(element) = &self.element
            && node.tag_name().name() != element
        {
            return false;
        }
        if let Some(id) = &self.id
            && node.attribute("id") != Some(id.as_str())
        {
            return false;
        }
        if !self.classes.is_empty() {
            let present: BTreeSet<&str> = node
                .attribute("class")
                .unwrap_or("")
                .split_whitespace()
                .collect();
            if !self
                .classes
                .iter()
                .all(|class| present.contains(class.as_str()))
            {
                return false;
            }
        }
        true
    }
}

impl Stylesheet {
    /// Parse the text of every `<style>` element, in document order. Selector
    /// shapes this subset cannot match come back as warnings, so the import
    /// report lists what was skipped instead of hiding it.
    pub(super) fn parse(texts: &[String]) -> Result<(Self, Vec<Diagnostic>)> {
        let mut sheet = Self::default();
        let mut warnings = vec![];
        let mut order = 0usize;
        for text in texts {
            let css = strip_comments(text);
            for (prelude, body) in split_rules(&css) {
                if let Some(at) = prelude.strip_prefix('@') {
                    let name: String = at
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '-')
                        .collect();
                    if name == "import" {
                        return Err(error(
                            "SVG_STYLESHEET",
                            "external stylesheets are unsupported; inline the rules or use presentation attributes",
                        ));
                    }
                    warnings.push(warning(format!(
                        "CSS at-rule '@{name}' is ignored; only plain element, .class and #id rules are applied"
                    )));
                    continue;
                }
                let mut malformed = vec![];
                let declarations = parse_declarations(&body, &mut order, &mut malformed);
                warnings.extend(malformed.into_iter().map(Diagnostic::warning));
                if declarations.is_empty() {
                    continue;
                }
                for token in prelude.split(',') {
                    match parse_selector(token) {
                        Some(selector) => {
                            let specificity = selector.specificity();
                            sheet.rules.push(Rule {
                                selector,
                                specificity,
                                declarations: declarations.clone(),
                            });
                        }
                        None => warnings.push(
                            error(
                                "SVG_STYLE_SELECTOR",
                                format!(
                                    "CSS selector '{}' is unsupported and its {} declaration(s) were ignored; use element, .class or #id selectors",
                                    token.trim(),
                                    declarations.len()
                                ),
                            )
                            .warning(),
                        ),
                    }
                }
            }
        }
        Ok((sheet, warnings))
    }
}

fn warning(message: String) -> Diagnostic {
    error("SVG_STYLE_IGNORED", message).warning()
}

/// Drop `/* ... */` comments, including ones holding braces.
fn strip_comments(css: &str) -> String {
    let mut out = String::with_capacity(css.len());
    let mut chars = css.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut previous = '\0';
            for c in chars.by_ref() {
                if previous == '*' && c == '/' {
                    break;
                }
                previous = c;
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Split a comment-free stylesheet into `(prelude, body)` pairs, honouring
/// nested braces so an at-rule's body is not mistaken for the next rule.
fn split_rules(css: &str) -> Vec<(String, String)> {
    let mut rules = vec![];
    let mut prelude = String::new();
    let mut body = String::new();
    let mut depth = 0usize;
    let mut in_body = false;
    for c in css.chars() {
        match c {
            '{' if !in_body => {
                in_body = true;
                depth = 1;
            }
            '{' => {
                depth += 1;
                body.push(c);
            }
            '}' if in_body => {
                depth -= 1;
                if depth == 0 {
                    rules.push((prelude.trim().to_owned(), body.trim().to_owned()));
                    prelude.clear();
                    body.clear();
                    in_body = false;
                } else {
                    body.push(c);
                }
            }
            '}' => {}
            _ if in_body => body.push(c),
            _ => prelude.push(c),
        }
    }
    // A trailing at-rule has no body (`@import url(...);`), but it still has
    // to reach the caller: an external stylesheet is a hard failure.
    if !prelude.trim().is_empty() {
        rules.push((prelude.trim().to_owned(), String::new()));
    }
    rules
}

/// Declarations in source order. A chunk with no `name: value` pair is pushed
/// to `malformed`; the caller decides whether that is a warning (a stylesheet
/// rule, where CSS itself drops invalid declarations) or a hard failure (the
/// inline style attribute, which the user typed directly).
fn parse_declarations(
    body: &str,
    order: &mut usize,
    malformed: &mut Vec<Diagnostic>,
) -> Vec<Declaration> {
    let mut declarations = vec![];
    for chunk in body.split(';') {
        let chunk = chunk.trim();
        if chunk.is_empty() {
            continue;
        }
        let parsed = chunk.split_once(':').and_then(|(name, value)| {
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim();
            let important = value.to_ascii_lowercase().ends_with("!important");
            let value = if important {
                value[..value.len() - "!important".len()].trim()
            } else {
                value
            };
            (!name.is_empty() && !value.is_empty()).then(|| (name, value.to_owned(), important))
        });
        let Some((name, value, important)) = parsed else {
            malformed.push(error(
                "SVG_STYLE",
                format!("malformed CSS declaration '{chunk}'"),
            ));
            continue;
        };
        declarations.push(Declaration {
            name,
            value,
            important,
            order: *order,
        });
        *order += 1;
    }
    declarations
}

fn parse_selector(token: &str) -> Option<Selector> {
    let token = token.trim();
    if token.is_empty() {
        return None;
    }
    if token == "*" {
        return Some(Selector::default());
    }
    let identifier = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_');
    let head: String = token.chars().take_while(|c| identifier(*c)).collect();
    let mut rest = &token[head.len()..];
    let mut selector = Selector::default();
    if !head.is_empty() {
        selector.element = Some(head);
    }
    while !rest.is_empty() {
        let marker = rest.chars().next()?;
        if marker != '.' && marker != '#' {
            return None;
        }
        let body: String = rest[1..].chars().take_while(|c| identifier(*c)).collect();
        if body.is_empty() {
            return None;
        }
        rest = &rest[1 + body.len()..];
        if marker == '#' {
            if selector.id.is_some() {
                return None;
            }
            selector.id = Some(body);
        } else {
            selector.classes.push(body);
        }
    }
    Some(selector)
}

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
    /// Resolve one element against its parent's inherited state. The second
    /// return is the list of properties this subset read but could not use.
    pub fn resolve(
        node: Node<'_, '_>,
        parent: &Self,
        sheet: &Stylesheet,
    ) -> Result<(Self, Vec<Diagnostic>)> {
        // (declaration, specificity). The cascade is a stable sort on
        // (importance, specificity, source order), so a stylesheet rule beats
        // a presentation attribute of the same specificity (the attribute is
        // pushed first) and an inline declaration beats every selector.
        let mut candidates: Vec<(Declaration, u64)> = vec![];
        for name in presentation_properties() {
            if let Some(value) = node.attribute(name) {
                candidates.push((
                    Declaration {
                        name: name.to_owned(),
                        value: value.trim().to_owned(),
                        important: false,
                        order: 0,
                    },
                    0,
                ));
            }
        }
        for rule in &sheet.rules {
            if rule.selector.matches(node) {
                for declaration in &rule.declarations {
                    candidates.push((declaration.clone(), rule.specificity));
                }
            }
        }
        if let Some(style) = node.attribute("style") {
            let mut order = 0usize;
            let mut malformed = vec![];
            for declaration in parse_declarations(style, &mut order, &mut malformed) {
                candidates.push((declaration, u64::MAX));
            }
            if let Some(diagnostic) = malformed.into_iter().next() {
                return Err(diagnostic);
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
        candidates.sort_by_key(|(declaration, specificity)| {
            (declaration.important as u8, *specificity, declaration.order)
        });
        let mut ignored: BTreeSet<String> = BTreeSet::new();
        let mut s = parent.clone();
        s.local_opacity = 1.;
        for (declaration, _) in candidates {
            let value = declaration.value.as_str();
            if value == "inherit" {
                if declaration.name == "opacity" {
                    s.local_opacity = parent.local_opacity;
                }
                continue;
            }
            match declaration.name.as_str() {
                "fill" => s.fill = value.to_owned(),
                "color" => s.color = value.to_owned(),
                "stroke" => s.stroke = value.to_owned(),
                "fill-rule" => {
                    s.rule = match value {
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
                    s.visible = match value {
                        "visible" => true,
                        "hidden" | "collapse" => false,
                        _ => return Err(error("SVG_VISIBILITY", "unsupported visibility value")),
                    }
                }
                "display" => match value {
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
                        format!(
                            "{0} is unsupported; convert its visible result to plain paths",
                            declaration.name
                        ),
                    ));
                }
                "filter" | "mask" | "clip-path" | "marker" | "marker-start" | "marker-mid"
                | "marker-end" => {}
                "transform-origin" | "transform" => {
                    return Err(error(
                        "SVG_STYLE_UNSUPPORTED",
                        format!(
                            "CSS {} is unsupported; use the SVG transform attribute",
                            declaration.name
                        ),
                    ));
                }
                _ => {
                    ignored.insert(declaration.name);
                }
            }
        }
        s.opacity = parent.opacity * s.local_opacity;
        if s.opacity == 0. {
            s.suppressed = true;
        }
        let warnings = ignored
            .into_iter()
            .map(|name| {
                warning(format!(
                    "CSS property '{name}' is not part of the supported subset and was ignored; it does not affect carve geometry"
                ))
            })
            .collect();
        Ok((s, warnings))
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
