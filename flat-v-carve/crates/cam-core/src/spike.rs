//! Reproducible M0 fixtures and measured capability evidence.
use crate::geometry::{
    BooleanOp, Diagnostic, Grid, Linearization, Point, Region, Result, Segment, VoronoiDiagram,
};
use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub id: String,
    pub description: String,
    pub tolerance_mm: f64,
    pub rings: Vec<Vec<Point>>,
    pub operation: Operation,
    pub expected: Expected,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Normalize,
    Boolean {
        op: BooleanOp,
        other: Vec<Vec<Point>>,
    },
    Erode {
        radius_mm: f64,
    },
    Voronoi,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub area_mm2: Option<f64>,
    pub area_tolerance_mm2: Option<f64>,
    pub components: Option<usize>,
    pub holes: Option<usize>,
    pub min_curved_edges: Option<usize>,
    pub min_straight_edges: Option<usize>,
    pub error_code: Option<String>,
    pub diagnostic_code: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Measurement {
    pub name: String,
    pub measured: f64,
    pub expected: f64,
    pub tolerance: f64,
    pub passed: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct EdgePreview {
    pub edge: usize,
    pub linearization: Linearization,
}

#[derive(Clone, Debug, Serialize)]
pub struct FixtureResult {
    pub fixture: Fixture,
    pub passed: bool,
    pub grid: Option<Grid>,
    pub input_region: Option<Region>,
    pub output_region: Option<Region>,
    pub voronoi: Option<VoronoiDiagram>,
    pub edge_previews: Vec<EdgePreview>,
    pub measurements: Vec<Measurement>,
    pub diagnostics: Vec<Diagnostic>,
}

impl FixtureResult {
    fn measure(&mut self, name: &str, measured: f64, expected: f64, tolerance: f64) {
        self.measurements.push(Measurement {
            name: name.into(),
            measured,
            expected,
            tolerance,
            passed: measured.is_finite()
                && expected.is_finite()
                && tolerance.is_finite()
                && tolerance >= 0.0
                && (measured - expected).abs() <= tolerance,
        });
    }
}

/// Runs in memory. Even failures return the complete input and any available preview.
pub fn run_fixture(fixture: Fixture) -> FixtureResult {
    let mut result = FixtureResult {
        fixture,
        passed: false,
        grid: None,
        input_region: None,
        output_region: None,
        voronoi: None,
        edge_previews: vec![],
        measurements: vec![],
        diagnostics: vec![],
    };
    let outcome = evaluate(&mut result);
    match outcome {
        Err(error) => {
            result.passed = result.fixture.expected.error_code.as_deref() == Some(&error.code);
            result.diagnostics.push(error);
        }
        Ok(()) => {
            if result.fixture.expected.error_code.is_some() {
                result.diagnostics.push(Diagnostic::new(
                    "EXPECTED_ERROR_MISSING",
                    "fixture expected a rejected input",
                ));
            } else {
                result.passed = result.measurements.iter().all(|m| m.passed);
            }
        }
    }
    if let Some(code) = &result.fixture.expected.diagnostic_code
        && !result.diagnostics.iter().any(|d| &d.code == code)
    {
        result.passed = false;
        result.diagnostics.push(Diagnostic::new(
            "EXPECTED_DIAGNOSTIC_MISSING",
            format!("expected {code}"),
        ));
    }
    result
}

fn evaluate(result: &mut FixtureResult) -> Result<()> {
    let fixture = &result.fixture;
    let other = match &fixture.operation {
        Operation::Boolean { other, .. } => other.as_slice(),
        _ => &[],
    };
    let extent = fixture
        .rings
        .iter()
        .chain(other)
        .flatten()
        .map(|p| p.x.abs().max(p.y.abs()))
        .fold(0.0, f64::max);
    let grid = Grid::new(fixture.tolerance_mm, extent)?;
    result.grid = Some(grid);
    let input = Region::from_rings(grid, &fixture.rings)?;
    result
        .diagnostics
        .extend(input.diagnostics().iter().cloned());
    result.input_region = Some(input.clone());
    let output = match &fixture.operation {
        Operation::Normalize => input.clone(),
        Operation::Boolean { op, other } => {
            input.boolean(*op, &Region::from_rings(grid, other)?)?
        }
        Operation::Erode { radius_mm } => input.erode(*radius_mm)?,
        Operation::Voronoi => {
            let diagram = VoronoiDiagram::build(&input)?;
            result.voronoi = Some(diagram.clone());
            measure_diagram(result, &diagram)?;
            input.clone()
        }
    };
    // Keep output before checking expectations so numerical failures are inspectable.
    result.output_region = Some(output.clone());
    result.diagnostics.extend(
        output
            .diagnostics()
            .iter()
            .filter(|d| !input.diagnostics().contains(d))
            .cloned(),
    );
    let expected = result.fixture.expected.clone();
    if let Some(area) = expected.area_mm2 {
        result.measure(
            "area_mm2",
            output.area_mm2(),
            area,
            expected.area_tolerance_mm2.unwrap_or(1e-9),
        );
    }
    if let Some(n) = expected.components {
        result.measure("components", output.component_count() as f64, n as f64, 0.0);
    }
    if let Some(n) = expected.holes {
        result.measure("holes", output.hole_count() as f64, n as f64, 0.0);
    }
    if let Operation::Erode { radius_mm } = result.fixture.operation {
        let mut max_residual: f64 = 0.0;
        for segment in output.segments() {
            for i in 0..=16 {
                let p = segment.start.lerp(segment.end, i as f64 / 16.0);
                max_residual = max_residual.max((input.boundary_distance_mm(p) - radius_mm).abs());
            }
        }
        if !output.rings().is_empty() {
            result.measure(
                "offset_sampled_clearance_residual_mm",
                max_residual,
                0.0,
                grid.tolerance_mm(),
            );
        }
    }
    Ok(())
}

fn measure_diagram(result: &mut FixtureResult, diagram: &VoronoiDiagram) -> Result<()> {
    let tolerance = result.fixture.tolerance_mm;
    let mut equal_error: f64 = 0.0;
    let mut nearest_error: f64 = 0.0;
    let mut chord_error: f64 = 0.0;
    let mut max_bound: f64 = 0.0;
    let mut curved: usize = 0;
    let mut straight: usize = 0;
    for (index, edge) in diagram.edges.iter().enumerate() {
        let Some(curve) = &edge.curve else { continue };
        if edge.curved {
            curved += 1
        } else {
            straight += 1
        };
        let preview = curve.linearize(tolerance / 4.0, 65536)?;
        max_bound = max_bound.max(preview.total_bound_mm);
        for i in 0..=64 {
            let p = curve.evaluate(i as f64 / 64.0)?;
            let a = edge.sites[0].distance_mm(p, &diagram.source_segments);
            let b = edge.sites[1].distance_mm(p, &diagram.source_segments);
            let nearest = diagram
                .source_segments
                .iter()
                .map(|s| s.distance(p))
                .fold(f64::INFINITY, f64::min);
            equal_error = equal_error.max((a - b).abs());
            nearest_error = nearest_error.max((a - nearest).abs());
        }
        let count = preview.points.len() - 1;
        for (i, points) in preview.points.windows(2).enumerate() {
            let segment = Segment {
                start: points[0],
                end: points[1],
            };
            for j in 1..8 {
                let p = curve.evaluate((i as f64 + j as f64 / 8.0) / count as f64)?;
                chord_error = chord_error.max(segment.distance(p));
                if segment.distance(p) > preview.total_bound_mm + 1e-12 {
                    return Err(Diagnostic::new(
                        "CURVE_BOUND_VIOLATION",
                        format!("edge {index} exceeds its declared chord bound"),
                    ));
                }
            }
        }
        result.edge_previews.push(EdgePreview {
            edge: index,
            linearization: preview,
        });
    }
    result.measure(
        "voronoi_sampled_equidistance_residual_mm",
        equal_error,
        0.0,
        tolerance / 16.0,
    );
    result.measure(
        "voronoi_sampled_nearest_site_residual_mm",
        nearest_error,
        0.0,
        tolerance / 16.0,
    );
    result.measure(
        "curve_sampled_chord_error_mm",
        chord_error,
        0.0,
        tolerance / 4.0,
    );
    result.measure(
        "curve_max_declared_bound_mm",
        max_bound,
        0.0,
        tolerance / 4.0,
    );
    if let Some(n) = result.fixture.expected.min_curved_edges {
        result.measure(
            "finite_curved_edges_at_least",
            curved as f64,
            n as f64,
            curved.saturating_sub(n) as f64,
        );
    }
    if let Some(n) = result.fixture.expected.min_straight_edges {
        result.measure(
            "finite_straight_edges_at_least",
            straight as f64,
            n as f64,
            straight.saturating_sub(n) as f64,
        );
    }
    Ok(())
}
