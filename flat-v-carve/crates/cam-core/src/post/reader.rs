//! Deliberately strict reader for our numeric subset. The grammar and required
//! modal/tool-change sequence are checked independently of the writer. No
//! comments, cached reports, or motion labels authorize executable blocks.
use super::*;

pub(super) struct Readback {
    pub motions: Vec<Motion>,
    pub clearance_links: usize,
}
struct Reader<'a> {
    lines: Vec<(usize, &'a str)>,
    next: usize,
}
impl<'a> Reader<'a> {
    fn new(text: &'a str) -> Result<Self> {
        if text.len() > 128_000_000 || !text.is_ascii() {
            return Err(error(
                "POST_GCODE_SUBSET",
                "program must be ASCII and at most 128 MB",
            ));
        }
        let mut lines = vec![];
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.len() > 240 {
                return Err(error(
                    "POST_GCODE_SUBSET",
                    format!("line {} exceeds 240 characters", i + 1),
                ));
            }
            if line.is_empty() {
                continue;
            }
            if line.starts_with("(CAM ")
                && line.ends_with(')')
                && !line[1..line.len() - 1].contains(['(', ')'])
                && line.bytes().all(|b| (32..=126).contains(&b))
            {
                continue;
            }
            lines.push((i + 1, line));
        }
        Ok(Self { lines, next: 0 })
    }
    fn block(&mut self) -> Result<Vec<(char, f64)>> {
        let (line, text) = self
            .lines
            .get(self.next)
            .copied()
            .ok_or_else(|| error("POST_GCODE_SUBSET", "unexpected end of program"))?;
        self.next += 1;
        let mut words = vec![];
        for token in text.split_ascii_whitespace() {
            let (letter, number) = token.split_at(1);
            if number.is_empty()
                || !letter.as_bytes()[0].is_ascii_uppercase()
                || !number
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'-' | b'+'))
            {
                return Err(error(
                    "POST_GCODE_SUBSET",
                    format!("line {line}: only explicit numeric words are supported"),
                ));
            }
            let value: f64 = number
                .parse()
                .map_err(|_| error("POST_GCODE_SUBSET", format!("line {line}: invalid number")))?;
            if !value.is_finite() || value.abs() > 1_000_000. {
                return Err(error(
                    "POST_GCODE_SUBSET",
                    format!("line {line}: number out of range"),
                ));
            }
            words.push((letter.as_bytes()[0] as char, value));
        }
        Ok(words)
    }
    fn expect(&mut self, expected: &[(char, f64)]) -> Result<()> {
        if self.block()? != expected {
            return Err(error(
                "POST_GCODE_STATE",
                format!(
                    "line {}: unexpected block; required {expected:?}",
                    self.lines[self.next - 1].0
                ),
            ));
        }
        Ok(())
    }
    fn modal(&mut self, work: &str) -> Result<()> {
        self.expect(&[
            ('G', 21.),
            ('G', 17.),
            ('G', 90.),
            ('G', 94.),
            ('G', 40.),
            ('G', 80.),
            ('G', 61.),
        ])?;
        self.expect(&[('G', work[1..].parse().expect("validated work offset"))])?;
        self.expect(&[('G', 92.1)])
    }
    fn xyz(&mut self, g: f64, feed: Option<f64>) -> Result<Position> {
        let words = self.block()?;
        if words.len() != if feed.is_some() { 5 } else { 4 }
            || words[0] != ('G', g)
            || words[1].0 != 'X'
            || words[2].0 != 'Y'
            || words[3].0 != 'Z'
            || feed.is_some_and(|f| words[4] != ('F', f))
        {
            return Err(error(
                "POST_GCODE_MOTION",
                format!(
                    "line {}: expected explicit G{g} XYZ and matching feed",
                    self.lines[self.next - 1].0
                ),
            ));
        }
        Ok(Position {
            x: words[1].1,
            y: words[2].1,
            z: words[3].1,
        })
    }
}

pub(super) fn read(
    text: &str,
    p: &LinuxCncProfile,
    stages: &[&Stage<'_>],
    offset: f64,
) -> Result<Readback> {
    let mut r = Reader::new(text)?;
    r.expect(&[('M', 5.)])?;
    r.expect(&[('M', 9.)])?;
    r.modal(&p.work_offset)?;
    let mut previous_stage_end = p.program_start_position_mm;
    let mut result = Readback {
        motions: vec![],
        clearance_links: 0,
    };
    let clearance = rounded(p.clearance_z_mm + offset, p.decimal_places);
    for stage in stages {
        if previous_stage_end.is_some_and(|s| s.z != clearance) {
            return Err(error(
                "POST_TOOL_CHANGE",
                "tool change caller must be at clearance",
            ));
        }
        let tool = p.tool(stage.id);
        r.expect(&[('M', 5.)])?;
        r.expect(&[('M', 9.)])?;
        r.expect(&[('T', tool.tool_number as f64), ('M', 6.)])?;
        r.expect(&[('M', 5.)])?;
        r.expect(&[('M', 9.)])?;
        // All cutting modes are required anew after each opaque macro call.
        r.modal(&p.work_offset)?;
        match p.length_compensation {
            LengthCompensation::ToolTable => {
                r.expect(&[('G', 43.), ('H', tool.length_offset_number.unwrap() as f64)])?
            }
            LengthCompensation::MacroManaged => {}
        }
        let mut current = p.returned_position(previous_stage_end);
        if let M6Return::SafeRetract {
            z_mm,
            transit_xy_mm,
        } = p.m6.return_position
        {
            r.expect(&[('G', 0.), ('Z', z_mm)])?;
            let end = r.xyz(0., None)?;
            if end != Position::new(transit_xy_mm, z_mm) {
                return Err(error(
                    "POST_M6_RETRACT",
                    "safe retract must precede XY transit at the declared safe Z",
                ));
            }
            // These two blocks establish the first known position. Their
            // clearance is the explicit machine contract, not an M5 claim.
        }
        let start = machine_position(stage.motions[0].start, p, offset);
        if start.z != clearance || current.z < clearance {
            return Err(error(
                "POST_CLEARANCE",
                "stage entry must be at clearance and M6 must return at or above it",
            ));
        }
        if current.z != start.z {
            r.expect(&[('G', 0.), ('Z', start.z)])?;
            current.z = start.z;
            result.clearance_links += 1;
        }
        if current != start {
            let end = r.xyz(0., None)?;
            if end != start {
                return Err(error(
                    "POST_CLEARANCE_LINK",
                    "clearance link does not end at the stage's recorded start",
                ));
            }
            current = end;
            result.clearance_links += 1;
        }
        r.expect(&[('G', 97.), ('S', stage.spindle)])?;
        r.expect(&[(
            'M',
            match tool.spindle_direction {
                SpindleDirection::Clockwise => 3.,
                SpindleDirection::Counterclockwise => 4.,
            },
        )])?;
        r.expect(&[('G', 4.), ('P', p.spindle_spinup_seconds)])?;
        r.expect(&[(
            'M',
            match p.coolant {
                Coolant::Off => 9.,
                Coolant::Flood => 8.,
                Coolant::Mist => 7.,
            },
        )])?;
        for expected in stage.motions {
            let end = r.xyz(
                if expected.kind.rapid() { 0. } else { 1. },
                expected.feed_mm_min,
            )?;
            if current != machine_position(expected.start, p, offset)
                || end != machine_position(expected.end, p, offset)
            {
                return Err(error(
                    "POST_MOTION_MISMATCH",
                    format!(
                        "motion {}: numeric endpoints differ from the translated, formatted plan",
                        expected.id
                    ),
                ));
            }
            let mut decoded = expected.clone();
            decoded.start = stock_position(current, offset);
            decoded.end = stock_position(end, offset);
            let dot = (decoded.end.x - decoded.start.x) * (expected.end.x - expected.start.x)
                + (decoded.end.y - decoded.start.y) * (expected.end.y - expected.start.y)
                + (decoded.end.z - decoded.start.z) * (expected.end.z - expected.start.z);
            if decoded.start == decoded.end
                || dot <= 0.
                || expected.kind == MotionKind::Cut && decoded.start.xy() == decoded.end.xy()
                || matches!(
                    expected.kind,
                    MotionKind::Approach
                        | MotionKind::Plunge
                        | MotionKind::Ramp
                        | MotionKind::RapidRetract
                ) && expected.start.z != expected.end.z
                    && decoded.start.z == decoded.end.z
            {
                return Err(error(
                    "POST_ROUNDING",
                    format!(
                        "motion {} (tool {}, {} decimal places) collapses, reverses, or loses required travel after output formatting: {:?} -> {:?}",
                        expected.id, expected.tool_id, p.decimal_places, current, end
                    ),
                ));
            }
            result.motions.push(decoded);
            current = end;
        }
        if current.z != clearance {
            return Err(error(
                "POST_FINAL_RETRACT",
                "stage does not finish at clearance",
            ));
        }
        previous_stage_end = Some(current);
    }
    r.expect(&[('M', 5.)])?;
    r.expect(&[('M', 9.)])?;
    r.expect(&[('M', 2.)])?;
    if r.next != r.lines.len() {
        return Err(error(
            "POST_GCODE_SUBSET",
            "unexpected blocks after program end",
        ));
    }
    Ok(result)
}
