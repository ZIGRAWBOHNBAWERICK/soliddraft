//! ECMA-363 §§9.2, 9.4, 9.5.2, 9.6.1, 9.7.5 and 9.8.3–4. One model node references
//! one CLOD generator at its base resolution. A single opaque shader lights flat face normals.

use super::{Error, Scene, cross, difference, normalized};

const FILE_HEADER: u32 = 0x0044_3355;
const MODIFIER_CHAIN: u32 = 0xFFFF_FF14;
const MODEL_NODE: u32 = 0xFFFF_FF22;
const CLOD_MESH: u32 = 0xFFFF_FF31;
const CLOD_BASE_MESH: u32 = 0xFFFF_FF3B;
const SHADING_MODIFIER: u32 = 0xFFFF_FF45;
const LIT_TEXTURE_SHADER: u32 = 0xFFFF_FF53;
const MATERIAL_RESOURCE: u32 = 0xFFFF_FF54;

const MODEL: &str = "SolidDraft model";
const MESH: &str = "Mesh";
const SHADER: &str = "Shader";
const MATERIAL: &str = "Material";

pub(super) fn base_mesh_size(scene: &Scene<'_>) -> Result<usize, Error> {
    if scene.triangles.is_empty() || scene.positions_mm.is_empty() {
        return Err(Error::EmptyMesh);
    }
    // String, chain index, six counts, positions, then a normal and seven index
    // fields per face (the shading ID and a position/normal pair at each corner).
    let bytes = 2
        + MESH.len() as u64
        + 7 * 4
        + 12 * scene.positions_mm.len() as u64
        + (12 + 7 * 4) * scene.triangles.len() as u64;
    if bytes > u64::from(u32::MAX) || bytes + 2048 > isize::MAX as u64 {
        return Err(Error::TooLarge);
    }
    Ok(bytes as usize)
}

pub(super) fn write(out: &mut Vec<u8>, scene: &Scene<'_>) -> Result<(), Error> {
    let start = out.len();
    let header = begin(out, FILE_HEADER);
    // ECMA-363 §9.4.1: negative major version before formal compliance validation.
    u16s(out, [0xFFFF, 0]);
    u32s(out, [0x0000_000C]); // No arithmetic compression; defined units.
    let sizes = out.len();
    out.extend_from_slice(&[0; 12]); // Declaration size (U32), file size (U64).
    u32s(out, [106]); // IANA UTF-8.
    out.extend_from_slice(&0.001_f64.to_le_bytes()); // Millimetres to metres.
    end(out, header);

    write_shader(out);
    let faces = scene.triangles.len() as u32;
    let positions = scene.positions_mm.len() as u32;
    let chain = begin_chain(out, MESH, 1, 1);
    let mesh = begin(out, CLOD_MESH);
    string(out, MESH);
    // Chain index, mesh attributes, faces, positions, normals, colour and texture counts,
    // followed by one shading description without colours or textures.
    u32s(out, [0, 0, faces, positions, faces, 0, 0, 0, 1, 0, 0, 0]);
    u32s(out, [positions, positions]); // Minimum and maximum CLOD resolutions are equal.
    u32s(out, [1000, 1000, 1000]); // Quality factors; no progressive updates use them.
    f32s(out, [1.0; 5]); // Inverse quantization; the base mesh stores raw floats.
    f32s(out, [0.9, 0.5, 0.985]); // CLOD normal crease, update, and tolerance parameters.
    u32s(out, [0]); // No skeleton.
    end(out, mesh);
    end(out, chain);

    let chain = begin_chain(out, MODEL, 0, 2);
    let node = begin(out, MODEL_NODE);
    string(out, MODEL);
    u32s(out, [1]);
    string(out, ""); // The implicit world parent.
    f32s(out, [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
    string(out, MESH);
    u32s(out, [1]); // Render front faces.
    end(out, node);
    let shading = begin(out, SHADING_MODIFIER);
    string(out, MODEL);
    u32s(out, [1, 1, 1, 1]); // Chain index, mesh flag, one shader list with one shader.
    string(out, SHADER);
    end(out, shading);
    end(out, chain);
    let declarations = (out.len() - start) as u32;

    let mesh = begin(out, CLOD_BASE_MESH);
    string(out, MESH);
    u32s(out, [0, faces, positions, faces, 0, 0, 0]);
    for &position in scene.positions_mm {
        if !position.iter().all(|x| x.is_finite()) {
            return Err(Error::NonFinitePosition);
        }
        f32s(out, position);
    }
    for (i, &[a, b, c]) in scene.triangles.iter().enumerate() {
        let get = |index: u32| scene.positions_mm.get(index as usize).copied();
        let (Some(a), Some(b), Some(c)) = (get(a), get(b), get(c)) else {
            return Err(Error::InvalidTriangle(i));
        };
        let (normal, _) = normalized(cross(difference(b, a), difference(c, a))).ok_or(Error::InvalidTriangle(i))?;
        f32s(out, normal.map(|x| x as f32));
    }
    for (i, &[a, b, c]) in scene.triangles.iter().enumerate() {
        // The no-compression profile replaces the range-coded face fields with raw U32s.
        let n = i as u32;
        u32s(out, [0, a, n, b, n, c, n]);
    }
    end(out, mesh);
    let file_size = (out.len() - start) as u64;
    out[sizes..sizes + 4].copy_from_slice(&declarations.to_le_bytes());
    out[sizes + 4..sizes + 12].copy_from_slice(&file_size.to_le_bytes());
    Ok(())
}

fn write_shader(out: &mut Vec<u8>) {
    let material = begin(out, MATERIAL_RESOURCE);
    string(out, MATERIAL);
    u32s(out, [0x3F]); // All six colour/surface attributes are present.
    f32s(out, [0.17, 0.24, 0.34]); // Ambient.
    f32s(out, [0.28, 0.49, 0.77]); // Diffuse.
    f32s(out, [0.15, 0.15, 0.15]); // Specular.
    f32s(out, [0.0, 0.0, 0.0]); // Emissive.
    f32s(out, [0.2, 1.0]); // Reflectivity, opacity.
    end(out, material);
    let shader = begin(out, LIT_TEXTURE_SHADER);
    string(out, SHADER);
    u32s(out, [1]); // Lighting on, no alpha test or vertex colours.
    f32s(out, [0.0]); // Alpha test reference.
    u32s(out, [0x617, 0x606, 1, 0, 0]); // Always pass, alpha blend, pass 0, no texture channels.
    string(out, MATERIAL);
    end(out, shader);
}

fn begin_chain(out: &mut Vec<u8>, name: &str, kind: u32, count: u32) -> usize {
    let start = begin(out, MODIFIER_CHAIN);
    string(out, name);
    u32s(out, [kind, 0]); // No optional bounding data.
    pad(out, start);
    u32s(out, [count]);
    start
}

fn begin(out: &mut Vec<u8>, block_type: u32) -> usize {
    u32s(out, [block_type, 0, 0]); // Data size patched by end; metadata is empty.
    out.len()
}

fn end(out: &mut Vec<u8>, start: usize) {
    let size = (out.len() - start) as u32; // The variable-size base mesh was bounded up front.
    out[start - 8..start - 4].copy_from_slice(&size.to_le_bytes());
    pad(out, start);
}

fn pad(out: &mut Vec<u8>, start: usize) {
    let len = out.len();
    out.resize(len + (len - start).wrapping_neg() % 4, 0);
}

fn string(out: &mut Vec<u8>, s: &str) {
    u16s(out, [s.len() as u16]); // Only fixed internal names enter U3D.
    out.extend_from_slice(s.as_bytes());
}

fn u16s<const N: usize>(out: &mut Vec<u8>, values: [u16; N]) {
    for x in values {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn u32s<const N: usize>(out: &mut Vec<u8>, values: [u32; N]) {
    for x in values {
        out.extend_from_slice(&x.to_le_bytes());
    }
}

fn f32s<const N: usize>(out: &mut Vec<u8>, values: [f32; N]) {
    for x in values {
        out.extend_from_slice(&x.to_le_bytes());
    }
}
