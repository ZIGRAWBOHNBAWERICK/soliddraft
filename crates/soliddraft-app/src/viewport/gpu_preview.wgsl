// The Catallax mesher's vertices to the viewport's face vertices and silhouette
// inputs. One invocation per triangle; a vertex shared by two triangles of one face
// is written twice with the same value.

struct PreviewVertex {
    position: vec4<f32>,
    normal: vec4<f32>,
}

struct Params {
    origin: vec4<f32>,
    triangles: u32,
    row: u32,
    pad0: u32,
    pad1: u32,
}

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var<storage, read> source: array<PreviewVertex>;
@group(0) @binding(2) var<storage, read> indices: array<u32>;
@group(0) @binding(3) var<storage, read> triangle_faces: array<u32>;
// pick, feature, appearance, unused
@group(0) @binding(4) var<storage, read> faces: array<vec4<u32>>;
// FaceVertex: position xyz, normal xyz, face id, feature id, appearance
@group(0) @binding(5) var<storage, read_write> vertices: array<u32>;
// SilhouetteTriangle: three of position xyz, normal xyz
@group(0) @binding(6) var<storage, read_write> silhouettes: array<f32>;

@compute @workgroup_size(64)
fn convert(@builtin(global_invocation_id) id: vec3<u32>) {
    let t = id.x + id.y * params.row;
    if t >= params.triangles {
        return;
    }
    let face = faces[triangle_faces[t]];
    for (var k = 0u; k < 3u; k++) {
        let index = indices[3u * t + k];
        let v = source[index];
        let p = v.position.xyz + params.origin.xyz;
        let n = v.normal.xyz;
        let base = index * 9u;
        vertices[base] = bitcast<u32>(p.x);
        vertices[base + 1u] = bitcast<u32>(p.y);
        vertices[base + 2u] = bitcast<u32>(p.z);
        vertices[base + 3u] = bitcast<u32>(n.x);
        vertices[base + 4u] = bitcast<u32>(n.y);
        vertices[base + 5u] = bitcast<u32>(n.z);
        vertices[base + 6u] = face.x;
        vertices[base + 7u] = face.y;
        vertices[base + 8u] = face.z;
        let s = (3u * t + k) * 6u;
        silhouettes[s] = p.x;
        silhouettes[s + 1u] = p.y;
        silhouettes[s + 2u] = p.z;
        silhouettes[s + 3u] = n.x;
        silhouettes[s + 4u] = n.y;
        silhouettes[s + 5u] = n.z;
    }
}
