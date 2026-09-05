use cam_core::{
    geometry::{BoundaryQuery, Point, PointLocation},
    job::{Job, ToolGeometry},
    model::{EndmillSpec, VBitSpec},
    svg::{ImportOptions, NormalizedGeometry, Placement, import_svg},
};

fn svg(body: &str) -> String {
    format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="100mm" height="100mm" viewBox="0 0 100 100">{body}</svg>"#
    )
}
fn read(body: &str) -> NormalizedGeometry {
    import_svg(&svg(body), &ImportOptions::default(), None).unwrap()
}
fn near(a: f64, b: f64, e: f64) {
    assert!((a - b).abs() <= e, "{a} != {b} +/- {e}");
}
fn area(body: &str) -> f64 {
    read(body).selected.area_mm2()
}
fn bounds(body: &str) -> [f64; 4] {
    let b = read(body).bounds.unwrap();
    [b.min.x, b.min.y, b.max.x, b.max.y]
}
const BOX: &str = r#"<rect id="box" x="10" y="10" width="20" height="10"/>"#;

#[test]
fn physical_units_and_css_pixels_have_explicit_dimensions() {
    for (w, h) in [
        ("25.4mm", "12.7mm"),
        ("2.54cm", "1.27cm"),
        ("1in", "0.5in"),
        ("96px", "48px"),
        ("72pt", "36pt"),
        ("6pc", "3pc"),
        ("96", "48"),
    ] {
        let raw = format!(
            r#"<svg width="{w}" height="{h}" viewBox="0 0 96 48"><path id="box" d="M0 0H96V48H0Z"/></svg>"#
        );
        let g = import_svg(&raw, &ImportOptions::default(), None).unwrap();
        near(g.page_width_mm, 25.4, 1e-12);
        near(g.page_height_mm, 12.7, 1e-12);
        near(g.selected.area_mm2(), 25.4 * 12.7, 1e-9);
    }
    let g = import_svg(
        r#"<svg width="25.4mm" height="25.4mm"><rect width="12.7mm" height="36pt"/></svg>"#,
        &ImportOptions::default(),
        None,
    )
    .unwrap();
    near(g.selected.area_mm2(), 12.7 * 12.7, 1e-9);
}

#[test]
fn viewbox_origin_alignment_and_y_axis_are_resolved() {
    let raw = r#"<svg width="40mm" height="20mm" viewBox="10 20 10 10"><rect x="10" y="20" width="10" height="10"/></svg>"#;
    let g = import_svg(raw, &ImportOptions::default(), None).unwrap();
    let b = g.bounds.unwrap();
    assert_eq!([b.min.x, b.min.y, b.max.x, b.max.y], [10., 0., 30., 20.]);
    let g = import_svg(
        &raw.replace("viewBox=", "preserveAspectRatio=\"none\" viewBox="),
        &ImportOptions::default(),
        None,
    )
    .unwrap();
    near(g.selected.area_mm2(), 800., 1e-9);
    let g = import_svg(
        &raw.replace("viewBox=", "preserveAspectRatio=\"xMaxYMin meet\" viewBox="),
        &ImportOptions::default(),
        None,
    )
    .unwrap();
    near(g.bounds.unwrap().min.x, 20., 1e-9);
    assert_eq!(bounds(BOX), [10., 80., 30., 90.]);
}

#[test]
fn nested_transforms_apply_in_svg_order_and_preserve_ids() {
    let body = r#"<g transform="translate(10,20) scale(2)"><g transform="rotate(90,5,5)"><rect id="box" width="5" height="10"/></g></g>"#;
    assert_eq!(bounds(body), [10., 70., 30., 80.]);
    let g = read(body);
    assert_eq!(g.sources[0].id, "box::0");
    near(
        area(
            r#"<rect id="r" x="20" y="20" width="10" height="10" transform="matrix(1 0.2 -0.1 1 0 0)"/>"#,
        ),
        102.,
        1e-8,
    );
    near(
        area(r#"<rect x="20" y="20" width="10" height="10" transform="skewX(30) skewY(10)"/>"#),
        100.,
        0.002,
    );
}

#[test]
fn placement_changes_origin_rotation_and_scale_without_changing_source_selection() {
    let options = ImportOptions {
        placement: Placement {
            origin_mm: Point::new(10., 80.),
            scale: 2.,
            rotation_deg: 90.,
        },
        ..Default::default()
    };
    let g = import_svg(&svg(BOX), &options, Some(&["box::0".into()])).unwrap();
    let b = g.bounds.unwrap();
    assert_eq!([b.min.x, b.min.y, b.max.x, b.max.y], [-20., 0., 0., 40.]);
    near(g.selected.area_mm2(), 800., 1e-9);
    assert_eq!(g.selected_region_ids, ["box::0"]);
}

#[test]
fn compound_fill_rules_and_reversed_winding_preserve_holes() {
    let same = "M0 0H20V20H0Z M5 5H15V15H5Z";
    let opposite = "M0 0H20V20H0Z M5 5V15H15V5Z";
    near(
        area(&format!(r#"<path fill-rule="evenodd" d="{same}"/>"#)),
        300.,
        1e-9,
    );
    near(
        area(&format!(r#"<path fill-rule="nonzero" d="{same}"/>"#)),
        400.,
        1e-9,
    );
    near(
        area(&format!(r#"<path fill-rule="nonzero" d="{opposite}"/>"#)),
        300.,
        1e-9,
    );
    let reversed = "M0 0V20H20V0Z M5 5H15V15H5Z";
    let g = read(&format!(r#"<path fill-rule="nonzero" d="{reversed}"/>"#));
    near(g.selected.area_mm2(), 300., 1e-9);
    assert_eq!(g.selected.hole_count(), 1);
    assert_eq!(
        BoundaryQuery::new(&g.selected)
            .sample(Point::new(10., 90.))
            .unwrap()
            .location,
        PointLocation::Outside
    );
}

#[test]
fn intersecting_subpaths_and_bowties_resolve_before_voronoi_input() {
    let paths = "M0 0H10V10H0Z M5 0H15V10H5Z";
    near(
        area(&format!(r#"<path fill-rule="nonzero" d="{paths}"/>"#)),
        150.,
        1e-9,
    );
    near(
        area(&format!(r#"<path fill-rule="evenodd" d="{paths}"/>"#)),
        100.,
        1e-9,
    );
    let g = read(r#"<path d="M0 0L10 10L0 10L10 0Z"/>"#);
    near(g.selected.area_mm2(), 50., 1e-9);
    assert_eq!(g.selected.component_count(), 2);
}

#[test]
fn selected_shape_unions_keep_all_contributing_source_ids() {
    let raw = svg(
        r#"<rect id="a" width="10" height="10"/><rect id="b" x="5" width="10" height="10"/><rect id="c" x="30" width="10" height="10"/>"#,
    );
    let g = import_svg(&raw, &ImportOptions::default(), None).unwrap();
    near(g.selected.area_mm2(), 250., 1e-9);
    assert_eq!(g.components.len(), 2);
    assert_eq!(g.components[0].source_ids, ["a", "b"]);
    let selected = import_svg(&raw, &ImportOptions::default(), Some(&["b::0".into()])).unwrap();
    near(selected.selected.area_mm2(), 100., 1e-9);
    assert_eq!(selected.components[0].source_ids, ["b"]);
    let none = import_svg(&raw, &ImportOptions::default(), Some(&[])).unwrap();
    assert_eq!(none.selected.area_mm2(), 0.);
    assert!(none.bounds.is_none());
    for ids in [
        vec!["missing::0".into()],
        vec!["a::0".into(), "a::0".into()],
    ] {
        assert_eq!(
            import_svg(&raw, &ImportOptions::default(), Some(&ids))
                .unwrap_err()
                .code,
            "SVG_SELECTION"
        );
    }
}

#[test]
fn nested_islands_have_stable_component_ids_and_source_labels() {
    let body = r#"<path xmlns:inkscape="http://www.inkscape.org/namespaces/inkscape" inkscape:label="nested artwork" id="nested" fill-rule="evenodd" d="M0 0H30V30H0Z M5 5H25V25H5Z M10 10H20V20H10Z"/>"#;
    let g = read(body);
    assert_eq!(g.sources.len(), 2);
    assert_eq!(g.sources[0].label.as_deref(), Some("nested artwork"));
    assert_eq!(g.selected.component_count(), 2);
    assert_eq!(g.selected.hole_count(), 1);
    near(g.selected.area_mm2(), 600., 1e-9);
    let island = import_svg(
        &svg(body),
        &ImportOptions::default(),
        Some(&["nested::1".into()]),
    )
    .unwrap();
    near(island.selected.area_mm2(), 100., 1e-9);
}

#[test]
fn inherited_styles_visibility_and_inline_precedence_are_resolved() {
    let body = r##"<g fill="none" visibility="hidden"><g color="#112233" style="fill:currentColor"><rect id="yes" visibility="visible" width="10" height="10"/><rect id="no" x="20" width="10" height="10"/></g></g><g display="none"><text>skip</text></g><rect fill="red" style="fill:none" width="20" height="20"/><rect opacity="0" x="30" width="20" height="20"/>"##;
    let g = read(body);
    assert_eq!(g.sources.len(), 1);
    assert_eq!(g.sources[0].source_id, "yes");
    near(g.selected.area_mm2(), 100., 1e-9);
    assert!(g.diagnostics.iter().any(|d| d.code == "SVG_HIDDEN"));
    let g = read(r#"<g opacity="0.5"><rect width="10" height="10"/></g>"#);
    assert!(g.diagnostics.iter().any(|d| d.code == "SVG_OPACITY"));
}

#[test]
fn basic_shapes_and_transformed_ellipse_have_bounded_dimensions() {
    let g = read(r#"<circle cx="20" cy="20" r="10"/>"#);
    near(g.selected.area_mm2(), std::f64::consts::PI * 100., 0.03);
    let query = BoundaryQuery::new(&g.selected);
    for i in 0..360 {
        let a = (i as f64).to_radians();
        let d = query
            .sample(Point::new(20. + 10. * a.cos(), 80. + 10. * a.sin()))
            .unwrap()
            .distance_mm;
        assert!(d <= 0.0004, "{d}");
    }
    near(
        area(r#"<rect x="5" y="5" width="20" height="10" rx="2"/>"#),
        200. - 16. + 4. * std::f64::consts::PI,
        0.006,
    );
    near(
        area(r#"<polygon points="0,0 10,0 10,10 0,10"/>"#),
        100.,
        1e-9,
    );
    near(
        area(r#"<ellipse cx="20" cy="20" rx="5" ry="10" transform="translate(5) scale(2,1)"/>"#),
        100. * std::f64::consts::PI,
        0.03,
    );
}

#[test]
fn arcs_support_relative_large_sweep_rotation_and_radius_correction() {
    for d in [
        "M10 20 A10 10 0 0 1 30 20 A10 10 0 0 1 10 20 Z",
        "M10 20 a10 10 0 0 0 20 0 a10 10 0 0 0 -20 0z",
        "M10 20 A1 1 45 1 1 30 20 A1 1 45 1 1 10 20 Z",
    ] {
        near(
            area(&format!(r#"<path d="{d}"/>"#)),
            100. * std::f64::consts::PI,
            0.03,
        );
    }
    near(
        area(r#"<path d="M10 10A0 5 0 0 0 20 10L20 20L10 20Z"/>"#),
        100.,
        1e-9,
    );
    let large = area(r#"<path d="M20 10A10 10 0 1 1 30 20L20 10Z"/>"#);
    near(large, 75. * std::f64::consts::PI + 50., 0.03);
}

#[test]
fn smooth_bezier_commands_match_explicit_controls() {
    let a = read(
        r#"<path d="M10 20 C10 10 20 10 20 20 S30 30 30 20 Q35 10 40 20 T50 20 L50 40H10Z"/>"#,
    );
    let b = read(
        r#"<path d="M10 20 C10 10 20 10 20 20 C20 30 30 30 30 20 Q35 10 40 20 Q45 30 50 20 L50 40H10Z"/>"#,
    );
    near(a.selected.area_mm2(), b.selected.area_mm2(), 1e-10);
    assert_eq!(
        serde_json::to_value(a.selected).unwrap(),
        serde_json::to_value(b.selected).unwrap()
    );
    near(area(r#"<path d="m10 10 10 0 0 10 -10 0z"/>"#), 100., 1e-9);
    near(area(r#"<path d="M10 10H20V20H10L10 10"/>"#), 100., 1e-9);
}

#[test]
fn unsupported_visible_content_has_source_diagnostics() {
    for (body, code) in [
        (r#"<text id="bad">Hello</text>"#, "SVG_TEXT"),
        (
            r#"<rect id="bad" width="10" height="10" stroke="red"/>"#,
            "SVG_STROKE",
        ),
        (r#"<path id="bad" d="M0 0L10 0L10 10"/>"#, "SVG_OPEN_PATH"),
        (r##"<use id="bad" href="#object"/>"##, "SVG_REFERENCE"),
        (
            r#"<image id="bad" href="https://example.invalid/a.png"/>"#,
            "SVG_REFERENCE",
        ),
        (
            r#"<rect id="bad" width="10" height="10" fill="url(#paint)"/>"#,
            "SVG_PAINT",
        ),
        (
            r#"<g id="bad" clip-path="url(#clip)"><rect width="10" height="10"/></g>"#,
            "SVG_RENDERING_FEATURE",
        ),
        (
            r#"<rect id="bad" width="10" height="10" style="filter:blur(2px)"/>"#,
            "SVG_RENDERING_FEATURE",
        ),
        (
            r#"<rect id="bad" width="10" height="10" style="transform:scale(2)"/>"#,
            "SVG_STYLE_UNSUPPORTED",
        ),
        (
            r#"<svg id="bad" width="10" height="10"/>"#,
            "SVG_UNSUPPORTED_ELEMENT",
        ),
    ] {
        let d = import_svg(&svg(body), &ImportOptions::default(), None).unwrap_err();
        assert_eq!(d.code, code, "{body}: {d}");
        assert_eq!(d.source_id.as_deref(), Some("bad"));
    }
}

#[test]
fn xml_stylesheets_units_and_ambiguous_dimensions_are_rejected() {
    for (raw,code) in [
        ("<!DOCTYPE svg [<!ENTITY x 'bad'>]><svg>&x;</svg>".into(),"SVG_XML"),
        (svg("<style>rect {display:none}</style><rect width='10' height='10'/>"),"SVG_STYLESHEET"),
        ("<?xml-stylesheet href='foo.css'?><svg width='10mm' height='10mm'/>".into(),"SVG_STYLESHEET"),
        ("<svg viewBox='0 0 10 10'/>".into(),"SVG_PAGE_SIZE"),
        ("<svg width='100%' height='100%'/>".into(),"SVG_LENGTH_UNIT"),
        (svg("<rect width='2em' height='10'/>"),"SVG_LENGTH_UNIT"),
        (svg("<rect id='x' width='10' height='10'/><rect id='x' width='10' height='10'/>"),"SVG_DUPLICATE_ID"),
        (svg("<rect x='95' width='10' height='10'/>"),"SVG_VIEWPORT_CLIPPING"),
        (svg("<rect width='10' height='10' transform='scale(0)'/>"),"SVG_TRANSFORM"),
        ("<svg width='10mm' height='10mm' viewBox='0 0 10 20' preserveAspectRatio='xMidYMid slice'/>".into(),"SVG_ASPECT_RATIO"),
    ] {assert_eq!(import_svg(&raw,&ImportOptions::default(),None).unwrap_err().code,code,"{raw}");}
}

#[test]
fn actual_inkscape_exports_preserve_compound_letters_and_overlapping_sources() {
    let mut reports = vec![];
    for raw in [
        include_str!("../../../fixtures/m2/inkscape-export.svg"),
        include_str!("../../../fixtures/m2/inkscape-native.svg"),
    ] {
        let g = import_svg(raw, &ImportOptions::default(), None).unwrap();
        near(g.page_width_mm, 100., 1e-10);
        near(g.page_height_mm, 60., 1e-10);
        let o = g
            .sources
            .iter()
            .find(|s| s.source_id == "letter-o")
            .unwrap();
        near(o.geometry.area_mm2(), 300., 1e-9);
        assert_eq!(o.geometry.hole_count(), 1);
        let b = g
            .sources
            .iter()
            .find(|s| s.source_id == "letter-b")
            .unwrap();
        assert_eq!(b.geometry.hole_count(), 2);
        assert!(
            g.components
                .iter()
                .any(|c| c.source_ids == ["overlap-a", "overlap-b"])
        );
        for line in include_str!("../../../fixtures/m2/inkscape-bounds.csv").lines() {
            let fields: Vec<_> = line.split(',').collect();
            if let Some(source) = g.sources.iter().find(|s| s.source_id == fields[0]) {
                let [x, y, w, h]: [f64; 4] = fields[1..]
                    .iter()
                    .map(|s| s.parse::<f64>().unwrap() * 25.4 / 96.)
                    .collect::<Vec<_>>()
                    .try_into()
                    .unwrap();
                let bounds = cam_core::svg::Bounds::of(&source.geometry).unwrap();
                for (a, b) in [
                    (bounds.min.x, x),
                    (bounds.min.y, 60. - y - h),
                    (bounds.max.x, x + w),
                    (bounds.max.y, 60. - y),
                ] {
                    near(a, b, 0.001);
                }
            }
        }
        reports.push(g);
    }
    near(
        reports[0].selected.area_mm2(),
        reports[1].selected.area_mm2(),
        1e-9,
    );
    assert_eq!(
        reports[0].selected_region_ids,
        reports[1].selected_region_ids
    );
    assert_eq!(
        import_svg(
            include_str!("../../../fixtures/m2/inkscape-source.svg"),
            &ImportOptions::default(),
            None
        )
        .unwrap_err()
        .code,
        "SVG_TEXT"
    );
}

#[test]
fn curve_and_input_limits_fail_explicitly() {
    let fine = ImportOptions {
        geometry_tolerance_mm: 1e-9,
        ..Default::default()
    };
    assert_eq!(
        import_svg(&svg(r#"<circle cx="20" cy="20" r="10"/>"#), &fine, None)
            .unwrap_err()
            .code,
        "SVG_CURVE_LIMIT"
    );
    assert_eq!(
        import_svg(&" ".repeat(2_000_001), &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "SVG_RESOURCE_LIMIT"
    );
    let nested = svg(&format!("{}{}{}", "<g>".repeat(66), BOX, "</g>".repeat(66)));
    assert_eq!(
        import_svg(&nested, &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "SVG_RESOURCE_LIMIT"
    );
    let collapsed = svg(r#"<path d="M10 10H10.000001V20H10Z"/>"#);
    assert_eq!(
        import_svg(&collapsed, &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "QUANTIZATION_COLLAPSE"
    );
}

#[test]
fn jobs_embed_source_and_round_trip_incomplete_settings_and_selection() {
    let raw = include_str!("../../../fixtures/m2/inkscape-export.svg");
    let mut job = Job::from_svg("coupon.svg".into(), raw.into(), ImportOptions::default()).unwrap();
    assert!(
        job.tools
            .iter()
            .all(|t| t.geometry.is_none() && t.spindle_rpm.is_none())
    );
    assert!(job.operation.max_depth_mm.is_none());
    assert!(job.machine_profile.is_none());
    job.selected_region_ids = vec!["letter-o::0".into()];
    job.import.placement.scale = 2.;
    job.name = "My carving".into();
    let saved = job.to_json().unwrap();
    let loaded = Job::from_json(&saved).unwrap();
    assert_eq!(loaded.source.svg, raw);
    assert_eq!(loaded.to_json().unwrap(), saved);
    let inspection = loaded.inspect().unwrap();
    near(inspection.geometry.selected.area_mm2(), 1200., 1e-8);
    assert!(inspection.planning_available);
    assert!(
        inspection
            .missing_machining_fields
            .contains(&"operation.max_depth_mm".into())
    );
}

#[test]
fn job_edits_rebuild_geometry_and_reject_stale_selection() {
    let mut job = Job::from_svg("box.svg".into(), svg(BOX), ImportOptions::default()).unwrap();
    let before = job.inspect().unwrap();
    near(before.geometry.selected.area_mm2(), 200., 1e-9);
    job.source.svg = svg(&BOX.replace("width=\"20\"", "width=\"30\""));
    near(
        job.inspect().unwrap().geometry.selected.area_mm2(),
        300.,
        1e-9,
    );
    job.source.svg = svg(&BOX.replace("id=\"box\"", "id=\"changed\""));
    assert_eq!(job.inspect().unwrap_err().code, "SVG_SELECTION");
}

#[test]
fn jobs_reject_future_schema_unknown_fields_and_invalid_partial_settings() {
    let job = Job::from_svg("box.svg".into(), svg(BOX), ImportOptions::default()).unwrap();
    let mut value = serde_json::to_value(&job).unwrap();
    value["schema_version"] = serde_json::json!(3);
    assert_eq!(
        Job::from_json(&value.to_string()).unwrap_err().code,
        "JOB_SCHEMA_VERSION"
    );
    value["schema_version"] = serde_json::json!(1);
    value["cached_geometry"] = serde_json::json!({});
    assert_eq!(
        Job::from_json(&value.to_string()).unwrap_err().code,
        "JOB_JSON"
    );
    let mut invalid = job.clone();
    invalid.tools[0].cutting_feed_mm_min = Some(-2.);
    assert_eq!(invalid.to_json().unwrap_err().code, "JOB_PARAMETER");
    invalid = job.clone();
    invalid.operation.vbit_id = "missing".into();
    assert_eq!(invalid.inspect().unwrap_err().code, "JOB_OPERATION");
    invalid = job;
    invalid.stock.thickness_mm = Some(2.);
    invalid.operation.max_depth_mm = Some(3.);
    assert_eq!(invalid.inspect().unwrap_err().code, "JOB_STOCK_DEPTH");
}

#[test]
fn tool_settings_remain_editable_but_completed_dimensions_obey_m1_contracts() {
    let mut job = Job::from_svg("box.svg".into(), svg(BOX), ImportOptions::default()).unwrap();
    job.tools[0].geometry = Some(ToolGeometry::Endmill(EndmillSpec {
        diameter_mm: 4.,
        cutting_length_mm: 10.,
        plunge_capable: false,
    }));
    job.tools[1].geometry = Some(ToolGeometry::Vbit(VBitSpec {
        included_angle_deg: 90.,
        tip_diameter_mm: 1.,
        max_cutting_diameter_mm: 12.,
        cutting_height_mm: 4.,
    }));
    job.operation.max_depth_mm = Some(2.);
    assert!(job.inspect().is_ok());
    job.operation.max_depth_mm = Some(5.);
    assert_eq!(job.inspect().unwrap_err().code, "VBIT_CUTTING_HEIGHT");
}

#[test]
fn component_selection_survives_rotation_of_disconnected_compound_paths() {
    let raw = svg(r#"<path id="parts" d="M10 10h5v10h-5z M30 10h10v10h-10z"/>"#);
    for angle in [0., 27., 90., 180., 270.] {
        let options = ImportOptions {
            placement: Placement {
                rotation_deg: angle,
                ..Default::default()
            },
            ..Default::default()
        };
        let g = import_svg(&raw, &options, Some(&["parts::0".into()])).unwrap();
        near(g.selected.area_mm2(), 50., 0.002);
    }
}

#[test]
fn distinct_sources_cannot_gain_a_contact_silently_when_snapped() {
    let raw = svg(
        r#"<rect id="a" x="10" y="10" width="10" height="10"/><rect id="b" x="20.000001" y="10" width="10" height="10"/>"#,
    );
    assert_eq!(
        import_svg(&raw, &ImportOptions::default(), None)
            .unwrap_err()
            .code,
        "QUANTIZATION_TOPOLOGY"
    );
    let fine = ImportOptions {
        ticks_per_mm: Some(1_000_000.),
        ..Default::default()
    };
    let g = import_svg(&raw, &fine, None).unwrap();
    assert_eq!(g.selected.component_count(), 2);
    near(g.selected.area_mm2(), 200., 1e-7);
}

#[test]
fn transparent_paint_and_important_styles_cannot_create_unintended_regions() {
    let raw = svg(
        r#"<rect id="yes" width="10" height="10"/><rect id="clear" x="20" width="10" height="10" fill="transparent"/><rect id="important" x="40" width="10" height="10" style="fill:none !important;fill:black"/>"#,
    );
    let g = import_svg(&raw, &ImportOptions::default(), None).unwrap();
    assert_eq!(g.sources.len(), 1);
    assert_eq!(g.sources[0].source_id, "yes");
    for body in [
        r#"<rect width="10" height="10"><animate attributeName="width" values="10;100" dur="1s"/></rect>"#,
        r#"<rect width="10" height="10" onload="change()"/>"#,
    ] {
        assert_eq!(
            import_svg(&svg(body), &ImportOptions::default(), None)
                .unwrap_err()
                .code,
            "SVG_DYNAMIC_CONTENT"
        );
    }
}

#[test]
fn curve_error_is_bounded_after_shear_nonuniform_scale_and_job_placement() {
    let raw =
        svg(r#"<path transform="matrix(4 0.5 -0.3 1 20 20)" d="M5 20C5 8 15 8 15 20L15 30H5Z"/>"#);
    let placement = Placement {
        origin_mm: Point::new(10., 20.),
        scale: 3.,
        rotation_deg: 27.,
    };
    let options = ImportOptions {
        placement: placement.clone(),
        ..Default::default()
    };
    let g = import_svg(&raw, &options, None).unwrap();
    let q = BoundaryQuery::new(&g.selected);
    let allowance = g.flattening_bound_mm + g.source_snap_bound_mm + g.grid.snap_bound_mm();
    let (sn, cs) = placement.rotation_deg.to_radians().sin_cos();
    for i in 0..=1000 {
        let t = i as f64 / 1000.;
        let u = 1. - t;
        let x = 5. * u.powi(3) + 15. * u * u * t + 45. * u * t * t + 15. * t.powi(3);
        let y = 20. * u.powi(3) + 24. * u * u * t + 24. * u * t * t + 20. * t.powi(3);
        let px = 4. * x - 0.3 * y + 20. - 10.;
        let py = 100. - (0.5 * x + y + 20.) - 20.;
        let p = Point::new(3. * (cs * px - sn * py), 3. * (sn * px + cs * py));
        let distance = q.sample(p).unwrap().distance_mm;
        assert!(
            distance <= allowance + 1e-10,
            "distance {distance} exceeds {allowance}"
        );
    }
    assert_eq!(
        import_svg(
            &svg(r#"<rect xmlns="" width="10" height="10"/>"#),
            &ImportOptions::default(),
            None
        )
        .unwrap_err()
        .code,
        "SVG_NAMESPACE"
    );
}
