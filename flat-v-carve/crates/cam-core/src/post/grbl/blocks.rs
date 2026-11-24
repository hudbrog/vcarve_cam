//! The writer and reader operate exclusively on machine actions in work coordinates.
use super::{GrblProfile, Run};
use crate::{
    geometry::{Diagnostic, Point, Result},
    motion::Position,
    post::{Coolant, SpindleDirection, error, sequence::PreparedSpindle},
    toolpath::Interpolation,
};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action<'a> {
    Modal(&'a str),
    Z(f64),
    Xy(f64, f64),
    Rapid(Position),
    Line(Position, f64),
    Arc {
        end: Position,
        center: (f64, f64),
        clockwise: bool,
        feed: f64,
    },
    Dwell(f64),
    Spindle(SpindleDirection, f64),
}

fn actions<'a>(
    runs: &[Run<'_>],
    profile: &'a GrblProfile,
    zero: Position,
    mut visit: impl FnMut(Action<'a>) -> Result<()>,
) -> Result<()> {
    let work = |p: Position| Position {
        x: p.x - zero.x,
        y: p.y - zero.y,
        z: p.z - zero.z,
    };
    let clearance = profile.clearance_z_mm - zero.z;
    for mode in ["M5", "M9", "G21 G17 G90 G94", &profile.work_offset] {
        visit(Action::Modal(mode))?;
    }
    let mut previous = None;
    let mut spindle = PreparedSpindle::Off;
    let mut coolant = Coolant::Off;
    for run in runs {
        let mut motions = run.motions.iter().peekable();
        let first = motions
            .peek()
            .ok_or_else(|| error("POST_EMPTY", "empty Grbl stage"))?;
        let entry = work(first.start);
        if spindle != run.process.spindle && spindle != PreparedSpindle::Off {
            visit(Action::Modal("M5"))?;
        }
        if coolant != run.process.coolant && coolant != Coolant::Off {
            visit(Action::Modal("M9"))?;
        }
        if previous != Some(entry) {
            if entry.z < clearance - 1e-9 {
                return Err(error(
                    "GRBL_ENTRY",
                    "disconnected stage entry is below job clearance",
                ));
            }
            visit(Action::Z(entry.z))?;
            visit(Action::Xy(entry.x, entry.y))?;
        }
        if spindle != run.process.spindle
            && let PreparedSpindle::On { rpm, direction } = run.process.spindle
        {
            visit(Action::Spindle(direction, rpm))?;
            visit(Action::Dwell(profile.spindle_spinup_seconds))?;
        }
        if coolant != run.process.coolant && run.process.coolant == Coolant::Flood {
            visit(Action::Modal("M8"))?;
        }
        spindle = run.process.spindle;
        coolant = run.process.coolant;
        let mut at = entry;
        for motion in motions {
            let start = work(motion.start);
            if start != at {
                return Err(error(
                    "POST_SEQUENCE_MISMATCH",
                    "motion start is discontinuous within a stage",
                ));
            }
            let end = work(motion.end);
            let feed = || {
                motion
                    .feed_mm_min
                    .filter(|f| f.is_finite() && *f > 0.)
                    .ok_or_else(|| error("POST_FEED", "missing or invalid feed"))
            };
            visit(match motion.interpolation {
                Interpolation::Rapid => Action::Rapid(end),
                Interpolation::LinearFeed => Action::Line(end, feed()?),
                Interpolation::Dwell { seconds } => Action::Dwell(seconds),
                Interpolation::ArcFeed(arc) => Action::Arc {
                    end,
                    center: (arc.center.x - zero.x, arc.center.y - zero.y),
                    clockwise: arc.clockwise,
                    feed: feed()?,
                },
            })?;
            at = end;
        }
        previous = Some(at);
    }
    visit(Action::Z(clearance))?;
    for mode in ["M5", "M9", "M2"] {
        visit(Action::Modal(mode))?;
    }
    Ok(())
}

fn decimal(v: f64, places: usize) -> String {
    let s = format!("{v:.places$}");
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        &s
    };
    if s == "-0" { "0".into() } else { s.into() }
}

fn encode(action: Action<'_>, places: usize, start: Option<Position>) -> String {
    let n = |v| decimal(v, places);
    let xyz = |p: Position| format!("X{} Y{} Z{}", n(p.x), n(p.y), n(p.z));
    match action {
        Action::Modal(mode) => mode.into(),
        Action::Z(z) => format!("G0 Z{}", n(z)),
        Action::Xy(x, y) => format!("G0 X{} Y{}", n(x), n(y)),
        Action::Rapid(p) => format!("G0 {}", xyz(p)),
        Action::Line(p, f) => format!("G1 {} F{f}", xyz(p)),
        Action::Arc {
            end,
            center,
            clockwise,
            feed,
        } => {
            let start = start.expect("XY established before arc");
            let ij = |c, s| {
                n(super::super::sequence::rounded(c, places)
                    - super::super::sequence::rounded(s, places))
            };
            format!(
                "{} {} I{} J{} F{feed}",
                if clockwise { "G2" } else { "G3" },
                xyz(end),
                ij(center.0, start.x),
                ij(center.1, start.y)
            )
        }
        Action::Dwell(sec) => format!("G4 P{sec}"),
        Action::Spindle(d, rpm) => format!(
            "{} S{rpm}",
            if d == SpindleDirection::Clockwise {
                "M3"
            } else {
                "M4"
            }
        ),
    }
}

#[derive(Default)]
struct Cursor {
    xy: Option<(f64, f64)>,
    z: Option<f64>,
}
impl Cursor {
    fn at(&self) -> Option<Position> {
        let (x, y) = self.xy?;
        Some(Position { x, y, z: self.z? })
    }
    fn apply(&mut self, action: Action<'_>) {
        match action {
            Action::Z(z) => self.z = Some(z),
            Action::Xy(x, y) => self.xy = Some((x, y)),
            Action::Rapid(p) | Action::Line(p, _) | Action::Arc { end: p, .. } => {
                self.xy = Some((p.x, p.y));
                self.z = Some(p.z);
            }
            _ => {}
        }
    }
}

pub(super) fn program(
    runs: &[Run<'_>],
    profile: &GrblProfile,
    zero: Position,
    places: usize,
    tolerance: f64,
) -> Result<String> {
    let mut result = String::new();
    let mut at = Cursor::default();
    actions(runs, profile, zero, |a| {
        result.push_str(&encode(a, places, at.at()));
        result.push('\n');
        at.apply(a);
        Ok(())
    })?;
    verify(&result, runs, profile, zero, tolerance)?;
    Ok(result)
}

fn subset() -> Diagnostic {
    error("GRBL_SUBSET", "unexpected or malformed Grbl program block")
}
fn precision() -> Diagnostic {
    error(
        "POST_PRECISION",
        "Grbl numeric precision cannot preserve the requested motion or arc",
    )
}

pub(super) fn output_xy(p: Position, zero: Position, places: usize) -> Result<Point> {
    Ok(Point::new(
        read_number(&decimal(p.x - zero.x, places))?,
        read_number(&decimal(p.y - zero.y, places))?,
    ))
}

fn number(word: &str, letter: char) -> Result<f64> {
    read_number(word.strip_prefix(letter).ok_or_else(subset)?)
}

// Upstream read_float accumulates at most eight decimal digits and scales a 32-bit float.
fn read_number(raw: &str) -> Result<f64> {
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    let mut value = 0u32;
    let mut count = 0;
    let mut exp = 0;
    let mut decimal = false;
    for b in digits.bytes() {
        match b {
            b'.' if !decimal => decimal = true,
            b'0'..=b'9' => {
                count += 1;
                if count <= 8 {
                    value = value * 10 + u32::from(b - b'0');
                    if decimal {
                        exp -= 1;
                    }
                } else if !decimal {
                    exp += 1;
                }
            }
            _ => return Err(subset()),
        }
    }
    if count == 0 {
        return Err(subset());
    }
    let mut n = value as f32;
    while exp <= -2 {
        n *= 0.01;
        exp += 2;
    }
    if exp < 0 {
        n *= 0.1;
    } else {
        for _ in 0..exp {
            n *= 10.;
        }
    }
    if !n.is_finite() {
        return Err(precision());
    }
    Ok(f64::from(if raw.starts_with('-') { -n } else { n }))
}

fn decode<'a>(line: &'a str, at: Option<Position>) -> Result<Action<'a>> {
    if !line.is_ascii() || line.bytes().filter(|b| *b != b' ').count() > 79 {
        return Err(error(
            "GRBL_BLOCK_LENGTH",
            "Grbl executable block must fit 79 ASCII characters",
        ));
    }
    let mut words = line.split_ascii_whitespace();
    let first = words.next().ok_or_else(subset)?;
    let mut next = |c| number(words.next().ok_or_else(subset)?, c);
    let action = match first {
        "M5" | "M9" | "M8" | "M2" | "G54" | "G55" | "G56" | "G57" | "G58" | "G59" => {
            Action::Modal(first)
        }
        "G21" if line == "G21 G17 G90 G94" => return Ok(Action::Modal(line)),
        "M3" | "M4" => Action::Spindle(
            if first == "M3" {
                SpindleDirection::Clockwise
            } else {
                SpindleDirection::Counterclockwise
            },
            next('S')?,
        ),
        "G4" => {
            let seconds = next('P')?;
            if !(0. ..=GrblProfile::MAX_DWELL_SECONDS).contains(&seconds) {
                return Err(error(
                    "GRBL_DWELL",
                    "dwell exceeds the Grbl 1.1 timer range",
                ));
            }
            Action::Dwell(seconds)
        }
        "G0" if line.starts_with("G0 Z") => Action::Z(next('Z')?),
        "G0" => {
            let (x, y) = (next('X')?, next('Y')?);
            match words.next() {
                None => return Ok(Action::Xy(x, y)),
                Some(word) => Action::Rapid(Position {
                    x,
                    y,
                    z: number(word, 'Z')?,
                }),
            }
        }
        "G1" | "G2" | "G3" => {
            let end = Position {
                x: next('X')?,
                y: next('Y')?,
                z: next('Z')?,
            };
            if first == "G1" {
                Action::Line(end, next('F')?)
            } else {
                let start = at.ok_or_else(subset)?;
                let (i, j) = (next('I')? as f32, next('J')? as f32);
                let radius = (i * i + j * j).sqrt();
                let dx = end.x as f32 - start.x as f32 - i;
                let dy = end.y as f32 - start.y as f32 - j;
                let delta = ((dx * dx + dy * dy).sqrt() - radius).abs();
                if radius == 0.
                    || !delta.is_finite()
                    || (delta > 0.005 && (delta > 0.5 || delta > 0.001 * radius))
                {
                    return Err(precision());
                }
                Action::Arc {
                    end,
                    center: (f64::from(start.x as f32 + i), f64::from(start.y as f32 + j)),
                    clockwise: first == "G2",
                    feed: next('F')?,
                }
            }
        }
        _ => return Err(subset()),
    };
    if words.next().is_some() {
        return Err(subset());
    }
    Ok(action)
}

pub(super) fn verify(
    gcode: &str,
    runs: &[Run<'_>],
    profile: &GrblProfile,
    zero: Position,
    tolerance: f64,
) -> Result<()> {
    let mut lines = gcode.lines().enumerate();
    let mut actual = Cursor::default();
    let mut expected = Cursor::default();
    let tol = tolerance + 1e-12;
    let near = |a: f64, b: f64| (a - b).abs() <= tol;
    let point = |a: Position, b: Position| near(a.x, b.x) && near(a.y, b.y) && near(a.z, b.z);
    let scalar = |a: f64, b: f64| (a - b).abs() <= b.abs() * 1e-6 && (b == 0. || a > 0.);
    actions(runs, profile, zero, |want| {
        let (index, line) = lines.next().ok_or_else(subset)?;
        let got = decode(line, actual.at())?;
        let ok = match (got, want) {
            (Action::Modal(a), Action::Modal(b)) => a == b,
            (Action::Z(a), Action::Z(b)) => near(a, b),
            (Action::Xy(x, y), Action::Xy(u, v)) => near(x, u) && near(y, v),
            (Action::Rapid(a), Action::Rapid(b)) => point(a, b),
            (Action::Line(a, f), Action::Line(b, g)) => point(a, b) && scalar(f, g),
            (Action::Dwell(a), Action::Dwell(b)) => scalar(a, b),
            (Action::Spindle(d, a), Action::Spindle(e, b)) => d == e && scalar(a, b),
            (
                Action::Arc {
                    end: a,
                    center: c,
                    clockwise: d,
                    feed: f,
                },
                Action::Arc {
                    end: b,
                    center: k,
                    clockwise: e,
                    feed: g,
                },
            ) => point(a, b) && near(c.0, k.0) && near(c.1, k.1) && d == e && scalar(f, g),
            _ => false,
        };
        if !ok {
            return Err(error(
                "POST_PRECISION",
                format!(
                    "Grbl readback at line {} differs from the required machine action: read {got:?}, expected {want:?}",
                    index + 1
                ),
            ));
        }
        // Grbl interprets a signed sweep within 5e-7 radians of zero as a full
        // turn. Rounding a short planned arc across this boundary can cut a circle.
        if let (
            Action::Arc {
                end,
                center,
                clockwise,
                ..
            },
            Action::Arc {
                end: target,
                center: planned_center,
                ..
            },
            Some(start),
            Some(planned_start),
        ) = (got, want, actual.at(), expected.at())
        {
            let sweep = |start: Position, end: Position, c: (f64, f64)| {
                let (x, y) = (start.x - c.0, start.y - c.1);
                let (u, v) = (end.x - c.0, end.y - c.1);
                let angle = (x * v - y * u).atan2(x * u + y * v);
                if clockwise { -angle } else { angle }
            };
            let actual_sweep = sweep(start, end, center);
            let intended_sweep = sweep(planned_start, target, planned_center);
            if (actual_sweep <= 5e-7) != (intended_sweep <= 0.)
                && (actual_sweep - intended_sweep).abs() < std::f64::consts::PI
            {
                return Err(precision());
            }
        }
        let before = actual.at();
        let intended = expected.at();
        actual.apply(got);
        expected.apply(want);
        if let (Some(a), Some(b), Some(p), Some(q)) = (before, actual.at(), intended, expected.at())
        {
            let collapsed = a == b && p != q;
            let arc_became_circle =
                matches!(want, Action::Arc { .. }) && a.xy() == b.xy() && p.xy() != q.xy();
            if collapsed || arc_became_circle {
                return Err(precision());
            }
        }
        Ok(())
    })?;
    if lines.next().is_some() {
        return Err(subset());
    }
    Ok(())
}
