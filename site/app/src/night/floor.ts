/**
 * The shared dot floor. One big plane at y = 0, dots at the grid points, fading with distance,
 * tinted per chapter, drifting slowly so the page never sits still. A ripple ring can be fired
 * from any point (the steel frame landing uses it).
 */
import * as THREE from "three";

import type { FloorControl } from "./types";

const vert = /* glsl */ `
  uniform float uTime;
  uniform vec3 uRipple;   // x, z, start time
  uniform float uRippleStrength;
  varying vec3 vWorld;
  varying float vRipple;
  void main() {
    vec4 world = modelMatrix * vec4(position, 1.0);
    float age = uTime - uRipple.z;
    float d = distance(world.xz, uRipple.xy);
    float ring = 0.0;
    if (uRippleStrength > 0.0 && age > 0.0 && age < 4.0) {
      float r = age * 9.0;
      float w = 1.2 + age * 0.8;
      ring = exp(-pow((d - r) / w, 2.0)) * exp(-age * 1.1) * uRippleStrength;
      world.y += ring * 0.55;
    }
    vRipple = ring;
    vWorld = world.xyz;
    gl_Position = projectionMatrix * viewMatrix * world;
  }
`;

const frag = /* glsl */ `
  precision highp float;
  uniform vec3 uColor;
  uniform float uIntensity;
  uniform float uTime;
  uniform vec3 uCamera;
  uniform float uSpacing;
  uniform float uFade;
  varying vec3 vWorld;
  varying float vRipple;
  void main() {
    // Slow drift so the grid is always moving a little.
    vec2 p = vWorld.xz + vec2(0.0, uTime * 0.35);
    vec2 cell = fract(p / uSpacing) - 0.5;
    float d = length(cell) * uSpacing;
    float fw = fwidth(d) * 1.5;
    float dot = 1.0 - smoothstep(0.055 - fw, 0.055 + fw, d);
    // Slight shimmer per cell.
    vec2 id = floor(p / uSpacing);
    float tw = 0.75 + 0.25 * sin(uTime * 1.3 + id.x * 1.7 + id.y * 2.3);
    float dist = distance(vWorld.xz, uCamera.xz);
    float fade = 1.0 - smoothstep(uFade * 0.25, uFade, dist);
    float a = dot * tw * fade * uIntensity;
    a += vRipple * 0.3 * fade;
    gl_FragColor = vec4(uColor * (1.0 + vRipple * 1.5), a);
  }
`;

export class Floor {
  readonly mesh: THREE.Mesh;
  readonly control: FloorControl;
  private readonly material: THREE.ShaderMaterial;
  private readonly targetColor = new THREE.Color("#ff5b1f");
  private targetIntensity = 0.9;
  private targetVisible = 0;
  private visibleMix = 0;

  constructor() {
    const geometry = new THREE.PlaneGeometry(260, 260, 120, 120);
    geometry.rotateX(-Math.PI / 2);
    this.material = new THREE.ShaderMaterial({
      vertexShader: vert,
      fragmentShader: frag,
      transparent: true,
      depthWrite: false,
      blending: THREE.AdditiveBlending,
      uniforms: {
        uTime: { value: 0 },
        uColor: { value: new THREE.Color("#ff5b1f") },
        uIntensity: { value: 0 },
        uCamera: { value: new THREE.Vector3() },
        uSpacing: { value: 1.0 },
        uFade: { value: 70 },
        uRipple: { value: new THREE.Vector3(0, 0, -100) },
        uRippleStrength: { value: 0 },
      },
    });
    this.mesh = new THREE.Mesh(geometry, this.material);
    this.mesh.name = "floor";
    this.mesh.frustumCulled = false;
    this.mesh.renderOrder = -1;
    this.control = {
      set: (opts) => {
        if (opts.color !== undefined) this.targetColor.set(opts.color);
        if (opts.intensity !== undefined) this.targetIntensity = opts.intensity;
        if (opts.visible !== undefined) this.targetVisible = opts.visible ? 1 : 0;
      },
      ripple: (x, z, strength = 1) => {
        const u = this.material.uniforms;
        (u.uRipple.value as THREE.Vector3).set(x, z, u.uTime.value as number);
        u.uRippleStrength.value = strength;
      },
    };
  }

  update(dt: number, t: number, camera: THREE.Camera) {
    const u = this.material.uniforms;
    u.uTime.value = t;
    (u.uCamera.value as THREE.Vector3).copy(camera.position);
    const k = 1 - Math.exp(-dt * 4);
    (u.uColor.value as THREE.Color).lerp(this.targetColor, k);
    this.visibleMix += (this.targetVisible - this.visibleMix) * k;
    u.uIntensity.value += (this.targetIntensity * this.visibleMix - (u.uIntensity.value as number)) * k;
    this.mesh.visible = this.visibleMix > 0.01;
    // Keep the floor centred under the camera so it never runs out.
    this.mesh.position.x = Math.round(camera.position.x);
    this.mesh.position.z = Math.round(camera.position.z);
  }

  dispose() {
    this.mesh.geometry.dispose();
    this.material.dispose();
  }
}
