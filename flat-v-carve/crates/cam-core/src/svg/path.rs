use super::{Result, error};
use crate::geometry::{Point, Segment};
use svgtypes::{PathParser, PathSegment as S};

pub(super) const MAX_VERTICES: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub(super) struct Matrix(pub [f64; 6]);
impl Matrix {
    pub const ID: Self = Self([1., 0., 0., 1., 0., 0.]);
    pub fn parse(s: &str) -> Result<Self> {
        let t: svgtypes::Transform = s
            .parse()
            .map_err(|e| error("SVG_TRANSFORM", format!("{e}")))?;
        let m = Self([t.a, t.b, t.c, t.d, t.e, t.f]);
        m.validate()?;
        Ok(m)
    }
    pub fn validate(self) -> Result<()> {
        let [a, b, c, d, _, _] = self.0;
        if self.0.iter().any(|v| !v.is_finite() || v.abs() > 1e12) || a * d - b * c == 0.0 {
            return Err(error(
                "SVG_TRANSFORM",
                "nonfinite, singular, or excessive transform",
            ));
        }
        Ok(())
    }
    pub fn apply(self, p: Point) -> Point {
        let [a, b, c, d, e, f] = self.0;
        Point::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }
    pub fn then(self, rhs: Self) -> Self {
        // Matrix product self * rhs: rhs acts first.
        let [a, b, c, d, e, f] = self.0;
        let [g, h, i, j, k, l] = rhs.0;
        Self([
            a * g + c * h,
            b * g + d * h,
            a * i + c * j,
            b * i + d * j,
            a * k + c * l + e,
            b * k + d * l + f,
        ])
    }
}

pub(super) struct Flattener {
    pub matrix: Matrix,
    pub tolerance: f64,
    pub points: Vec<Point>,
    pub rings: Vec<Vec<Point>>,
    pub total: usize,
}
impl Flattener {
    pub fn new(matrix: Matrix, tolerance: f64) -> Self {
        Self {
            matrix,
            tolerance,
            points: vec![],
            rings: vec![],
            total: 0,
        }
    }
    pub fn push(&mut self, p: Point) -> Result<()> {
        if !p.finite() || p.x.abs().max(p.y.abs()) > 1e9 {
            return Err(error(
                "SVG_COORDINATE_RANGE",
                "coordinates must be finite and within 1e9 mm",
            ));
        }
        if self.points.last() == Some(&p) {
            return Ok(());
        }
        self.total += 1;
        if self.total > MAX_VERTICES {
            return Err(error(
                "SVG_CURVE_LIMIT",
                "flattening exceeds 4096 vertices; increase tolerance or simplify artwork",
            ));
        }
        self.points.push(p);
        Ok(())
    }
    fn finish(&mut self, explicit: bool) -> Result<()> {
        if self.points.is_empty() {
            return Ok(());
        }
        let closed = self.points.first() == self.points.last();
        if !explicit && !closed {
            return Err(error(
                "SVG_OPEN_PATH",
                "close each subpath in Inkscape before import",
            ));
        }
        if closed {
            self.points.pop();
        }
        if self.points.len() < 3 {
            return Err(error(
                "SVG_DEGENERATE_PATH",
                "closed subpath needs three distinct vertices",
            ));
        }
        self.rings.push(std::mem::take(&mut self.points));
        Ok(())
    }
    fn cubic(&mut self, p: [Point; 4], level: usize) -> Result<()> {
        if p.iter().any(|p| !p.finite()) {
            return Err(error("SVG_CURVE_RANGE", "nonfinite curve controls"));
        }
        let chord = Segment {
            start: p[0],
            end: p[3],
        };
        // The Bezier lies in its control hull; distances to this segment bound it.
        if chord.distance(p[1]).max(chord.distance(p[2])) <= self.tolerance {
            return self.push(p[3]);
        }
        if level >= 32 {
            return Err(error(
                "SVG_CURVE_LIMIT",
                "Bezier subdivision cannot meet the requested tolerance",
            ));
        }
        let a = p[0].lerp(p[1], 0.5);
        let b = p[1].lerp(p[2], 0.5);
        let c = p[2].lerp(p[3], 0.5);
        let d = a.lerp(b, 0.5);
        let e = b.lerp(c, 0.5);
        let f = d.lerp(e, 0.5);
        self.cubic([p[0], a, d, f], level + 1)?;
        self.cubic([f, e, c, p[3]], level + 1)
    }
    pub fn ellipse(
        &mut self,
        center: Point,
        axes: [Point; 2],
        start: f64,
        sweep: f64,
    ) -> Result<()> {
        let o = self.matrix.apply(center);
        let u = self
            .matrix
            .apply(Point::new(center.x + axes[0].x, center.y + axes[0].y));
        let v = self
            .matrix
            .apply(Point::new(center.x + axes[1].x, center.y + axes[1].y));
        let u = Point::new(u.x - o.x, u.y - o.y);
        let v = Point::new(v.x - o.x, v.y - o.y);
        // |r''(t)| <= |u|+|v|; interpolation error <= max|r''| * dt^2/8.
        let curvature = u.x.hypot(u.y) + v.x.hypot(v.y);
        let n = (sweep.abs() * (curvature / (8.0 * self.tolerance)).sqrt())
            .ceil()
            .max(1.0);
        if !n.is_finite() || n > MAX_VERTICES as f64 {
            return Err(error(
                "SVG_CURVE_LIMIT",
                "elliptical arc exceeds the curve subdivision budget",
            ));
        }
        for i in 1..=n as usize {
            let t = start + sweep * i as f64 / n;
            self.push(Point::new(
                o.x + u.x * t.cos() + v.x * t.sin(),
                o.y + u.y * t.cos() + v.y * t.sin(),
            ))?;
        }
        Ok(())
    }
    fn arc(
        &mut self,
        from: Point,
        to: Point,
        radii: Point,
        rotation: f64,
        large: bool,
        sweep: bool,
    ) -> Result<()> {
        if from == to {
            return Ok(());
        }
        let (mut rx, mut ry) = (radii.x.abs(), radii.y.abs());
        if rx == 0.0 || ry == 0.0 {
            return self.push(self.matrix.apply(to));
        }
        if [rx, ry, rotation, from.x, from.y, to.x, to.y]
            .iter()
            .any(|v| !v.is_finite() || v.abs() > 1e12)
        {
            return Err(error(
                "SVG_CURVE_RANGE",
                "arc parameters exceed the numeric range",
            ));
        }
        // SVG endpoint-to-center conversion, W3C SVG implementation notes B.2.
        let (sin, cos) = rotation.to_radians().sin_cos();
        let dx = (from.x - to.x) / 2.;
        let dy = (from.y - to.y) / 2.;
        let x = cos * dx + sin * dy;
        let y = -sin * dx + cos * dy;
        let correction = (x / rx).hypot(y / ry);
        if correction > 1. {
            rx *= correction;
            ry *= correction;
        }
        let denom = (x / rx).powi(2) + (y / ry).powi(2);
        if !denom.is_finite() || denom <= 0. {
            return Err(error(
                "SVG_CURVE_RANGE",
                "arc center is numerically unresolved",
            ));
        }
        let sign = if large == sweep { -1. } else { 1. };
        let factor = sign * ((1. - denom).max(0.) / denom).sqrt();
        let cx = factor * rx * y / ry;
        let cy = -factor * ry * x / rx;
        let center = Point::new(
            cos * cx - sin * cy + (from.x + to.x) / 2.,
            sin * cx + cos * cy + (from.y + to.y) / 2.,
        );
        let start = ((y - cy) / ry).atan2((x - cx) / rx);
        let end = ((-y - cy) / ry).atan2((-x - cx) / rx);
        let mut delta = (end - start) % std::f64::consts::TAU;
        if sweep && delta < 0. {
            delta += std::f64::consts::TAU;
        }
        if !sweep && delta > 0. {
            delta -= std::f64::consts::TAU;
        }
        self.ellipse(
            center,
            [
                Point::new(rx * cos, rx * sin),
                Point::new(-ry * sin, ry * cos),
            ],
            start,
            delta,
        )?;
        // Keep the exact requested endpoint, avoiding trigonometric closure slivers.
        if let Some(last) = self.points.last_mut() {
            *last = self.matrix.apply(to);
        }
        Ok(())
    }
    pub fn path(mut self, data: &str) -> Result<Vec<Vec<Point>>> {
        let mut current = Point::new(0., 0.);
        let mut start = current;
        let mut cubic_control = None;
        let mut quadratic_control = None;
        let mut command_count = 0;
        for command in PathParser::from(data) {
            command_count += 1;
            if command_count > MAX_VERTICES {
                return Err(error("SVG_PATH_LIMIT", "too many path commands"));
            }
            let command = command.map_err(|e| error("SVG_PATH_SYNTAX", format!("{e}")))?;
            let point = |abs: bool, x: f64, y: f64| {
                if abs {
                    Point::new(x, y)
                } else {
                    Point::new(current.x + x, current.y + y)
                }
            };
            let mut next_cubic = None;
            let mut next_quad = None;
            match command {
                S::MoveTo { abs, x, y } => {
                    self.finish(false)?;
                    current = point(abs, x, y);
                    start = current;
                    self.push(self.matrix.apply(current))?;
                }
                S::ClosePath { .. } => {
                    self.finish(true)?;
                    current = start;
                }
                other => {
                    if self.points.is_empty() {
                        self.push(self.matrix.apply(current))?;
                    }
                    match other {
                        S::LineTo { abs, x, y } => {
                            current = point(abs, x, y);
                            self.push(self.matrix.apply(current))?;
                        }
                        S::HorizontalLineTo { abs, x } => {
                            current = Point::new(if abs { x } else { current.x + x }, current.y);
                            self.push(self.matrix.apply(current))?;
                        }
                        S::VerticalLineTo { abs, y } => {
                            current = Point::new(current.x, if abs { y } else { current.y + y });
                            self.push(self.matrix.apply(current))?;
                        }
                        S::CurveTo {
                            abs,
                            x1,
                            y1,
                            x2,
                            y2,
                            x,
                            y,
                        } => {
                            let p1 = point(abs, x1, y1);
                            let p2 = point(abs, x2, y2);
                            let p3 = point(abs, x, y);
                            self.cubic([current, p1, p2, p3].map(|p| self.matrix.apply(p)), 0)?;
                            next_cubic = Some(p2);
                            current = p3;
                        }
                        S::SmoothCurveTo { abs, x2, y2, x, y } => {
                            let p1 = cubic_control.map_or(current, |p: Point| {
                                Point::new(2. * current.x - p.x, 2. * current.y - p.y)
                            });
                            let p2 = point(abs, x2, y2);
                            let p3 = point(abs, x, y);
                            self.cubic([current, p1, p2, p3].map(|p| self.matrix.apply(p)), 0)?;
                            next_cubic = Some(p2);
                            current = p3;
                        }
                        S::Quadratic { abs, x1, y1, x, y } => {
                            let p1 = point(abs, x1, y1);
                            let p2 = point(abs, x, y);
                            self.cubic(
                                [current, current.lerp(p1, 2. / 3.), p2.lerp(p1, 2. / 3.), p2]
                                    .map(|p| self.matrix.apply(p)),
                                0,
                            )?;
                            next_quad = Some(p1);
                            current = p2;
                        }
                        S::SmoothQuadratic { abs, x, y } => {
                            let p1 = quadratic_control.map_or(current, |p: Point| {
                                Point::new(2. * current.x - p.x, 2. * current.y - p.y)
                            });
                            let p2 = point(abs, x, y);
                            self.cubic(
                                [current, current.lerp(p1, 2. / 3.), p2.lerp(p1, 2. / 3.), p2]
                                    .map(|p| self.matrix.apply(p)),
                                0,
                            )?;
                            next_quad = Some(p1);
                            current = p2;
                        }
                        S::EllipticalArc {
                            abs,
                            rx,
                            ry,
                            x_axis_rotation,
                            large_arc,
                            sweep,
                            x,
                            y,
                        } => {
                            let end = point(abs, x, y);
                            self.arc(
                                current,
                                end,
                                Point::new(rx, ry),
                                x_axis_rotation,
                                large_arc,
                                sweep,
                            )?;
                            current = end;
                        }
                        _ => unreachable!(),
                    }
                }
            }
            cubic_control = next_cubic;
            quadratic_control = next_quad;
        }
        self.finish(false)?;
        if self.rings.is_empty() {
            return Err(error(
                "SVG_EMPTY_PATH",
                "path has no closed filled contours",
            ));
        }
        Ok(self.rings)
    }
}
