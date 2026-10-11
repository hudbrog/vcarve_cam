use super::*;
use crate::{app::observe_control, ui_theme as theme};
use egui::{Color32, Pos2, Rect, Stroke};

pub(crate) struct View {
    pub scale: f64,
    center: Point,
    size: Option<egui::Vec2>,
    pub(super) auto_fit: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            scale: 0.,
            center: Point::new(0., 0.),
            size: None,
            auto_fit: true,
        }
    }
}
impl View {
    fn screen(&self, rect: Rect, p: Point) -> Pos2 {
        rect.center()
            + egui::vec2(
                ((p.x - self.center.x) * self.scale) as f32,
                (-(p.y - self.center.y) * self.scale) as f32,
            )
    }
    fn world(&self, rect: Rect, p: Pos2) -> Point {
        Point::new(
            self.center.x + (p.x - rect.center().x) as f64 / self.scale,
            self.center.y - (p.y - rect.center().y) as f64 / self.scale,
        )
    }
}

#[derive(Clone, Debug)]
pub struct Snap {
    pub point: Point,
    pub label: &'static str,
}

impl Editor {
    fn box_select(&mut self, canvas: Rect, start: Pos2, end: Pos2, additive: bool) {
        let bounds = Rect::from_two_pos(start, end);
        let crossing = end.x < start.x;
        let hits: Vec<_> = self
            .saved
            .model
            .entities
            .iter()
            .filter(|e| {
                let ps: Vec<_> = geometry_points(&e.geometry, 128)
                    .into_iter()
                    .map(|p| self.view.screen(canvas, p))
                    .collect();
                if crossing {
                    ps.iter().any(|p| bounds.contains(*p))
                        || ps
                            .windows(2)
                            .any(|p| segment_intersects(bounds, p[0], p[1]))
                } else {
                    ps.iter().all(|p| bounds.contains(*p))
                }
            })
            .map(|e| constraints::Operand::Entity(e.id.clone()))
            .collect();
        if !additive {
            self.pick_operand(None, false);
        }
        for operand in hits {
            if !self.saved.constraint_form.operands.contains(&operand) {
                self.pick_operand(Some(operand), true);
            }
        }
    }
    pub(super) fn locate_selected(&mut self) {
        let ps: Vec<_> = self
            .saved
            .model
            .entities
            .iter()
            .filter(|e| self.selected.contains(&e.id))
            .flat_map(|e| geometry_points(&e.geometry, 32))
            .collect();
        if ps.is_empty() {
            return;
        }
        self.view.center = Point::new(
            ps.iter().map(|p| p.x).sum::<f64>() / ps.len() as f64,
            ps.iter().map(|p| p.y).sum::<f64>() / ps.len() as f64,
        );
        self.view.auto_fit = false;
    }
    fn hit_operand(&self, p: Point, radius: f64) -> Option<constraints::Operand> {
        self.saved
            .model
            .entities
            .iter()
            .flat_map(|e| {
                [
                    PointRole::Start,
                    PointRole::End,
                    PointRole::Center,
                    PointRole::Position,
                ]
                .into_iter()
                .filter_map(move |role| {
                    e.geometry.point(role).map(|q| {
                        (
                            q.distance(p),
                            PointRef {
                                entity_id: e.id.clone(),
                                point: role,
                            },
                        )
                    })
                })
            })
            .filter(|(d, _)| *d <= radius)
            .min_by(|(a, _), (b, _)| a.total_cmp(b))
            .map(|(_, p)| constraints::Operand::Point(p))
            .or_else(|| self.hit(p, radius).map(constraints::Operand::Entity))
    }
    pub fn snap(&self, point: Point, radius: f64, exclude: &BTreeSet<String>) -> Option<Snap> {
        if !self.snapping {
            return None;
        }
        let mut best: Option<(f64, Snap)> = None;
        let mut consider = |p: Point, label| {
            let d = p.distance(point);
            if d <= radius && best.as_ref().is_none_or(|(distance, _)| d < *distance) {
                best = Some((d, Snap { point: p, label }));
            }
        };
        for e in &self.saved.model.entities {
            if exclude.contains(&e.id) {
                continue;
            }
            for role in [
                PointRole::Start,
                PointRole::End,
                PointRole::Center,
                PointRole::Position,
            ] {
                if let Some(p) = e.geometry.point(role) {
                    consider(
                        p,
                        match role {
                            PointRole::Start | PointRole::End => "Endpoint",
                            PointRole::Center => "Centre",
                            _ => "Point",
                        },
                    );
                }
            }
            if let Some((a, b)) = e.geometry.line() {
                consider(Point::new((a.x + b.x) / 2., (a.y + b.y) / 2.), "Midpoint");
            }
            if let SketchGeometry::Circle { center, radius_mm } = e.geometry {
                for (x, y) in [(1., 0.), (0., 1.), (-1., 0.), (0., -1.)] {
                    consider(
                        Point::new(center.x + x * radius_mm, center.y + y * radius_mm),
                        "Quadrant",
                    );
                }
            }
        }
        for p in &self.points {
            consider(*p, "Vertex");
        }
        consider(Point::new(0., 0.), "Origin");
        if best.is_some() {
            return best.map(|(_, snap)| snap);
        }
        if self.grid_snap && self.grid_mm.is_finite() && self.grid_mm > 0. {
            let g = self.grid_mm;
            let p = Point::new((point.x / g).round() * g, (point.y / g).round() * g);
            if p.distance(point) <= radius {
                return Some(Snap {
                    point: p,
                    label: "Grid",
                });
            }
        }
        None
    }
    pub fn hit(&self, point: Point, radius: f64) -> Option<String> {
        self.saved
            .model
            .entities
            .iter()
            .map(|e| (e, distance(&e.geometry, point)))
            .filter(|(_, d)| *d <= radius)
            .min_by(|(_, a), (_, b)| a.total_cmp(b))
            .map(|(e, _)| e.id.clone())
    }
    pub fn canvas(&mut self, ui: &mut egui::Ui) {
        let (rect, response) = ui.allocate_exact_size(
            ui.available_size().max(egui::vec2(1., 1.)),
            egui::Sense::click_and_drag(),
        );
        observe_control("Sketch canvas", rect);
        if self.view.scale == 0. || (self.view.auto_fit && self.view.size != Some(rect.size())) {
            self.fit_rect(rect);
        }
        self.view.size = Some(rect.size());
        let pointer = response
            .hover_pos()
            .or_else(|| response.interact_pointer_pos());
        if response.hovered() {
            let zoom = ui
                .input(|i| (i.smooth_scroll_delta.y as f64 * 0.002).exp() * i.zoom_delta() as f64);
            if (zoom - 1.).abs() > 1e-8 {
                self.view.auto_fit = false;
            }
            if let Some(p) = pointer {
                let before = self.view.world(rect, p);
                self.view.scale = (self.view.scale * zoom).clamp(0.005, 100_000.);
                let after = self.view.world(rect, p);
                self.view.center.x += before.x - after.x;
                self.view.center.y += before.y - after.y;
            }
        }
        if response.dragged_by(egui::PointerButton::Middle)
            || response.dragged_by(egui::PointerButton::Secondary)
        {
            self.view.auto_fit = false;
            let delta = ui.input(|i| i.pointer.delta());
            self.view.center.x -= delta.x as f64 / self.view.scale;
            self.view.center.y += delta.y as f64 / self.view.scale;
        }
        let world = pointer.map(|p| self.view.world(rect, p));
        let empty = BTreeSet::new();
        let snap = world.and_then(|p| {
            self.snap(
                p,
                10. / self.view.scale,
                if self.drag.is_some() {
                    &self.selected
                } else {
                    &empty
                },
            )
        });
        let target = snap.as_ref().map(|s| s.point).or(world);
        if self.tool == Tool::Select {
            if response.clicked_by(egui::PointerButton::Primary)
                && let Some(p) = world
            {
                if response.double_clicked() {
                    let hit = self.hit(p, 8. / self.view.scale);
                    self.select_wire(hit.as_deref(), ui.input(|i| i.modifiers.shift));
                } else {
                    self.pick_operand(
                        self.hit_operand(p, 8. / self.view.scale),
                        ui.input(|i| i.modifiers.shift),
                    );
                }
            }
            if response.drag_started_by(egui::PointerButton::Primary)
                && let Some(p) = ui.input(|i| i.pointer.press_origin())
            {
                let start = self.view.world(rect, p);
                if let Some(operand) = self.hit_operand(start, 8. / self.view.scale) {
                    if !self.saved.constraint_form.operands.contains(&operand) {
                        self.pick_operand(Some(operand), ui.input(|i| i.modifiers.shift));
                    }
                    let mut subjects = Vec::new();
                    for operand in &self.saved.constraint_form.operands {
                        match operand {
                            constraints::Operand::Point(p) => subjects.push(p.clone()),
                            constraints::Operand::Entity(id) => {
                                if let Ok(e) = self.saved.model.entity(id) {
                                    for role in [
                                        PointRole::Start,
                                        PointRole::End,
                                        PointRole::Center,
                                        PointRole::Position,
                                    ] {
                                        let p = PointRef {
                                            entity_id: id.clone(),
                                            point: role,
                                        };
                                        if e.geometry.point(role).is_some()
                                            && !subjects.contains(&p)
                                        {
                                            subjects.push(p);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Match the snapped destination to a snapped pickup, so rounded
                    // device pixels do not introduce a sub-grid translation.
                    let anchor = self
                        .snap(start, 10. / self.view.scale, &empty)
                        .map_or(start, |s| s.point);
                    self.start_drag(anchor, subjects);
                } else {
                    self.box_start = Some(p);
                }
            }
            if response.dragged_by(egui::PointerButton::Primary)
                && let Some(p) = target
            {
                self.drag_to(p);
            }
            if response.drag_stopped_by(egui::PointerButton::Primary) {
                if let Some(start) = self.box_start.take()
                    && let Some(end) = pointer
                {
                    self.box_select(rect, start, end, ui.input(|i| i.modifiers.shift));
                }
                if let Some(p) = target {
                    self.drag_to(p);
                }
                if let Some(drag) = &mut self.drag {
                    drag.released = true;
                }
            }
        } else if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = target
            && let Err(e) = self.click(p)
        {
            self.message = e;
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0., Color32::from_rgb(251, 253, 254));
        self.paint_grid(&painter, rect);
        let f = &self.saved.model.frame;
        let frame = Rect::from_two_pos(
            self.view.screen(rect, f.min),
            self.view.screen(
                rect,
                Point::new(f.min.x + f.width_mm, f.min.y + f.height_mm),
            ),
        );
        painter.rect_stroke(
            frame,
            0.,
            Stroke::new(1., theme::DIVIDER),
            egui::StrokeKind::Inside,
        );
        for e in &self.saved.model.entities {
            let selected = self
                .saved
                .constraint_form
                .operands
                .contains(&constraints::Operand::Entity(e.id.clone()));
            let color = if selected {
                theme::ACCENT
            } else if e.construction {
                Color32::from_rgb(81, 126, 180)
            } else {
                theme::TEXT
            };
            self.paint_geometry(
                &painter,
                rect,
                &e.geometry,
                Stroke::new(if selected { 3. } else { 1.7 }, color),
                e.construction,
            );
        }
        for p in &self.points {
            painter.circle_filled(self.view.screen(rect, *p), 3., theme::ACCENT);
        }
        {
            for operand in &self.saved.constraint_form.operands {
                if let constraints::Operand::Point(p) = operand
                    && let Ok(p) = self.saved.model.point(p)
                {
                    painter.circle_stroke(
                        self.view.screen(rect, p),
                        6.,
                        Stroke::new(2., theme::ACCENT),
                    );
                }
            }
            let mut occupied: Vec<Rect> = vec![];
            for (id, p, label, failed) in self.dimension_labels() {
                let color = if failed { theme::ERROR } else { theme::ACCENT };
                let galley = painter.layout_no_wrap(label, egui::FontId::proportional(12.), color);
                let pos = self.view.screen(rect, p) + egui::vec2(8., -14. - galley.size().y);
                let mut bounds = Rect::from_min_size(pos, galley.size());
                for _ in 0..occupied.len() {
                    if !occupied.iter().any(|r| r.intersects(bounds.expand(2.))) {
                        break;
                    }
                    bounds = bounds.translate(egui::vec2(0., -galley.size().y - 4.));
                }
                occupied.push(bounds);
                painter.rect_filled(bounds.expand(1.), 2., Color32::from_white_alpha(230));
                painter.galley(bounds.min, galley, color);
                observe_control(&format!("Dimension label {id}"), bounds);
                if self.tool == Tool::Select
                    && response.double_clicked()
                    && pointer.is_some_and(|p| bounds.expand(3.).contains(p))
                {
                    self.edit_constraint(&id);
                }
            }
        }
        if let Some(start) = self.box_start
            && let Some(end) = pointer
        {
            painter.rect_stroke(
                Rect::from_two_pos(start, end),
                0.,
                Stroke::new(1., theme::ACCENT),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(p) = target {
            self.paint_preview(&painter, rect, p);
        }
        if let Some(snap) = &snap
            && (self.tool != Tool::Select || self.drag.is_some())
        {
            let pos = self.view.screen(rect, snap.point);
            painter.rect_stroke(
                Rect::from_center_size(pos, egui::vec2(10., 10.)),
                0.,
                Stroke::new(1.5, theme::SUCCESS),
                egui::StrokeKind::Middle,
            );
            painter.text(
                pos + egui::vec2(10., -12.),
                egui::Align2::LEFT_BOTTOM,
                snap.label,
                egui::FontId::proportional(12.),
                theme::SUCCESS,
            );
        }
        if let Some(p) = target {
            painter.text(
                rect.right_bottom() - egui::vec2(14., 12.),
                egui::Align2::RIGHT_BOTTOM,
                format!("X {:.3}   Y {:.3} mm", p.x, p.y),
                egui::FontId::monospace(12.),
                theme::MUTED,
            );
        }
        painter.text(
            rect.left_bottom() + egui::vec2(12., -12.),
            egui::Align2::LEFT_BOTTOM,
            if rect.width() > 600. {
                "XY · mm · Y up   |   Wheel: zoom · Middle/right drag: pan"
            } else {
                "XY · mm · Y up"
            },
            egui::FontId::proportional(12.),
            theme::MUTED,
        );
    }
    fn fit_rect(&mut self, rect: Rect) {
        let mut points: Vec<_> = self
            .saved
            .model
            .entities
            .iter()
            .flat_map(|e| geometry_points(&e.geometry, 64))
            .collect();
        if points.is_empty() {
            points = vec![Point::new(0., 0.), Point::new(100., 100.)];
        }
        let min = Point::new(
            points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min),
            points.iter().map(|p| p.y).fold(f64::INFINITY, f64::min),
        );
        let max = Point::new(
            points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max),
            points.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max),
        );
        self.view.center = Point::new((min.x + max.x) / 2., (min.y + max.y) / 2.);
        self.view.scale = (rect.width() as f64 / (max.x - min.x).max(10.))
            .min(rect.height() as f64 / (max.y - min.y).max(10.))
            * 0.8;
    }
    fn paint_grid(&self, painter: &egui::Painter, rect: Rect) {
        let mut step = self.grid_mm.max(0.001);
        while step * self.view.scale < 16. {
            step *= 10.;
        }
        let a = self.view.world(rect, rect.left_bottom());
        let b = self.view.world(rect, rect.right_top());
        for vertical in [true, false] {
            let (min, max) = if vertical { (a.x, b.x) } else { (a.y, b.y) };
            for i in 0..=((max - min) / step).ceil().min(200.) as usize {
                let v = (min / step).ceil() * step + i as f64 * step;
                let (p, q) = if vertical {
                    (Point::new(v, a.y), Point::new(v, b.y))
                } else {
                    (Point::new(a.x, v), Point::new(b.x, v))
                };
                painter.line_segment(
                    [self.view.screen(rect, p), self.view.screen(rect, q)],
                    Stroke::new(1., Color32::from_rgb(231, 237, 241)),
                );
            }
        }
        for (p, q, color) in [
            (
                Point::new(a.x, 0.),
                Point::new(b.x, 0.),
                Color32::from_rgb(191, 119, 119),
            ),
            (
                Point::new(0., a.y),
                Point::new(0., b.y),
                Color32::from_rgb(110, 160, 121),
            ),
        ] {
            painter.line_segment(
                [self.view.screen(rect, p), self.view.screen(rect, q)],
                Stroke::new(1., color),
            );
        }
    }
    fn paint_geometry(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        g: &SketchGeometry,
        stroke: Stroke,
        dashed: bool,
    ) {
        let radius = g.circle().map_or(0., |(_, r)| r);
        let segments = ((radius * self.view.scale).sqrt() * 10.).clamp(32., 512.) as usize;
        let points: Vec<_> = geometry_points(g, segments)
            .iter()
            .map(|p| self.view.screen(rect, *p))
            .collect();
        if points.len() == 1 {
            painter.circle_filled(points[0], 3.5, stroke.color);
            return;
        }
        if dashed {
            painter.extend(egui::Shape::dashed_line(&points, stroke, 6., 4.));
        } else {
            painter.add(egui::Shape::line(points, stroke));
        }
        for role in [PointRole::Start, PointRole::End, PointRole::Center] {
            if let Some(p) = g.point(role) {
                painter.circle_filled(self.view.screen(rect, p), 2.5, stroke.color);
            }
        }
    }
    fn paint_preview(&self, painter: &egui::Painter, rect: Rect, p: Point) {
        let Some(&a) = self.points.last() else { return };
        let geometries = match self.tool {
            Tool::Circle => vec![SketchGeometry::Circle {
                center: a,
                radius_mm: a.distance(p),
            }],
            Tool::Rectangle => {
                let ps = [a, Point::new(p.x, a.y), p, Point::new(a.x, p.y), a];
                ps.windows(2)
                    .map(|s| SketchGeometry::Line {
                        start: s[0],
                        end: s[1],
                    })
                    .collect()
            }
            Tool::Arc if self.points.len() == 2 => arc_through(self.points[0], a, p)
                .map(|g| vec![g])
                .unwrap_or_default(),
            _ => vec![SketchGeometry::Line { start: a, end: p }],
        };
        for g in geometries {
            self.paint_geometry(painter, rect, &g, Stroke::new(1.5, theme::ACCENT), true);
        }
    }
    pub fn canvas_probe(&self) -> serde_json::Value {
        serde_json::json!({"center":[self.view.center.x,self.view.center.y],"scale":self.view.scale,"tool":format!("{:?}",self.tool),"selected":self.selected,"undo":self.undo.len(),"redo":self.redo.len(),"pending":self.pending(),"message":self.message,
            "show_constraints":self.show_constraints,"show_geometry":self.show_geometry,"shortcut":self.prefix_hint(),"solving":self.solving(),"solver_status":self.solver_status(),
            "solver":self.solver.outcome.as_ref().map(|r|serde_json::json!({"accepted":r.accepted,"dof":r.degrees_of_freedom,"issues":r.issues,"drag_error_mm":r.drag_error_mm}))})
    }
}

fn segment_intersects(r: Rect, a: Pos2, b: Pos2) -> bool {
    let mut low: f32 = 0.;
    let mut high: f32 = 1.;
    for (p, delta, min, max) in [
        (a.x, b.x - a.x, r.min.x, r.max.x),
        (a.y, b.y - a.y, r.min.y, r.max.y),
    ] {
        if delta.abs() < f32::EPSILON {
            if p < min || p > max {
                return false;
            }
        } else {
            let t = (min - p) / delta;
            let u = (max - p) / delta;
            low = low.max(t.min(u));
            high = high.min(t.max(u));
            if low > high {
                return false;
            }
        }
    }
    true
}

fn geometry_points(g: &SketchGeometry, n: usize) -> Vec<Point> {
    match *g {
        SketchGeometry::Point { position } => vec![position],
        SketchGeometry::Line { start, end } => vec![start, end],
        SketchGeometry::Circle { center, radius_mm } => {
            arc_points(center, radius_mm, 0., std::f64::consts::TAU, n)
        }
        SketchGeometry::Arc {
            center,
            radius_mm,
            start_angle_rad,
            sweep_angle_rad,
        } => arc_points(center, radius_mm, start_angle_rad, sweep_angle_rad, n),
    }
}
fn arc_points(c: Point, r: f64, a: f64, s: f64, n: usize) -> Vec<Point> {
    (0..=n)
        .map(|i| {
            let t = a + s * i as f64 / n as f64;
            Point::new(c.x + r * t.cos(), c.y + r * t.sin())
        })
        .collect()
}
fn distance(g: &SketchGeometry, p: Point) -> f64 {
    match *g {
        SketchGeometry::Point { position } => p.distance(position),
        SketchGeometry::Line { start: a, end: b } => {
            let x = b.x - a.x;
            let y = b.y - a.y;
            if x * x + y * y < 1e-20 {
                return p.distance(a);
            }
            let t = (((p.x - a.x) * x + (p.y - a.y) * y) / (x * x + y * y)).clamp(0., 1.);
            p.distance(Point::new(a.x + t * x, a.y + t * y))
        }
        SketchGeometry::Circle { center, radius_mm } => (p.distance(center) - radius_mm).abs(),
        SketchGeometry::Arc {
            center,
            radius_mm,
            start_angle_rad,
            sweep_angle_rad,
        } => {
            let a = (p.y - center.y).atan2(p.x - center.x);
            let delta = if sweep_angle_rad > 0. {
                (a - start_angle_rad).rem_euclid(std::f64::consts::TAU)
            } else {
                (start_angle_rad - a).rem_euclid(std::f64::consts::TAU)
            };
            if delta <= sweep_angle_rad.abs() {
                (p.distance(center) - radius_mm).abs()
            } else {
                p.distance(g.point(PointRole::Start).unwrap())
                    .min(p.distance(g.point(PointRole::End).unwrap()))
            }
        }
    }
}
