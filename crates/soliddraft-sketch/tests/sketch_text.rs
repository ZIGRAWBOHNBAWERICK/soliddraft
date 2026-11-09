//! Closed font contours, regeneration, persistence and validation.
use soliddraft_kernel_api::{BezierControls, Plane, Segment};
use soliddraft_sketch::{EntityKind, P2, Sketch, SketchText, TextFont, solve};

#[test]
fn glyph_contours_close_and_counters_remain_holes() {
    for font in [TextFont::InterRegular, TextFont::InterMedium] {
        for (content, holes) in [("o", 1), ("a", 1), ("e", 1), ("B", 2), ("oae", 3), ("Hi", 0)] {
            let text = SketchText { content: content.into(), font, ..Default::default() };
            let mut sketch = Sketch::new();
            let id = sketch.add(EntityKind::Text { text }, false);
            solve::solve(&mut sketch, None, true);
            let regions = sketch.regions().unwrap_or_else(|error| panic!("{font:?} {content}: {error}"));
            assert_eq!(regions.iter().map(|r| r.holes.len()).sum::<usize>(), holes, "{content}");
            let mut has_curve = false;
            for region in regions {
                assert!(region.area() > 0.0);
                region.to_profile(Plane::XY).check().unwrap();
                for boundary in std::iter::once(&region.outer).chain(&region.holes) {
                    assert!(boundary.entities.iter().all(|entity| *entity == id));
                    for (segment, next) in boundary.segments.iter().zip(boundary.segments.iter().cycle().skip(1)) {
                        assert_eq!(segment.end(), next.start(), "{content}: exact closed contour");
                        has_curve |= matches!(segment, Segment::Bezier { controls: BezierControls::Quadratic(_), .. });
                    }
                }
            }
            if holes > 0 {
                assert!(has_curve, "font curves retained");
            }
        }
    }
}

#[test]
fn edits_regenerate_profiles_and_survive_serialization() {
    let mut sketch = Sketch::new();
    let id = sketch.add(EntityKind::Text { text: SketchText { content: "o".into(), ..Default::default() } }, false);
    let original = sketch.regions().unwrap()[0].area();
    let EntityKind::Text { text } = &mut sketch.entity_mut(id).unwrap().kind else { unreachable!() };
    text.height *= 2.0;
    text.position = P2::new(20.0, 30.0);
    text.angle = 0.7;
    assert!((sketch.regions().unwrap()[0].area() - original * 4.0).abs() < 1e-7);
    let EntityKind::Text { text } = &mut sketch.entity_mut(id).unwrap().kind else { unreachable!() };
    text.font = TextFont::InterMedium;
    assert!((sketch.regions().unwrap()[0].area() - original * 4.0).abs() > 0.01);
    text_content(&mut sketch, id, "B");
    assert_eq!(sketch.regions().unwrap()[0].holes.len(), 2);
    let saved = serde_json::to_string(&sketch).unwrap();
    let mut reopened: Sketch = serde_json::from_str(&saved).unwrap();
    reopened.normalise();
    assert_eq!(reopened.regions().unwrap(), sketch.regions().unwrap());
    reopened.entity_mut(id).unwrap().construction = true;
    assert!(reopened.regions().is_err());
}

fn text_content(sketch: &mut Sketch, id: u32, content: &str) {
    let EntityKind::Text { text } = &mut sketch.entity_mut(id).unwrap().kind else { unreachable!() };
    text.content = content.into();
}

#[test]
fn invalid_content_and_settings_are_explicit_refusals() {
    for content in ["", " ", "line\nbreak", "\u{10ffff}"] {
        assert!(SketchText { content: content.into(), ..Default::default() }.outlines().is_err());
    }
    for height in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(SketchText { height, ..Default::default() }.outlines().is_err());
    }
    assert!(SketchText { content: "x".repeat(257), ..Default::default() }.outlines().is_err());
}

#[test]
fn glyphs_do_not_offer_point_on_snaps_and_baseline_constraints_remain_available() {
    use soliddraft_sketch::{
        ConstraintKind, PointRef, SolveStatus,
        inference::{self, InferenceOptions, SnapFilter},
    };
    let mut sketch = Sketch::new();
    let id = sketch.add(EntityKind::Text { text: SketchText::default() }, false);
    let at = sketch.entity(id).unwrap().polylines(16)[0][1];
    let (_, snaps) =
        inference::infer(&sketch, at, None, InferenceOptions::uniform(1.0, SnapFilter::On), |a, b| a.dist(b));
    assert!(snaps.is_empty());
    sketch.constrain(ConstraintKind::Coincident { a: PointRef::new(id, 0), b: PointRef::ORIGIN });
    solve::solve(&mut sketch, None, true);
    assert!(!matches!(sketch.status, SolveStatus::Failed(_)));
    let point = sketch.add(EntityKind::Point { p: at }, true);
    sketch.constrain(ConstraintKind::PointOn { point: PointRef::new(point, 0), entity: id });
    solve::solve(&mut sketch, None, true);
    assert!(matches!(sketch.status, SolveStatus::Failed(_)));
}

#[test]
fn trimming_text_is_an_atomic_refusal() {
    let mut sketch = Sketch::new();
    let id = sketch.add(EntityKind::Text { text: SketchText::default() }, false);
    let before = sketch.clone();
    let error = sketch.trim_preview(id, P2::new(2.0, 2.0), Default::default()).unwrap_err();
    assert!(error.to_string().contains("text"));
    assert_eq!(sketch, before);
}

#[test]
fn extrusion_limit_only_applies_to_drawn_text_with_disconnected_regions() {
    use soliddraft_sketch::text::DISCONNECTED_TEXT_EXTRUSION_SUPPORTED;
    let mut sketch = Sketch::new();
    let id = sketch.add(EntityKind::Text { text: SketchText { content: "i".into(), ..Default::default() } }, false);
    let count = sketch.regions().unwrap().len();
    assert_eq!(count, 2, "one letter can still contain disconnected regions");
    assert_eq!(sketch.text_extrusion_notice(count).is_some(), !DISCONNECTED_TEXT_EXTRUSION_SUPPORTED);
    assert!(sketch.text_extrusion_notice(1).is_none());
    sketch.entity_mut(id).unwrap().construction = true;
    assert!(sketch.text_extrusion_notice(count).is_none());
}
