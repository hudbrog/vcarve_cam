//! Display-only appearance of the simulated stock: how surfaces and walls are
//! coloured, how deep gradients run, and whether the stock is solid or x-ray.
//!
//! Nothing here reaches the job, the plan identity or the emitted program: it
//! is workspace state beside the camera (plan `stock-display-plan.md` §7), and
//! every colour is keyed by an operation or tool **id** so regenerating a plan
//! cannot recolour a part.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Palette slots: stage identities first, then tools.
pub const PALETTE_STAGES: usize = 256;
pub const PALETTE_TOOLS: usize = 256;
pub const PALETTE_ENTRIES: usize = PALETTE_STAGES + PALETTE_TOOLS;

/// How a floor's colour is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorMode {
    /// One material colour plus shading. The shipped default.
    #[default]
    Plain,
    /// The operation that removed the material.
    ByOperation,
    /// The tool that removed it.
    ByTool,
    /// A ramp across the stock thickness, so depth reads as colour.
    ByDepth,
}

impl ColorMode {
    pub const ALL: [Self; 4] = [Self::Plain, Self::ByOperation, Self::ByTool, Self::ByDepth];

    pub fn wire(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::ByOperation => "by_operation",
            Self::ByTool => "by_tool",
            Self::ByDepth => "by_depth",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Plain => "Plain",
            Self::ByOperation => "By operation",
            Self::ByTool => "By tool",
            Self::ByDepth => "By depth",
        }
    }
}

/// How a wall's colour is chosen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WallMode {
    /// Use the colour mode's own result for the wall's cell.
    #[default]
    MatchSurface,
    /// Colour the wall by the operation that removed the material beside it.
    OwnStage,
    /// Colour by the ramp at each fragment's height.
    ByDepth,
}

impl WallMode {
    pub const ALL: [Self; 3] = [Self::MatchSurface, Self::OwnStage, Self::ByDepth];

    pub fn wire(self) -> &'static str {
        match self {
            Self::MatchSurface => "match_surface",
            Self::OwnStage => "own_stage",
            Self::ByDepth => "by_depth",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::MatchSurface => "Match surface",
            Self::OwnStage => "Own operation",
            Self::ByDepth => "Depth gradient",
        }
    }
}

/// Solid stock, or the x-ray the tester asked for (artwork and paths read
/// across the material).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    Opaque,
    XRay,
}

impl Appearance {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Opaque => "opaque",
            Self::XRay => "xray",
        }
    }
}

/// One executed stage of the displayed plan: the index the field stores per
/// cell, with the ids a colour mode keys on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageIdentity {
    pub index: u16,
    pub operation: String,
    pub tool: usize,
    pub tool_id: String,
    pub role: String,
}

/// The display style. Serialized with the workspace view; older saved views
/// fall back to the defaults through `#[serde(default)]`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct StockStyle {
    pub surface: ColorMode,
    pub walls: WallMode,
    /// Ramp stops: stock top (`0`) and stock bottom (`1`) as fractions of the
    /// thickness, so the gradient is comparable across the part.
    pub ramp_top: [f32; 3],
    pub ramp_bottom: [f32; 3],
    /// Explicit per-id colours. Unlisted ids get the deterministic automatic
    /// colour, so a palette is stable across regeneration.
    pub overrides: BTreeMap<String, [f32; 3]>,
    /// Step height that counts as a wall, in millimetres. `None` keeps the
    /// default `max(0.25 × cell, 0.05 mm)`; there is no control for it yet.
    pub wall_threshold_mm: Option<f32>,
    pub appearance: Appearance,
    /// How much of the stock remains in front of what is behind it in x-ray.
    pub xray_opacity: f32,
    /// The plain material colours.
    pub plain: [f32; 3],
    pub plain_wall: [f32; 3],
    pub show_stock: bool,
    pub show_artwork: bool,
    pub show_paths: bool,
}

impl Default for StockStyle {
    fn default() -> Self {
        Self {
            surface: ColorMode::Plain,
            walls: WallMode::MatchSurface,
            ramp_top: [0.62, 0.80, 0.92],
            ramp_bottom: [0.12, 0.28, 0.55],
            overrides: BTreeMap::new(),
            wall_threshold_mm: None,
            appearance: Appearance::Opaque,
            xray_opacity: 0.45,
            plain: [0.60, 0.53, 0.41],
            plain_wall: [0.34, 0.29, 0.22],
            show_stock: true,
            show_artwork: true,
            show_paths: true,
        }
    }
}

impl StockStyle {
    /// Step height that counts as a wall for a display cell of `cell_mm`.
    pub fn wall_threshold(&self, cell_mm: f64) -> f32 {
        self.wall_threshold_mm
            .filter(|value| value.is_finite() && *value > 0.)
            .unwrap_or_else(|| ((cell_mm * 0.25) as f32).max(0.05))
    }

    /// Colour of one floor: the mode's answer, without shading.
    ///
    /// `depth` is the removed fraction of the stock thickness, so `0` is
    /// untouched material; `stage` and `tool` are the indices the field
    /// stores. Untouched material always keeps the plain colour, whatever the
    /// mode, because no cutter created it.
    pub fn floor_color(&self, palette: &[[f32; 4]], stage: u8, tool: u8, depth: f32) -> [f32; 3] {
        if depth <= 0. {
            return self.plain;
        }
        match self.surface {
            ColorMode::Plain => self.plain,
            ColorMode::ByOperation => palette
                .get(stage as usize)
                .map_or(self.plain, |color| rgb(*color)),
            ColorMode::ByTool => palette
                .get(PALETTE_STAGES + tool as usize)
                .map_or(self.plain, |color| rgb(*color)),
            ColorMode::ByDepth => self.ramp(depth),
        }
    }

    /// Colour of a wall fragment: `depth` is the fraction of the stock
    /// thickness the fragment sits below the top, and the identity is the cell
    /// that removed the material beside it.
    pub fn wall_color(&self, palette: &[[f32; 4]], stage: u8, tool: u8, depth: f32) -> [f32; 3] {
        match self.walls {
            WallMode::MatchSurface => {
                let mut color = self.floor_color(palette, stage, tool, 1.);
                if self.surface == ColorMode::Plain {
                    color = self.plain_wall;
                }
                color
            }
            WallMode::OwnStage => palette
                .get(stage as usize)
                .map_or(self.plain_wall, |color| rgb(*color)),
            WallMode::ByDepth => self.ramp(depth.clamp(0., 1.)),
        }
    }

    /// The ramp across the stock thickness.
    pub fn ramp(&self, depth: f32) -> [f32; 3] {
        let t = depth.clamp(0., 1.);
        let mut out = [0.; 3];
        for (channel, value) in out.iter_mut().enumerate() {
            *value =
                self.ramp_top[channel] + (self.ramp_bottom[channel] - self.ramp_top[channel]) * t;
        }
        out
    }

    /// Alpha the stock pass writes: opaque stock hides what is behind it, the
    /// x-ray appearance lets artwork and paths read across it.
    pub fn output_alpha(&self) -> f32 {
        match self.appearance {
            Appearance::Opaque => 1.,
            Appearance::XRay => self.xray_opacity.clamp(0.05, 1.),
        }
    }

    /// The palette buffer the stock pass reads: one entry per stage index
    /// (coloured by that stage's operation) followed by one per tool index
    /// (coloured by the tool's id).
    pub fn palette(&self, stages: &[StageIdentity], tools: &[String]) -> Vec<[f32; 4]> {
        let mut palette = vec![[0.; 4]; PALETTE_ENTRIES];
        for (slot, entry) in palette.iter_mut().enumerate() {
            let id = if slot < PALETTE_STAGES {
                let stage = stages.iter().find(|stage| stage.index as usize == slot);
                match stage {
                    Some(stage) => stage.operation.as_str(),
                    None => continue,
                }
            } else {
                let tool = slot - PALETTE_STAGES;
                match tools.get(tool) {
                    Some(id) if !id.is_empty() => id.as_str(),
                    _ => continue,
                }
            };
            let color = self
                .overrides
                .get(id)
                .copied()
                .unwrap_or_else(|| automatic_color(id));
            *entry = [color[0], color[1], color[2], 1.];
        }
        palette
    }

    /// The uniform the stock pass reads. `light` is the world-space key-light
    /// direction (`xyz`, normalized) with the ambient term in `w`, and
    /// `to_camera` is the world-space direction from the part to the viewer;
    /// both come from the camera, so orbiting never turns a face black.
    pub fn uniform(&self, light: [f32; 4], to_camera: [f32; 4], flags: u32) -> StockUniform {
        StockUniform {
            mode: match self.surface {
                ColorMode::Plain => 0,
                ColorMode::ByOperation => 1,
                ColorMode::ByTool => 2,
                ColorMode::ByDepth => 3,
            },
            wall_mode: match self.walls {
                WallMode::MatchSurface => 0,
                WallMode::OwnStage => 1,
                WallMode::ByDepth => 2,
            },
            appearance: match self.appearance {
                Appearance::Opaque => 0,
                Appearance::XRay => 1,
            },
            flags,
            opacity: self.xray_opacity.clamp(0.05, 1.),
            wall_threshold: self.wall_threshold_mm.unwrap_or(0.),
            ramp_top: 0.,
            ramp_bottom: 1.,
            light,
            to_camera,
            ramp_a: [self.ramp_top[0], self.ramp_top[1], self.ramp_top[2], 0.],
            ramp_b: [
                self.ramp_bottom[0],
                self.ramp_bottom[1],
                self.ramp_bottom[2],
                0.,
            ],
            plain: [self.plain[0], self.plain[1], self.plain[2], 1.],
            plain_wall: [
                self.plain_wall[0],
                self.plain_wall[1],
                self.plain_wall[2],
                1.,
            ],
        }
    }
}

/// The uniform `stock.wgsl` reads. Field order is the shader's; the test below
/// pins the size so a layout drift is caught without a GPU.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct StockUniform {
    pub mode: u32,
    pub wall_mode: u32,
    pub appearance: u32,
    pub flags: u32,
    pub opacity: f32,
    pub wall_threshold: f32,
    pub ramp_top: f32,
    pub ramp_bottom: f32,
    pub light: [f32; 4],
    pub to_camera: [f32; 4],
    pub ramp_a: [f32; 4],
    pub ramp_b: [f32; 4],
    pub plain: [f32; 4],
    pub plain_wall: [f32; 4],
}

/// Flag bits in [`StockUniform::flags`].
pub const FLAG_WALLS: u32 = 1 << 0;
/// Set in a true elevation, where the surface is edge-on: the pass draws the
/// section instead of the cell quads.
pub const FLAG_SECTION: u32 = 1 << 1;

/// The key light the stock pass shades with: a fixed world direction from the
/// upper left with the ambient term in `w`. Fixed rather than camera-relative
/// so a floor and the walls around it keep their contrast as the view orbits;
/// the ambient term keeps the far side readable.
pub fn key_light() -> [f32; 4] {
    [0.42, -0.36, 0.83, 0.45]
}

fn rgb(color: [f32; 4]) -> [f32; 3] {
    [color[0], color[1], color[2]]
}

/// Deterministic colour for an id: same id, same colour, on every machine and
/// after every regeneration.
pub fn automatic_color(id: &str) -> [f32; 3] {
    let mut hash = 2166136261u32;
    for byte in id.as_bytes() {
        hash = (hash ^ *byte as u32).wrapping_mul(16777619);
    }
    // Golden-ratio hue rotation with fixed saturation and value keeps adjacent
    // operations distinguishable and the palette readable.
    let hue = (hash as f32 / u32::MAX as f32 + 0.618_034).fract();
    hsv(hue, 0.55, 0.78)
}

fn hsv(hue: f32, saturation: f32, value: f32) -> [f32; 3] {
    let h = (hue.fract().abs() * 6.).min(5.999);
    let sector = h.floor();
    let fraction = h - sector;
    let p = value * (1. - saturation);
    let q = value * (1. - saturation * fraction);
    let t = value * (1. - saturation * (1. - fraction));
    match sector as u32 {
        0 => [value, t, p],
        1 => [q, value, p],
        2 => [p, value, t],
        3 => [p, q, value],
        4 => [t, p, value],
        _ => [value, p, q],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    fn stages() -> Vec<StageIdentity> {
        vec![
            StageIdentity {
                index: 0,
                operation: "op-a".into(),
                tool: 0,
                tool_id: "tool-a".into(),
                role: "Endmill".into(),
            },
            StageIdentity {
                index: 1,
                operation: "op-a".into(),
                tool: 1,
                tool_id: "tool-b".into(),
                role: "V-bit".into(),
            },
            StageIdentity {
                index: 2,
                operation: "op-b".into(),
                tool: 0,
                tool_id: "tool-a".into(),
                role: "Face".into(),
            },
        ]
    }

    /// The uniform is written straight into a wgpu buffer, so its layout is
    /// part of the shader contract.
    #[test]
    fn the_uniform_layout_matches_the_shader() {
        assert_eq!(size_of::<StockUniform>(), 128);
        assert_eq!(
            std::mem::offset_of!(StockUniform, light),
            32,
            "eight scalars before the first vec4"
        );
        assert_eq!(std::mem::offset_of!(StockUniform, to_camera), 48);
        assert_eq!(std::mem::offset_of!(StockUniform, plain_wall), 112);
    }

    #[test]
    fn palettes_are_keyed_by_id_and_stable() {
        let style = StockStyle::default();
        let stages = stages();
        let tools = vec!["tool-a".to_string(), "tool-b".to_string()];
        let palette = style.palette(&stages, &tools);
        assert_eq!(palette.len(), PALETTE_ENTRIES);
        // Two stages of one operation share a colour: that is what "by
        // operation" means.
        assert_eq!(palette[0], palette[1]);
        assert_ne!(palette[0], palette[2]);
        assert_ne!(palette[PALETTE_STAGES], palette[PALETTE_STAGES + 1]);
        // Unlisted indices stay empty rather than borrowing a neighbour.
        assert_eq!(palette[9], [0.; 4]);
        // Deterministic across calls and calls on a fresh style.
        assert_eq!(palette, StockStyle::default().palette(&stages, &tools));
        assert_eq!(automatic_color("op-a"), automatic_color("op-a"));
        assert_ne!(automatic_color("op-a"), automatic_color("op-b"));
    }

    #[test]
    fn overrides_win_over_the_automatic_colour() {
        let mut style = StockStyle::default();
        style.overrides.insert("op-b".into(), [1., 0., 0.]);
        let palette = style.palette(&stages(), &[]);
        assert_eq!(palette[2], [1., 0., 0., 1.]);
    }

    #[test]
    fn untouched_material_keeps_the_plain_colour_in_every_mode() {
        let style = StockStyle::default();
        let palette = style.palette(&stages(), &["tool-a".into()]);
        for mode in ColorMode::ALL {
            let mut style = style.clone();
            style.surface = mode;
            assert_eq!(
                style.floor_color(&palette, 0, 0, 0.),
                style.plain,
                "{mode:?} must not colour material no cutter touched"
            );
        }
    }

    #[test]
    fn the_depth_ramp_runs_across_the_stock_thickness() {
        let style = StockStyle::default();
        let close =
            |a: [f32; 3], b: [f32; 3]| (0..3).all(|channel| (a[channel] - b[channel]).abs() < 1e-6);
        assert!(close(style.ramp(0.), style.ramp_top));
        assert!(close(style.ramp(1.), style.ramp_bottom));
        assert!(close(style.ramp(-3.), style.ramp_top));
        assert!(close(style.ramp(7.), style.ramp_bottom));
        let middle = style.ramp(0.5);
        for (channel, value) in middle.iter().enumerate() {
            let (a, b) = (style.ramp_top[channel], style.ramp_bottom[channel]);
            assert!((value - (a + b) / 2.).abs() < 1e-6);
        }
        // A wall's gradient samples the same ramp at the fragment's height, so
        // a shallow wall shows only the slice between its levels.
        let mut style = style;
        style.walls = WallMode::ByDepth;
        assert_eq!(style.wall_color(&[], 0, 0, 0.1), style.ramp(0.1));
        assert_ne!(
            style.wall_color(&[], 0, 0, 0.1),
            style.wall_color(&[], 0, 0, 0.9)
        );
    }

    #[test]
    fn the_wall_threshold_defaults_to_the_documented_rule() {
        let style = StockStyle::default();
        assert!((style.wall_threshold(0.4) - 0.1).abs() < 1e-6);
        assert!((style.wall_threshold(0.1) - 0.05).abs() < 1e-6);
        let style = StockStyle {
            wall_threshold_mm: Some(0.3),
            ..StockStyle::default()
        };
        assert!((style.wall_threshold(0.4) - 0.3).abs() < 1e-6);
    }

    /// The x-ray appearance is the tester's answer to the translucency
    /// question: the stock is drawn so artwork and paths read across it, which
    /// is an alpha plus a depth-write decision in the pass.
    #[test]
    fn the_stock_alpha_follows_the_appearance() {
        let mut style = StockStyle::default();
        assert_eq!(style.output_alpha(), 1., "opaque stock writes no alpha");
        style.appearance = Appearance::XRay;
        style.xray_opacity = 0.35;
        assert!((style.output_alpha() - 0.35).abs() < 1e-6);
        style.xray_opacity = 0.;
        assert!(style.output_alpha() >= 0.05, "never fully invisible");
        assert_eq!(style.uniform(key_light(), [0.; 4], 0).appearance, 1);
    }
}
