# FreeCAD Sketcher shortcut compatibility

The native and browser sketch editors use FreeCAD **1.1.4 default command bindings**.
This is a compatibility map for the geometry and constraints currently supported by
Flat V-Carve. It does not imply support for every FreeCAD tool or selection combination.
Open **Shortcuts** in the sketch tools panel for the same catalogue inside the app.

Press comma-separated keys in sequence, without Ctrl or Alt. Prefixes expire after
two seconds; Escape cancels them. **C** waits 0.7 seconds for **C,C**, then applies
unified coincidence. Commands are disabled while a text field owns keyboard input.
Unknown sequences and known missing commands cannot fall through into bare-letter
constraints. Missing commands display their name and availability.

## Available commands

| Keys | Command |
| --- | --- |
| Q, P | Align View to Sketch |
| G, N | Toggle Construction Geometry |
| D | Dimension |
| A | Horizontal/Vertical Constraint |
| H | Horizontal Constraint |
| V | Vertical Constraint |
| K, L | Lock Position |
| K, B | Block Constraint |
| C | Coincident Constraint |
| C, C | Coincident Constraint |
| O | Point-On-Object Constraint |
| K, D | Distance Dimension |
| L | Horizontal Dimension |
| I | Vertical Dimension |
| P | Parallel Constraint |
| N | Perpendicular Constraint |
| T | Tangent/Collinear Constraint |
| K, R | Radius Dimension |
| K, O | Diameter Dimension |
| K, S | Radius/Diameter Dimension |
| R | Radius/Diameter Dimension |
| K, A | Angle Dimension |
| E | Equal Constraint |
| S | Symmetric Constraint |
| K, X | Toggle Driving/Reference Constraints |
| G, Y | Point |
| G, L | Line |
| G, M | Polyline |
| G, 3, A | Arc From 3 Points |
| G, C | Circle From Center |
| G, R | Rectangle |
| Z, O | Select Origin |
| Z, V | Select Vertical Axis |
| Z, H | Select Horizontal Axis |
| Z, E | Select Associated Geometry |
| Z, R | Remove Axes Alignment |

### Selection and editing behavior

- **D** infers line length, circle diameter, arc radius, angle between two lines,
  or point distance. Explicit dimension keys use the same value editor: the current
  value is selected immediately, Enter saves, Escape discards the value edit, and
  selecting the next geometry saves to the previous operands. Invalid values keep
  the original selection and an error. Repeating the command reopens an existing
  matching dimension instead of duplicating it.
- **A** chooses the nearer horizontal/vertical alignment for one line or two points.
  **H/V** also accept multiple lines, or two points (implemented as a zero Y/X
  distance). These alignment constraints remain driving in reference dimension mode.
- **C** accepts two points, two circle/arc centres (concentricity), or a point and
  a line/circle. **C,C** is strict coincidence; **O** is point-on-line/circle. Point
  and curve may be selected in either order. The default FreeCAD preference for
  unified coincidence is used; FreeCAD's alternative preference swaps C,C and C,O.
- **E** accepts two lines or two circles/arcs. **T** accepts a line and a circle/arc,
  in either order, or two lines for collinearity. Collinearity creates two
  point-on-line constraints in one undo step. Curve tangency uses the supporting
  circle; endpoint coincidence remains separate. Curve-to-curve tangency is absent.
- **S** accepts two points plus a symmetry line, or a point and a line for midpoint.
  Three-point symmetry and other unsupported selections retain selection guidance.
- **K,D** accepts a selected line, two points, or a point and a fixed axis.
  **L/I** accept a line, one/two points, or a point and the perpendicular axis.
  Point distances are signed A minus B; line X/Y dimensions are end minus start.
  **K,A** accepts two lines, or one line measured from the positive X axis.
- **R / K,S** choose circle diameter or arc radius. R represents FreeCAD's default
  radius/diameter flyout command; this editor does not remember a different flyout tool.
- **K,L** creates X and Y dimensions for selected points in one undo step. One point
  is measured from the origin; multiple points are measured from the last selected
  point. **K,B** blocks an entity, or fixes a selected endpoint/centre/point.
- **K,X** immediately converts the selected dimension between driving and reference.
  Press Tab to leave its numeric field first. Without an edited constraint, it
  switches the creation mode shown by **New dimensions are reference**. Non-dimensional
  constraints cannot be reference. Reference creation has no numeric input; select
  the next geometry or use Add constraint to finish.
- **Z,O / Z,H / Z,V** add or remove the fixed origin/axes in the current selection.
  **Z,E** selects the geometry of the current list/dimension constraint.
  **Z,R** removes H/V constraints from selected lines in one undo step.
- **G,N** toggles construction for the selected complete wires in Select mode,
  or for new geometry while drawing. **Q,P** acknowledges the local XY view;
  the editor already uses this plane and has no 3D sketch orientation.

## Commands whose tools are not available yet

These keys are recognized and report the missing command. They never invoke a
similar-looking tool with a different construction method (for example, G,A is an
arc from centre, while our three-point arc is G,3,A).

| Keys | Command |
| --- | --- |
| Q, S | Toggle Section View |
| K, W | Refraction Constraint |
| K, Z | Toggle Constraints |
| G, A | Arc From Center |
| G, E, A | Elliptical Arc |
| G, H | Hyperbolic Arc |
| G, J | Parabolic Arc |
| G, 3, C | Circle From 3 Points |
| G, E, E | Ellipse From Center |
| G, 3, E | Ellipse From 3 Points |
| G, V | Centered Rectangle |
| G, O | Rounded Rectangle |
| G, P, 3 | Triangle |
| G, P, 4 | Square |
| G, P, 5 | Pentagon |
| G, P, 6 | Hexagon |
| G, P, 7 | Heptagon |
| G, P, 8 | Octagon |
| G, P, R | Polygon |
| G, S | Slot |
| G, S, 2 | Arc Slot |
| G, B, B | B-Spline |
| G, B, P | Periodic B-Spline |
| G, B, I | B-Spline From Knots |
| G, B, O | Periodic B-Spline From Knots |
| G, F, F | Fillet |
| G, F, C | Chamfer |
| G, T | Trim Edge |
| G, Q | Extend Edge |
| G, Z | Split Edge |
| G, X | External Projection |
| G, I | External Intersection |
| G, W | Carbon Copy |
| Ctrl+C | Copy Elements |
| Ctrl+X | Cut Elements |
| Ctrl+V | Paste Elements |
| Z, K | Select Associated Constraints |
| Z, P, R | Select Redundant Constraints |
| Z, P, C | Select Conflicting Constraints |
| Z, F | Select Under-Constrained Elements |
| Z, I | Toggle Internal Geometry |
| Z, S | Mirror |
| Z, C | Copy |
| Z, L | Clone |
| Z, M | Move |
| Z, A | Rectangular Array |
| Z, T | Offset |
| Z, P | Rotate / Polar Transform |
| Z, P, S | Scale |
| W | Move / Array Transform |
| Z, Z | Switch Virtual Space |

Geometry clipboard commands are unavailable; Ctrl+C/X/V still work normally in text
fields. Selection of redundant/conflicting constraints or under-constrained entities
requires diagnostics beyond the current Arael adapter. The current constraint list
edits one constraint at a time, so selecting all associated constraints (Z,K) is absent.

## Editing and contextual keys

| Keys | Behavior in this editor |
| --- | --- |
| Ctrl+A | Select all authored geometry (fixed origin/axes excluded); text fields retain Select All |
| Ctrl+Z | Local sketch undo |
| Ctrl+Y / Ctrl+Shift+Z | Local sketch redo |
| Delete | Existing delete-selection action, subject to wire/constraint validation |
| Escape | Cancel a sequence or gesture; return to Select; close shortcut reference |
| Enter / Escape in dimension value | Save / discard numeric edit |
| Shift+click | Add/remove selection operands |

FreeCAD's drawing-handler mode keys are context-specific, separate from command
accelerators. In particular, **M** cycles polyline segment modes and the contextual
Dimension modes in FreeCAD; our polyline currently consists of straight segments,
and our D command derives the dimension from the selection. Mode cycling, on-view
parameter entry, snapping/auto-constraint modifier behavior, B-spline weight/degree
controls and transformation-mode keys are not replicated. There is no unrelated
bare-M tool binding. Standard Tab focus navigation remains available.

Commands without a FreeCAD default accelerator (including many B-spline operations,
validation actions, grid controls, and creating/editing/closing sketches) have no
invented FreeCAD key binding. Use New sketch, Edit sketch and Apply/Cancel in the UI.
FreeCAD's general 3D-view/navigation and other workbench shortcuts are outside the
sketch editing command map. Browser-reserved shortcuts remain browser-controlled.

## Sources and maintenance

Reviewed against the pinned [FreeCAD 1.1.4 release](https://github.com/FreeCAD/FreeCAD/releases/tag/1.1.4)
and its command registrations, including conditional defaults and flyout aliases:

- [Constraints and dimensions](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/CommandConstraints.cpp)
- [Geometry creation](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/CommandCreateGeo.cpp)
- [Selection and editing tools](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/CommandSketcherTools.cpp)
- [Sketch commands and views](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/Command.cpp)
- [Construction toggle](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/CommandAlterGeometry.cpp)
- [Virtual space](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/CommandSketcherVirtualSpace.cpp)
- [Polyline handler](https://github.com/FreeCAD/FreeCAD/blob/1.1.4/src/Mod/Sketcher/Gui/DrawSketchHandlerLineSet.h)

`crates/cam-gui/src/sketch/freecad_shortcuts.rs` is the single factual command catalogue
used for dispatch, tooltips and the in-app reference. Flyouts sharing a command's
accelerator appear once; R has a separate binding because it adds an accelerator.
The selection-sensitive implementation lives in `shortcut_actions.rs`. Update this
document together with those files when adding a supported tool or changing the
version baseline. Tests cover dispatch ambiguity, missing-command isolation, focus,
contextual constraints, atomic undo, dimensions and fixed-reference selection.
