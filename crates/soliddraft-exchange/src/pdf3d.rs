//! Interactive PDF 1.7 for one static indexed triangle mesh and named perspective cameras.
//!
//! The in-tree U3D encoder implements the uncompressed static CLOD subset of
//! [ECMA-363, 3rd edition](https://ecma-international.org/publications-and-standards/standards/ecma-363/).
//! There are no external encoders, proprietary SDKs, or embedded scripts. The PDF objects follow
//! [ISO 32000-1 §13.6](https://opensource.adobe.com/dc-acrobat-sdk-docs/pdfstandards/PDF32000_2008.pdf).
//! Activate the model in a 3D-capable reader such as Adobe Acrobat Reader; ordinary PDF readers
//! show the inactive appearance. This module does not encode textures, animation, exact surfaces,
//! or semantic PMI. It does not extract geometry or choose engineering values.

mod u3d;

use std::collections::HashSet;
use std::fmt;
use std::io::Write as _;

/// Neutral model geometry and saved cameras, in a shared right-handed world coordinate system.
#[derive(Debug)]
pub struct Scene<'a> {
    /// Vertex positions in millimetres, as stored in the U3D mesh.
    pub positions_mm: &'a [[f32; 3]],
    /// Zero-based position indices. Winding is counterclockwise when viewed from the front.
    pub triangles: &'a [[u32; 3]],
    /// Saved cameras in menu order; the first is the default. At least one is required.
    pub views: &'a [View<'a>],
}

/// A named perspective camera framing the mesh.
#[derive(Debug)]
pub struct View<'a> {
    /// A unique, non-blank display name.
    pub name: &'a str,
    /// Camera position in millimetres.
    pub eye_mm: [f32; 3],
    /// Point the camera looks at and orbits, in millimetres.
    pub target_mm: [f32; 3],
    /// Up direction, not parallel to the viewing direction.
    pub up: [f32; 3],
    /// Perspective field of view in degrees, strictly between 0 and 180.
    pub fov_degrees: f32,
}

/// A scene outside the supported 3D PDF subset.
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// No triangles or positions.
    EmptyMesh,
    /// At least one position is not finite.
    NonFinitePosition,
    /// Triangle at this zero-based index has a missing corner or zero area.
    InvalidTriangle(usize),
    /// At least one named view is required.
    NoViews,
    /// View at this zero-based index has an invalid name or camera.
    InvalidView(usize),
    /// The scene exceeds a U3D block or PDF file limit.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyMesh => f.write_str("3D PDF requires a non-empty triangle mesh"),
            Self::NonFinitePosition => f.write_str("3D PDF mesh contains a non-finite position"),
            Self::InvalidTriangle(i) => write!(f, "3D PDF triangle {i} has an invalid index or zero area"),
            Self::NoViews => f.write_str("3D PDF requires at least one named view"),
            Self::InvalidView(i) => write!(f, "3D PDF view {i} has an invalid or duplicate name or an invalid camera"),
            Self::TooLarge => f.write_str("3D PDF scene exceeds the supported file size"),
        }
    }
}

impl std::error::Error for Error {}

/// Encodes a one-page PDF with a click-to-activate U3D annotation and the supplied saved views.
/// Unsupported geometry and cameras are returned as errors rather than omitted or replaced.
pub fn encode(scene: &Scene<'_>) -> Result<Vec<u8>, Error> {
    if scene.views.is_empty() {
        return Err(Error::NoViews);
    }
    let mesh_size = u3d::base_mesh_size(scene)?;
    let mut pdf = Pdf { bytes: Vec::with_capacity(mesh_size.saturating_add(2048)), offsets: [0; 7] };
    pdf.bytes.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");
    pdf.object(1, b"<< /Type /Catalog /Pages 2 0 R >>");
    pdf.object(2, b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>");
    pdf.object(3, b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 612] /Resources << >> /Annots [4 0 R] >>");
    pdf.object(
        4,
        b"<< /Type /Annot /Subtype /3D /Rect [36 36 576 576] /Contents (Interactive 3D model) \
          /3DD 5 0 R /3DV 0 /3DI true /3DA << /A /XA /TB true >> /AP << /N 7 0 R >> >>",
    );
    pdf.begin(5);
    pdf.bytes.extend_from_slice(b"<< /Type /3D /Subtype /U3D /Length 6 0 R /DV 0 /VA [\n");
    let mut names = HashSet::with_capacity(scene.views.len());
    for (i, view) in scene.views.iter().enumerate() {
        if view.name.trim().is_empty() || view.name.chars().any(char::is_control) || !names.insert(view.name) {
            return Err(Error::InvalidView(i));
        }
        let (matrix, orbit) = camera(view).ok_or(Error::InvalidView(i))?;
        write_view(&mut pdf.bytes, view, matrix, orbit);
    }
    pdf.bytes.extend_from_slice(b"] >>\nstream\n");
    let start = pdf.bytes.len();
    u3d::write(&mut pdf.bytes, scene)?;
    let length = pdf.bytes.len() - start;
    pdf.bytes.extend_from_slice(b"\nendstream\nendobj\n");
    pdf.begin(6);
    writeln!(pdf.bytes, "{length}\nendobj").unwrap();

    // PDF readers without 3D support still show why the page is otherwise empty.
    let appearance = b"0.95 0.96 0.98 rg 0 0 540 540 re f\n\
                       0.18 0.22 0.28 rg BT /F1 18 Tf 155 264 Td (Activate 3D model in a 3D PDF reader) Tj ET";
    pdf.begin(7);
    writeln!(
        pdf.bytes,
        "<< /Type /XObject /Subtype /Form /BBox [0 0 540 540] \
         /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> \
         /Length {} >>\nstream",
        appearance.len(),
    )
    .unwrap();
    pdf.bytes.extend_from_slice(appearance);
    pdf.bytes.extend_from_slice(b"\nendstream\nendobj\n");
    pdf.finish()
}

fn write_view(out: &mut Vec<u8>, view: &View<'_>, matrix: [f64; 12], orbit: f64) {
    out.extend_from_slice(b"<< /Type /3DView /XN <FEFF");
    for unit in view.name.encode_utf16() {
        write!(out, "{unit:04X}").unwrap();
    }
    out.extend_from_slice(b"> /MS /M /C2W [");
    for number in matrix {
        write!(out, " {number}").unwrap();
    }
    writeln!(
        out,
        " ] /CO {orbit} /P << /Type /3DProjection /Subtype /P /N 0.01 /FOV {} >> \
         /LS << /Type /3DLightingScheme /Subtype /CAD >> >>",
        view.fov_degrees,
    )
    .unwrap();
}

fn camera(view: &View<'_>) -> Option<([f64; 12], f64)> {
    if !(0.0 < view.fov_degrees && view.fov_degrees < 180.0) {
        return None;
    }
    let eye = view.eye_mm.map(f64::from);
    let sight = difference(view.target_mm, view.eye_mm);
    let (forward, distance) = normalized(sight)?;
    if distance > f64::from(f32::MAX) {
        return None;
    }
    // PDF's camera looks along +Z, with +X screen-right and +Y screen-up.
    let (right, _) = normalized(cross(sight, view.up.map(f64::from)))?;
    let up = cross(right, forward);
    let [rx, ry, rz] = right;
    let [ux, uy, uz] = up;
    let [fx, fy, fz] = forward;
    let [ex, ey, ez] = eye;
    Some(([rx, ry, rz, ux, uy, uz, fx, fy, fz, ex, ey, ez], distance))
}

fn difference(a: [f32; 3], b: [f32; 3]) -> [f64; 3] {
    std::array::from_fn(|i| f64::from(a[i]) - f64::from(b[i]))
}

fn cross([ax, ay, az]: [f64; 3], [bx, by, bz]: [f64; 3]) -> [f64; 3] {
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn normalized(v: [f64; 3]) -> Option<([f64; 3], f64)> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length.is_finite() && length > 0.0).then(|| (v.map(|x| x / length), length))
}

struct Pdf {
    bytes: Vec<u8>,
    offsets: [usize; 7],
}

impl Pdf {
    fn begin(&mut self, id: usize) {
        self.offsets[id - 1] = self.bytes.len();
        writeln!(self.bytes, "{id} 0 obj").unwrap();
    }

    fn object(&mut self, id: usize, body: &[u8]) {
        self.begin(id);
        self.bytes.extend_from_slice(body);
        self.bytes.extend_from_slice(b"\nendobj\n");
    }

    fn finish(mut self) -> Result<Vec<u8>, Error> {
        let xref = self.bytes.len();
        if xref as u64 > 9_999_999_999 {
            return Err(Error::TooLarge);
        }
        let size = self.offsets.len() + 1;
        writeln!(self.bytes, "xref\n0 {size}\n0000000000 65535 f ").unwrap();
        for offset in self.offsets {
            writeln!(self.bytes, "{offset:010} 00000 n ").unwrap();
        }
        write!(self.bytes, "trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",).unwrap();
        Ok(self.bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TRIANGLE: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
    const VIEW: View<'static> = View {
        name: "Top",
        eye_mm: [0.0, 0.0, 4.0],
        target_mm: [0.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        fov_degrees: 35.0,
    };

    #[test]
    fn refuses_out_of_range_and_degenerate_triangles() {
        for triangles in [[[0, 1, 3]], [[0, 1, 1]]] {
            assert_eq!(
                encode(&Scene { positions_mm: &TRIANGLE, triangles: &triangles, views: &[VIEW] }),
                Err(Error::InvalidTriangle(0)),
            );
        }
    }

    #[test]
    fn refuses_parallel_camera_up_and_ambiguous_view_names() {
        let scene = Scene {
            positions_mm: &TRIANGLE,
            triangles: &[[0, 1, 2]],
            views: &[VIEW, View { name: "Parallel", up: [0.0, 0.0, 1.0], ..VIEW }],
        };
        assert_eq!(encode(&scene), Err(Error::InvalidView(1)));
        assert_eq!(encode(&Scene { views: &[VIEW, VIEW], ..scene }), Err(Error::InvalidView(1)));
    }
}
