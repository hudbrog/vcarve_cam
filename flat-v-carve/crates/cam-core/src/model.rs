//! Validated dimensions. Depth is positive downward; machine Z is negative below stock top.
use crate::geometry::{Diagnostic, Result};
use serde::{Deserialize, Serialize};

fn invalid(code: &str, message: &str) -> Diagnostic {
    Diagnostic::new(code, message).at_stage("model")
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Length(f64);

impl Length {
    pub fn new(mm: f64) -> Result<Self> {
        if !mm.is_finite() || mm < 0.0 {
            return Err(invalid(
                "INVALID_LENGTH",
                "length must be finite and nonnegative",
            ));
        }
        Ok(Self(if mm == 0.0 { 0.0 } else { mm }))
    }
    pub fn mm(self) -> f64 {
        self.0
    }
}
impl TryFrom<f64> for Length {
    type Error = Diagnostic;
    fn try_from(v: f64) -> Result<Self> {
        Self::new(v)
    }
}
impl From<Length> for f64 {
    fn from(v: Length) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Depth(f64);
impl Depth {
    pub fn new(mm: f64) -> Result<Self> {
        Ok(Self(Length::new(mm)?.mm()))
    }
    pub fn mm(self) -> f64 {
        self.0
    }
    pub fn machine_z_mm(self) -> f64 {
        if self.0 == 0.0 { 0.0 } else { -self.0 }
    }
}
impl TryFrom<f64> for Depth {
    type Error = Diagnostic;
    fn try_from(v: f64) -> Result<Self> {
        Self::new(v)
    }
}
impl From<Depth> for f64 {
    fn from(v: Depth) -> Self {
        v.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct IncludedAngle(f64, f64);
impl IncludedAngle {
    pub fn new(degrees: f64) -> Result<Self> {
        let slope = (degrees.to_radians() / 2.0).tan();
        if !degrees.is_finite()
            || degrees <= 0.0
            || degrees >= 180.0
            || !slope.is_finite()
            || slope <= 0.0
        {
            return Err(invalid(
                "INVALID_ANGLE",
                "included angle must be finite and strictly between 0 and 180 degrees",
            ));
        }
        Ok(Self(degrees, slope))
    }
    pub fn degrees(self) -> f64 {
        self.0
    }
    pub fn slope(self) -> f64 {
        // Validated and immutable; analytic stock queries reuse this millions
        // of times. Serialization still contains only the original degrees.
        self.1
    }
}
impl TryFrom<f64> for IncludedAngle {
    type Error = Diagnostic;
    fn try_from(v: f64) -> Result<Self> {
        Self::new(v)
    }
}
impl From<IncludedAngle> for f64 {
    fn from(v: IncludedAngle) -> Self {
        v.0
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EndmillSpec {
    pub diameter_mm: f64,
    pub cutting_length_mm: f64,
    pub plunge_capable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct Endmill {
    radius: Length,
    cutting_length: Length,
    plunge_capable: bool,
}
impl TryFrom<EndmillSpec> for Endmill {
    type Error = Diagnostic;
    fn try_from(spec: EndmillSpec) -> Result<Self> {
        if !spec.diameter_mm.is_finite()
            || spec.diameter_mm <= 0.0
            || !spec.cutting_length_mm.is_finite()
            || spec.cutting_length_mm <= 0.0
        {
            return Err(invalid(
                "INVALID_ENDMILL",
                "endmill diameter and cutting length must be finite and positive",
            ));
        }
        let radius = Length::new(spec.diameter_mm / 2.0)?;
        if radius.mm() == 0.0 {
            return Err(invalid("INVALID_ENDMILL", "endmill radius underflows"));
        }
        Ok(Self {
            radius,
            cutting_length: Length::new(spec.cutting_length_mm)?,
            plunge_capable: spec.plunge_capable,
        })
    }
}
impl Endmill {
    pub fn radius(&self) -> Length {
        self.radius
    }
    pub fn cutting_length(&self) -> Length {
        self.cutting_length
    }
    pub fn plunge_capable(&self) -> bool {
        self.plunge_capable
    }
    pub fn validate_depth(&self, depth: Depth) -> Result<()> {
        if depth.mm() > self.cutting_length.mm() {
            return Err(invalid(
                "ENDMILL_CUTTING_LENGTH",
                "requested depth exceeds the endmill cutting length",
            ));
        }
        Ok(())
    }
    pub fn removal_depth(&self, depth: Depth, radial_distance: Length) -> Result<Depth> {
        self.validate_depth(depth)?;
        Depth::new(if radial_distance <= self.radius {
            depth.mm()
        } else {
            0.0
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VBitSpec {
    pub included_angle_deg: f64,
    pub tip_diameter_mm: f64,
    pub max_cutting_diameter_mm: f64,
    pub cutting_height_mm: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct VBit {
    angle: IncludedAngle,
    tip_radius: Length,
    max_cutting_radius: Length,
    cutting_height: Length,
}
impl TryFrom<VBitSpec> for VBit {
    type Error = Diagnostic;
    fn try_from(spec: VBitSpec) -> Result<Self> {
        let angle = IncludedAngle::new(spec.included_angle_deg)?;
        if !spec.tip_diameter_mm.is_finite()
            || spec.tip_diameter_mm < 0.0
            || !spec.max_cutting_diameter_mm.is_finite()
            || spec.max_cutting_diameter_mm <= spec.tip_diameter_mm
            || !spec.cutting_height_mm.is_finite()
            || spec.cutting_height_mm <= 0.0
        {
            return Err(invalid(
                "INVALID_VBIT",
                "V-bit requires a nonnegative tip, a larger positive cutting diameter, and positive cutting height",
            ));
        }
        let tip_radius = Length::new(spec.tip_diameter_mm / 2.0)?;
        let max_cutting_radius = Length::new(spec.max_cutting_diameter_mm / 2.0)?;
        let top_radius = tip_radius.mm() + spec.cutting_height_mm * angle.slope();
        let reserve = 32.0 * f64::EPSILON * max_cutting_radius.mm();
        if !top_radius.is_finite()
            || top_radius > max_cutting_radius.mm() + reserve
            || max_cutting_radius.mm() == 0.0
            || (spec.tip_diameter_mm > 0.0 && tip_radius.mm() == 0.0)
        {
            return Err(invalid(
                "INCONSISTENT_VBIT",
                "angle, tip, cutting diameter and height do not describe a usable cutting cone",
            ));
        }
        Ok(Self {
            angle,
            tip_radius,
            max_cutting_radius,
            cutting_height: Length::new(spec.cutting_height_mm)?,
        })
    }
}
impl VBit {
    pub fn angle(&self) -> IncludedAngle {
        self.angle
    }
    pub fn tip_radius(&self) -> Length {
        self.tip_radius
    }
    pub fn max_cutting_radius(&self) -> Length {
        self.max_cutting_radius
    }
    pub fn cutting_height(&self) -> Length {
        self.cutting_height
    }
    pub fn validate_depth(&self, depth: Depth) -> Result<()> {
        if depth.mm() > self.cutting_height.mm() {
            return Err(invalid(
                "VBIT_CUTTING_HEIGHT",
                "requested depth exceeds usable V-bit cutting height; the target is not clamped",
            ));
        }
        Ok(())
    }
    pub fn radius_at_height(&self, height: Length) -> Result<Length> {
        if height > self.cutting_height {
            return Err(invalid(
                "VBIT_CUTTING_HEIGHT",
                "height exceeds the modeled cutting cone",
            ));
        }
        Length::new(self.tip_radius.mm() + height.mm() * self.angle.slope())
    }
    pub fn removal_depth(&self, depth: Depth, radial_distance: Length) -> Result<Depth> {
        self.validate_depth(depth)?;
        let r = radial_distance.mm();
        Depth::new(if r > self.max_cutting_radius.mm() {
            0.0
        } else {
            (depth.mm() - (r - self.tip_radius.mm()).max(0.0) / self.angle.slope()).max(0.0)
        })
    }
}
