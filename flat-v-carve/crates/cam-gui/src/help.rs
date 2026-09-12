//! Context help is shared by inspector and library controls. Hover for a quick
//! explanation; click to keep it open while reading.
use super::*;

pub(super) fn icon(ui: &mut egui::Ui, label: &str) {
    let Some(body) = explanation(label) else {
        return;
    };
    ui.push_id(("setting-help", label), |ui| {
        ui.spacing_mut().button_padding = egui::vec2(2., 1.);
        let r = ui.add(
            egui::Button::new("?")
                .corner_radius(20)
                .min_size(egui::vec2(18., 18.))
                .small(),
        );
        observe_control(&format!("Help {label}"), r.rect);
        let r = r.on_hover_text(body);
        egui::Popup::from_toggle_button_response(&r).show(|ui| {
            ui.set_max_width(340.);
            ui.strong(label);
            ui.label(body);
            if label.contains("M6")
                || label.contains("Contract")
                || label.contains("compensation")
                || label.contains("tolerance")
                || label.contains("offset")
                || label.contains("Tool number")
            {
                ui.hyperlink_to(
                    "LinuxCNC command reference",
                    "https://linuxcnc.org/docs/stable/html/gcode/g-code.html",
                );
            }
        });
    });
}
pub(super) fn label(ui: &mut egui::Ui, name: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(name);
        icon(ui, name);
    });
}

pub(super) fn explanation(label: &str) -> Option<&'static str> {
    if let Some(field) = FIELDS.iter().position(|s| *s == label) {
        return Some(FIELD_HELP[field]);
    }
    let label = if label == "Job compensation" {
        "Job length compensation"
    } else {
        label
    };
    let name = label
        .strip_prefix("Library ")
        .or_else(|| label.strip_prefix("Copied "))
        .unwrap_or(label);
    Some(match name {
        "tool name" => {
            "Your name for this physical cutter. Different tools may have the same diameter; changing the name does not merge them."
        }
        "diameter" => FIELD_HELP[12],
        "cutting length" => FIELD_HELP[13],
        "V-bit angle" => FIELD_HELP[16],
        "tip diameter" => FIELD_HELP[17],
        "cutting diameter" => FIELD_HELP[18],
        "cutting height" => FIELD_HELP[19],
        "plunge" | "Endmill can plunge" | "V-bit can plunge" => {
            "Can this cutter feed straight down into material? Choose Yes only for a cutter designed for it (for example a center-cutting endmill). This capability is copied with library geometry; the operation still chooses plunge or ramp entry."
        }
        "ramp" | "Endmill can ramp" => {
            "Can this tool cut while moving sideways and down together? This is a cutter capability. Ramp angle and ramp feed are separate operation settings."
        }
        "rotation" | "Endmill direction" | "V-bit direction" => {
            "Spindle rotation: CW emits M3, CCW emits M4. For a conventional right-hand cutter use CW, viewed from the spindle toward the work. This is spindle rotation, not climb/conventional path direction. Library rotation is copied when applying the tool; the job can override it."
        }
        "blade offset" => {
            "Distance in millimeters between the passive knife's pivot axis and cutting tip. Use the blade manufacturer's geometry."
        }
        "cut depth" => "Maximum supported depth of this knife blade, in millimeters.",
        "Profile name" => {
            "A descriptive name for this set of cutting values, such as Plywood roughing. Applying it copies the values into one job assignment."
        }
        "Profile material" => {
            "Optional material note to help choose the profile. It does not alter feeds automatically."
        }
        "Profile machine context" => {
            "Optional note about the machine for which these cutting values were chosen. It does not apply a machine configuration."
        }
        "Profile spindle RPM" => FIELD_HELP[11],
        "Profile cutting feed" => FIELD_HELP[2],
        "Profile plunge feed" => FIELD_HELP[10],
        "Profile stepdown" => FIELD_HELP[8],
        "Profile stepover" => FIELD_HELP[9],
        "Profile swivel feed" => FIELD_HELP[65],
        "New machine ID" => {
            "A short unique name for your machine configuration, for example workshop-router. This is a library identifier, not a controller address."
        }
        "Work offset" | "Job work offset" => {
            "Select the controller work coordinate system (G54, G55, etc.) whose zero you set for this stock. The CAM work zero is the corresponding point on the job. Choose the same work offset on the machine."
        }
        "Configuration clearance" => FIELD_HELP[7],
        "Configuration precision" => FIELD_HELP[35],
        "Configuration spinup seconds" => FIELD_HELP[34],
        "Length compensation" | "Job length compensation" => {
            "Tool table: output uses G43 Hn to apply the measured tool length in controller table entry Hn. Macro managed: your M6/tool-change procedure applies the measured Z compensation itself; this program emits no G43 H and H fields are unnecessary. Choose the method your controller actually uses."
        }
        "Coolant" | "Job coolant" => {
            "Off, flood (M8), or mist (M7), as supported by your machine. This configures coolant output; it does not affect simulated stock removal."
        }
        "Path control" | "Job path control" => {
            "Exact path (G61) follows programmed points, slowing as needed. Blend (G64) permits smoothing within the chosen tolerance and can run short segments more smoothly. This affects controller motion, not the CAM geometry."
        }
        "Configuration blend tolerance" => FIELD_HELP[36],
        "Configuration naive CAM tolerance" => {
            "LinuxCNC G64 Q, in millimeters: merge nearly collinear short feed moves into longer moves. Larger values can run smoother but discard more small detail. Example: 0.01 mm. Zero disables this simplification; when Q is omitted, LinuxCNC uses P. Active only in Blend mode. This is controller smoothing, separate from CAM planning tolerance."
        }
        "Explicit program start position" => {
            "Optional machine-contract position before this program's first move, in the selected work coordinates. Set it only if your setup procedure guarantees that position. It is different from Stock setup Start XY, which controls planning order."
        }
        "Program start X" | "Program start Y" | "Program start Z" => {
            "The guaranteed tool-tip position before the program begins, in the selected work coordinate system and millimeters. This must describe the real setup position, including Z; it is not the planner's preferred start point."
        }
        "M6 contract" => {
            "M6 is the controller's tool-change command. CAM cannot inspect your machine's M6 macro, tool setter or manual change procedure. Describe what it guarantees so checked output can safely join motions before and after a change. A manual tool change still needs a known return position and correct tool length compensation."
        }
        "Contract reference" => {
            "A note identifying how you checked your tool-change behavior: a manual section, macro filename/revision, or a description of your verified procedure. Example: 'toolchange.ngc revision 3: probe tool, restore work datum, return at clearance'. It is not executable code, a password, or a special reference number."
        }
        "Contract reviewed" => {
            "Confirm that you checked the notes and return behavior against your real controller or tool-change procedure. Completing this checkbox alone does not establish the other guarantees."
        }
        "Preserves work datum" => {
            "After M6, the same work offset/stock zero still applies, with no work-frame rotation. The new tool tip must refer to the same stock coordinates after length compensation."
        }
        "Local offsets unused" => {
            "The tool-change procedure leaves G52/G92 local coordinate shifts unused. This exporter cannot account for a hidden local shift left active by the macro."
        }
        "Z-only tool offsets" => {
            "Tool-length compensation changes Z only. This output workflow does not model lateral (X/Y) tool offsets or a rotated coordinate frame."
        }
        "M6 return" => {
            "Caller: the new tool tip returns to the position where M6 was called, after compensation. Fixed: it ends at the specified work-coordinate XYZ. Safe retract: your procedure guarantees an unobstructed upward retract and XY transit at the specified Z. Pick the behavior you have actually checked; none is universal."
        }
        "M6 fixed X" | "M6 fixed Y" | "M6 fixed Z" => {
            "The new tool tip's guaranteed position after M6 and length compensation, in millimeters in the selected work coordinates. Values must match the machine procedure."
        }
        "M6 retract Z" => {
            "The work-coordinate Z height at which your machine procedure guarantees clear XY travel after changing the tool. This is an absolute work Z, not the generic clearance distance above stock."
        }
        "M6 transit X" | "M6 transit Y" => {
            "The XY position used by the machine-owned safe-retract transition, in work coordinates. Enter values guaranteed by your M6 procedure."
        }
        "Operation top" => {
            "The height from which this operation measures its depth. Stock top is the common carving reference; the signed top offset adjusts it."
        }
        "Endmill entry" => {
            "Plunge feeds straight down and requires a plunge-capable cutter. Ramp feeds sideways while descending and requires a ramp-capable cutter, with a ramp angle and feed. Select the entry strategy supported by your tool and available space."
        }
        "Work zero" => {
            "The point you will set as zero on the machine for this stock. It shifts output coordinates; simulated geometry remains in setup coordinates."
        }
        "From library" => {
            "Select a saved cutter and optionally a cutting profile, then Apply. This copies geometry, capabilities and specified spindle direction, plus selected cutting values, into this job. Later library edits do not silently change the job."
        }
        "Assigned job tool" => {
            "The physical tool copy used by this assignment. Reusing a job tool shares geometry; cutting values remain specific to each assignment."
        }
        "Carving mode" => {
            "Endmill only clears as much of the V-shaped target as the endmill can reach. Combined also runs the V-bit finishing stage for the remaining detail. Both modes need V-bit geometry to define the target shape; only Combined needs V-bit cutting values and a controller tool number."
        }
        "Clearing strategy" => {
            "Depth-dependent clearing computes the reachable region separately at each depth. Deepest-region clearing uses the region reachable at the deepest cut for all layers. These can produce different coverage on sloped V-shaped targets."
        }
        _ if label.starts_with("Configuration T ") => FIELD_HELP[32],
        _ if label.starts_with("Configuration H ") => FIELD_HELP[33],
        _ => return None,
    })
}

const FIELD_HELP: [&str; 89] = [
    "Total carving depth below the operation top, in millimeters. Stepdown controls how much is removed in each pass.",
    "Material left on walls by roughing, in millimeters, for a later finishing pass. Zero requests no extra allowance.",
    "Endmill cutting speed along the path, in mm/min. Choose for your cutter, material and machine; it is not spindle RPM.",
    "V-bit finishing speed along the path, in mm/min.",
    "Maximum requested floor ridge left by the endmill clearing passes, in millimeters.",
    "Allowed remaining detail for the V-bit finishing planner, in millimeters. Smaller values demand finer coverage.",
    "Actual stock thickness, in millimeters. New jobs start at 18 mm; change it to match your material, especially when using stock-bottom Z zero.",
    "Travel height above the original stock top, in millimeters. New jobs use 5 mm. Set it high enough for your real fixtures and stock; this is not machine-coordinate Z.",
    "Maximum endmill depth removed in one pass, in millimeters. Smaller steps require more passes.",
    "Spacing between adjacent endmill clearing passes, in millimeters, not a percentage of diameter.",
    "Downward endmill entry feed, in mm/min. Use a suitable value for the tool's plunge capability.",
    "Spindle rotation speed in revolutions per minute (RPM). Rotation direction is a separate setting.",
    "Endmill cutting diameter in millimeters. Used for path clearance and stock removal, not controller tool numbering.",
    "Usable flute/cutting length in millimeters, not total tool length or tool-table offset. Must support the requested cut depth.",
    "Maximum downward ramp angle in degrees. A shallower ramp needs more horizontal room. Requires a ramp-capable tool.",
    "Entry length in millimeters. This field is reserved and is not used by the current carving workspace.",
    "Full included V-bit angle in degrees (for example 90), not the half-angle. It determines the wall slope.",
    "Flat diameter at the V-bit tip in millimeters. Zero describes a mathematically sharp tip; use the actual cutter geometry.",
    "Largest usable V-bit cutting diameter in millimeters.",
    "Usable V-bit cutting height in millimeters, not total tool length or the controller's length offset.",
    "Maximum vertical depth of a V-bit finishing pass, in millimeters.",
    "V-bit downward entry feed in mm/min.",
    "V-bit spindle speed in RPM; its rotation direction is set separately.",
    "CAM motion approximation tolerance in millimeters. Default 0.01 mm. Smaller values retain finer geometry but can produce more segments and take longer to plan. This is independent of controller G64 smoothing.",
    "Geometry import approximation tolerance. The current workspace uses the artwork import settings rather than this reserved field.",
    "Tolerance used by the planner's geometric verification, in millimeters. Default 0.05 mm. It controls acceptance of geometric error, not cutter diameter or machine calibration.",
    "SVG page X origin in millimeters. The placement maps page coordinates relative to this origin into setup coordinates.",
    "SVG page Y origin in millimeters. Placement uses a bottom-up page axis in setup millimeters.",
    "Artwork rotation in degrees about its placement origin. Positive angles rotate counterclockwise in the setup XY plane.",
    "Uniform artwork scale factor: 1 keeps the imported physical size, 2 doubles it. This also affects Stock XY from SVG page capture.",
    "Planner start X in setup millimeters. Default 0. It helps choose the first approach/path order. Use another point when you want to start near a particular side or feature. It does not set work zero or assert the physical machine's starting position.",
    "Planner start Y in setup millimeters. Default 0. Both X and Y form the planning start point; they are independent of the controller's actual program-start position.",
    "Controller tool number T, an integer such as 3 for T3 M6. This is the cutter's controller/tool-table identifier, not its diameter. Enter the number used by your machine for this physical tool. A machine profile can be complete while a newly selected job tool still needs a number.",
    "Controller tool-length table entry H, an integer, not a length in millimeters. G43 H3 applies the measured offset stored in table entry 3. H often equals T, but verify your controller table. Required only with Tool table compensation; hidden with Macro managed compensation.",
    "Pause after starting the spindle, in seconds, to let it reach speed before cutting.",
    "Minimum number of decimal places in output coordinates. More places reduce rounding error; export may increase precision to preserve small motions. This is distinct from CAM planning tolerance.",
    "LinuxCNC G64 P tolerance in millimeters: allowed blending deviation around programmed path points. Smaller values preserve detail more closely, but may slow motion. Only used in Blend mode.",
    "Recovery save interval. This field is reserved; recovery timing is managed automatically by this workspace.",
    "Controller T number for the V-bit when Combined mode executes it. Use the controller's number for that physical V-bit, independently of the endmill number.",
    "Controller H table entry for the V-bit's measured Z length compensation. This is a table index, not millimeters; used only in Tool table mode.",
    "Left edge of the physical stock rectangle in setup millimeters.",
    "Bottom edge of the physical stock rectangle in setup millimeters.",
    "Physical stock extent along setup X, in millimeters. Page capture uses the full placed SVG page, not just its drawn shapes.",
    "Physical stock extent along setup Y, in millimeters.",
    "Custom work-zero X in setup coordinates. The selected point becomes output X=0 in the machine's chosen work offset.",
    "Custom work-zero Y in setup coordinates. The selected point becomes output Y=0 in the machine's chosen work offset.",
    "Spacing between V-bit finishing passes in millimeters, not a percentage.",
    "Signed height adjustment to the operation top, in millimeters. Positive is upward; cut depth is measured downward from that adjusted top.",
    "Maximum roughing depth layers the planner may create. This is a computation limit, not a desired number of passes.",
    "Maximum clearing loops per roughing layer. Increase only when a valid complex job exceeds the limit.",
    "Maximum roughing motions allowed before planning reports a resource limit.",
    "Feed while ramping downward, in mm/min. Used only with Ramp entry; separate from straight plunge feed.",
    "Maximum number of finishing paths allowed by the planner.",
    "Maximum finishing motion count. This bounds computation and output size.",
    "Maximum curve segments allowed during V-bit planning. Finer tolerances can require more segments.",
    "Maximum depth passes allowed for finishing. Actual passes also depend on depth and stepdown.",
    "Maximum additional finishing cleanup iterations. Zero disables extra cleanup iterations.",
    "Spacing of finishing quality samples in millimeters. Finer spacing inspects more locations and costs more computation.",
    "Maximum finishing quality samples. This is a resource cap, not a machining feed or accuracy value.",
    "Maximum cells used for finishing reachability analysis. Increase only when an otherwise valid job hits this limit.",
    "Number of stock slices used for finishing analysis. This controls the planner's internal stock approximation, not the playback timeline.",
    "Distance in millimeters from the passive knife pivot to its cutting tip. Use the physical blade geometry.",
    "Maximum cutting depth supported by this knife blade, in millimeters. This is a tool limit, not the requested cut depth.",
    "Knife cutting feed in millimeters per minute along the compensated holder path.",
    "Knife plunge feed in millimeters per minute for lowering the blade into material.",
    "Holder feed in millimeters per minute during passive corner swivels. The blade must remain engaged in material.",
    "Maximum depth increment allowed by this tool assignment, in millimeters. Applying a knife profile copies this limit.",
    "Requested depth increment between knife passes, in millimeters. The planner checks it against the tool assignment limit.",
    "Blade engagement below the operation top while swiveling, in millimeters. Lifting completely clear cannot establish passive heading.",
    "Corner angle threshold in degrees used by the knife planner to choose supported corner handling.",
    "Optional additional through-cut allowance in millimeters. Leave blank when no extra allowance is intended; confirm physical backing separately.",
    "Optional overlap length in millimeters past closure on a closed knife chain. Open chains do not receive artificial closing segments.",
    "Explicit initial blade heading in degrees counterclockwise from +X, pointing from pivot toward tip. It must match the physical blade alignment.",
    "Signed height offset in millimeters from the selected knife top reference. Positive is upward, negative is downward.",
    "Signed height offset in millimeters from the selected knife bottom reference. A negative stock-top offset sets a cut below stock top.",
    "Facing pass angle in degrees. Only 0 (rows along X) and 90 (rows along Y) ship in this milestone; another angle is rejected with a located reason.",
    "Travel beyond the requested face coverage at each pass entry, in millimeters. This is allowed overhang, not a claim that material outside the request is faced.",
    "Travel beyond the requested face coverage at each pass exit, in millimeters. This is allowed overhang, not a claim that material outside the request is faced.",
    "Coverage expansion past the requested area's minimum X edge, in millimeters. Positive values extend outward and must be nonnegative.",
    "Coverage expansion past the requested area's maximum X edge, in millimeters. Positive values extend outward and must be nonnegative.",
    "Coverage expansion past the requested area's minimum Y edge, in millimeters. Positive values extend outward and must be nonnegative.",
    "Coverage expansion past the requested area's maximum Y edge, in millimeters. Positive values extend outward and must be nonnegative.",
    "Left edge of the requested face rectangle in setup millimeters. The rectangle must have positive width and length.",
    "Bottom edge of the requested face rectangle in setup millimeters. The rectangle must have positive width and length.",
    "Requested face rectangle extent along setup X in millimeters. Coverage beyond this comes only from the margins.",
    "Requested face rectangle extent along setup Y in millimeters. Coverage beyond this comes only from the margins.",
    "Signed height offset in millimeters from the selected face top reference. Positive is upward; the facing depth starts at this adjusted top.",
    "Signed height offset in millimeters from the selected face bottom reference. A negative stock-top offset sets how deeply the face removes material.",
    "Maximum depth increment allowed by this tool assignment. The requested face stepdown is clamped to this tool limit.",
];

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_numeric_field_and_machine_contract_control_has_specific_help() {
        for name in FIELDS.iter().chain(
            [
                "Configuration naive CAM tolerance",
                "Contract reference",
                "Contract reviewed",
                "M6 return",
                "Local offsets unused",
                "Length compensation",
                "Library rotation",
            ]
            .iter(),
        ) {
            assert!(explanation(name).is_some_and(|s| s.len() > 30), "{name}");
        }
    }
}
