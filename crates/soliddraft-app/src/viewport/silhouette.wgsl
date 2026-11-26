// One instance per smooth triangle. The zero of N dot V is approximated by
// interpolating the facing values along its sides. This uses the kept display
// mesh, not the B-rep or the picking geometry.
struct SilhouetteIn {
    @builtin(vertex_index) corner: u32,
    @location(0) a: vec3<f32>,
    @location(1) na: vec3<f32>,
    @location(2) b: vec3<f32>,
    @location(3) nb: vec3<f32>,
    @location(4) c: vec3<f32>,
    @location(5) nc: vec3<f32>,
};

fn zero_crossing(a: vec3<f32>, b: vec3<f32>, fa: f32, fb: f32) -> vec3<f32> {
    return mix(a, b, fa / (fa - fb));
}

@vertex
fn vs_silhouette(in: SilhouetteIn) -> LineOut {
    let fa = dot(in.na, toward_eye(in.a));
    let fb = dot(in.nb, toward_eye(in.b));
    let fc = dot(in.nc, toward_eye(in.c));
    let sa = fa >= 0.0;
    let sb = fb >= 0.0;
    let sc = fc >= 0.0;
    let clipped = LineOut(vec4<f32>(0.0, 0.0, -1.0, 1.0), 0u);
    if (sa == sb && sb == sc) {
        return clipped;
    }

    // With mixed signs, the lone vertex on one side meets both crossed edges.
    var a: vec3<f32>;
    var b: vec3<f32>;
    if (sa == sb) {
        a = zero_crossing(in.c, in.a, fc, fa);
        b = zero_crossing(in.c, in.b, fc, fb);
    } else if (sb == sc) {
        a = zero_crossing(in.a, in.b, fa, fb);
        b = zero_crossing(in.a, in.c, fa, fc);
    } else {
        a = zero_crossing(in.b, in.c, fb, fc);
        b = zero_crossing(in.b, in.a, fb, fa);
    }
    if (all(a == b)) {
        return clipped;
    }
    return LineOut(line_vertex(LineIn(in.corner, a, b, 0u), globals.viewport.z), 0u);
}
