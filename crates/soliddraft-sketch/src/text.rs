//! Retained sketch text and deterministic outlines from bundled SIL OFL fonts.

use crate::{Entity, EntityKind, P2, SplineMode};
use serde::{Deserialize, Serialize};
use ttf_parser::{Face, OutlineBuilder};

/// Enable once the pinned kernel can union disconnected polynomial glyph solids.
pub const DISCONNECTED_TEXT_EXTRUSION_SUPPORTED: bool = false;

impl crate::Sketch {
    /// Release limit shared by extrusion evaluation, previews and preflight.
    pub fn text_extrusion_notice(&self, region_count: usize) -> Option<&'static str> {
        (!DISCONNECTED_TEXT_EXTRUSION_SUPPORTED
            && region_count > 1
            && self.drawn().any(|entity| !entity.construction && matches!(entity.kind, EntityKind::Text { .. })))
        .then_some("Text extrusion currently supports one connected region at a time. Use one connected glyph per sketch; use separate parts for disconnected letters.")
    }
}

/// Bundled fonts; documents never depend on installed fonts or network access.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextFont {
    /// Inter Regular, SIL Open Font License 1.1.
    #[default]
    InterRegular,
    /// Inter Medium, SIL Open Font License 1.1.
    InterMedium,
}

impl TextFont {
    /// Font name shown in the editor.
    pub fn label(self) -> &'static str {
        match self {
            Self::InterRegular => "Inter Regular",
            Self::InterMedium => "Inter Medium",
        }
    }

    fn face(self) -> Face<'static> {
        let data: &[u8] = match self {
            Self::InterRegular => include_bytes!("../fonts/Inter-Regular.ttf"),
            Self::InterMedium => include_bytes!("../fonts/Inter-Medium.ttf"),
        };
        Face::parse(data, 0).expect("bundled Inter font")
    }
}

/// Editable single-line text. Height is the font's capital height in mm.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SketchText {
    /// Literal content, up to 256 characters; unsupported glyphs are refused.
    pub content: String,
    /// Embedded font selection.
    pub font: TextFont,
    /// Baseline origin in sketch coordinates.
    pub position: P2,
    /// Capital height in millimetres.
    pub height: f64,
    /// Baseline rotation in radians.
    #[serde(default)]
    pub angle: f64,
    /// Reflection across the baseline, retained by mirror operations.
    #[serde(default)]
    pub reflected: bool,
}

impl Default for SketchText {
    fn default() -> Self {
        Self {
            content: "Text".into(),
            font: TextFont::default(),
            position: P2::ZERO,
            height: 10.0,
            angle: 0.0,
            reflected: false,
        }
    }
}

impl SketchText {
    /// Applies a similarity transform to the entire text frame, including height.
    pub fn mapped(&self, map: impl Fn(P2) -> P2) -> Self {
        let mut result = self.clone();
        result.position = map(self.position);
        let x = map(self.position + P2::from_angle(self.angle).scale(self.height)) - result.position;
        let y = map(self.position + P2::from_angle(self.angle).perp().scale(self.height)) - result.position;
        result.height = x.len();
        result.angle = x.angle();
        result.reflected ^= x.cross(y) < 0.0;
        result
    }

    /// Closed glyph contours, preserving exact quadratic and cubic control points.
    pub fn outlines(&self) -> Result<Vec<Vec<EntityKind>>, String> {
        if !self.height.is_finite()
            || self.height < 0.01
            || self.height > 10000.0
            || !self.position.u.is_finite()
            || !self.position.v.is_finite()
            || !self.angle.is_finite()
        {
            return Err("Text needs a finite position, angle and height from 0.01 to 10000 mm".into());
        }
        if self.content.is_empty() || self.content.chars().count() > 256 || self.content.chars().any(char::is_control) {
            return Err("Enter one line of text, from 1 to 256 characters".into());
        }
        let face = self.font.face();
        let scale = self.height / f64::from(face.capital_height().unwrap_or(face.units_per_em() as i16));
        let mut builder = Builder {
            text: self,
            scale,
            advance: 0.0,
            contours: Vec::new(),
            current: Vec::new(),
            start: P2::ZERO,
            last: P2::ZERO,
        };
        for ch in self.content.chars() {
            let glyph = face.glyph_index(ch).ok_or_else(|| format!("{} has no glyph for {ch:?}", self.font.label()))?;
            let before = builder.contours.len();
            face.outline_glyph(glyph, &mut builder);
            if before == builder.contours.len() && ch != ' ' {
                return Err(format!("No usable outline for {ch:?}"));
            }
            builder.advance += f64::from(face.glyph_hor_advance(glyph).unwrap_or(0));
        }
        if builder.contours.is_empty() {
            return Err("Text must contain a visible glyph".into());
        }
        Ok(builder.contours)
    }

    /// Separate display paths; contour boundaries never create connecting strokes.
    pub fn polylines(&self, segments: usize) -> Vec<Vec<P2>> {
        self.outlines()
            .unwrap_or_default()
            .into_iter()
            .map(|contour| {
                let mut points = Vec::new();
                for kind in contour {
                    let path = Entity { id: 0, kind, construction: false }.polyline(segments);
                    if points.is_empty() {
                        points.extend(path);
                    } else {
                        points.extend(path.into_iter().skip(1));
                    }
                }
                points
            })
            .collect()
    }

    /// Closest point on any glyph contour.
    pub fn nearest(&self, target: P2) -> (P2, f64) {
        self.outlines()
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .map(|kind| Entity { id: 0, kind, construction: false }.nearest(target))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((self.position, f64::INFINITY))
    }
}

struct Builder<'a> {
    text: &'a SketchText,
    scale: f64,
    advance: f64,
    contours: Vec<Vec<EntityKind>>,
    current: Vec<EntityKind>,
    start: P2,
    last: P2,
}

impl Builder<'_> {
    fn point(&self, x: f32, y: f32) -> P2 {
        let axis = P2::from_angle(self.text.angle);
        self.text.position
            + axis.scale((f64::from(x) + self.advance) * self.scale)
            + axis.perp().scale(f64::from(y) * self.scale * if self.text.reflected { -1.0 } else { 1.0 })
    }

    fn curve(&mut self, controls: Vec<P2>) {
        self.last = *controls.last().unwrap();
        self.current.push(EntityKind::Spline { mode: SplineMode::Control, points: controls });
    }
}

impl OutlineBuilder for Builder<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        self.start = self.point(x, y);
        self.last = self.start;
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let to = self.point(x, y);
        if self.last.dist(to) > 1e-12 {
            self.current.push(EntityKind::Line { a: self.last, b: to });
        }
        self.last = to;
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        self.curve(vec![self.last, self.point(x1, y1), self.point(x, y)]);
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        self.curve(vec![self.last, self.point(x1, y1), self.point(x2, y2), self.point(x, y)]);
    }
    fn close(&mut self) {
        if self.last.dist(self.start) > 1e-12 {
            self.current.push(EntityKind::Line { a: self.last, b: self.start });
        }
        if !self.current.is_empty() {
            self.contours.push(std::mem::take(&mut self.current));
        }
        self.last = self.start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soliddraft_kernel_api::{BezierControls, Plane, Segment};

    #[test]
    fn outline_callbacks_preserve_cubic_curves_and_close_the_contour() {
        let text = SketchText::default();
        let mut builder = Builder {
            text: &text,
            scale: 1.0,
            advance: 0.0,
            contours: Vec::new(),
            current: Vec::new(),
            start: P2::ZERO,
            last: P2::ZERO,
        };
        builder.move_to(0.0, 0.0);
        builder.curve_to(0.0, 4.0, 4.0, 4.0, 4.0, 0.0);
        builder.close();
        let mut sketch = crate::Sketch::new();
        for kind in builder.contours.into_iter().flatten() {
            sketch.add(kind, false);
        }
        let region = &sketch.regions().unwrap()[0];
        assert!((region.area() - 9.6).abs() < 1e-9);
        assert!(
            region
                .outer
                .segments
                .iter()
                .any(|s| matches!(s, Segment::Bezier { controls: BezierControls::Cubic(_), .. }))
        );
        region.to_profile(Plane::XY).check().unwrap();
    }
}
