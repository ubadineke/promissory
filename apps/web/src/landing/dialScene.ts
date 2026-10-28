/**
 * The landing hero: a frosted-glass dial with a progress ring, over a rising sun. The ring
 * fills as a payment's time comes, the sun flares when it pays, and the cycle starts again.
 *
 * Loaded on demand by the landing page only, so the app itself never ships three.js.
 */

import {
  ACESFilmicToneMapping,
  AdditiveBlending,
  BoxGeometry,
  CanvasTexture,
  Color,
  CylinderGeometry,
  Group,
  Mesh,
  MeshBasicMaterial,
  MeshPhysicalMaterial,
  PerspectiveCamera,
  PMREMGenerator,
  Scene,
  ShaderMaterial,
  SphereGeometry,
  Sprite,
  SpriteMaterial,
  SRGBColorSpace,
  TorusGeometry,
  WebGLRenderer,
} from "three";
import { RoomEnvironment } from "three/examples/jsm/environments/RoomEnvironment.js";

export interface DialTick {
  /** Seconds left in the demo countdown; 0 while it shows "paid". */
  remaining: number;
  paid: boolean;
}

export interface DialOptions {
  /** Called a few times a second with what the overlay should show. */
  onTick?: (t: DialTick) => void;
  /** Draw one still frame and never animate. */
  still?: boolean;
}

const COUNTDOWN_SECS = 12;
const PAID_SECS = 2.4;
const RING_RADIUS = 1.62;
const DISTANCE = 9;

/** The ring is a whole torus; the shader hides the part not yet reached. */
function ringMaterial(uniforms: { uProgress: { value: number }; uColor: { value: Color } }) {
  return new ShaderMaterial({
    transparent: true,
    uniforms,
    vertexShader: /* glsl */ `
      varying vec3 vPos;
      varying vec3 vNormal;
      void main() {
        vPos = position;
        vNormal = normalize(normalMatrix * normal);
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: /* glsl */ `
      uniform float uProgress;
      uniform vec3 uColor;
      varying vec3 vPos;
      varying vec3 vNormal;
      const float TAU = 6.28318530718;
      void main() {
        // Clockwise from twelve o'clock, like a watch.
        float a = mod(atan(vPos.x, vPos.y) + TAU, TAU) / TAU;
        if (a > uProgress) discard;
        float light = 0.72 + 0.28 * max(dot(vNormal, normalize(vec3(-0.4, 0.6, 1.0))), 0.0);
        gl_FragColor = vec4(uColor * light, 1.0);
      }
    `,
  });
}

function glowTexture(): CanvasTexture {
  const c = document.createElement("canvas");
  c.width = c.height = 256;
  const g = c.getContext("2d")!;
  const grad = g.createRadialGradient(128, 128, 0, 128, 128, 128);
  grad.addColorStop(0, "rgba(255,226,184,1)");
  grad.addColorStop(0.35, "rgba(242,166,90,0.55)");
  grad.addColorStop(1, "rgba(242,166,90,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, 256, 256);
  const t = new CanvasTexture(c);
  t.colorSpace = SRGBColorSpace;
  return t;
}

/** A soft-edged sun: behind the translucent glass it reads as frosted. */
function sunTexture(): CanvasTexture {
  const c = document.createElement("canvas");
  c.width = c.height = 256;
  const g = c.getContext("2d")!;
  const grad = g.createRadialGradient(128, 110, 0, 128, 128, 128);
  grad.addColorStop(0, "#ffe9c9");
  grad.addColorStop(0.42, "#f7b56c");
  grad.addColorStop(0.6, "rgba(239,148,74,1)");
  grad.addColorStop(0.72, "rgba(242,166,90,0.3)");
  grad.addColorStop(1, "rgba(242,166,90,0)");
  g.fillStyle = grad;
  g.fillRect(0, 0, 256, 256);
  const t = new CanvasTexture(c);
  t.colorSpace = SRGBColorSpace;
  return t;
}

function cssColor(name: string, fallback: string): Color {
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return new Color(v || fallback);
}

/**
 * Renders into `host` and returns a disposer, or `null` when WebGL is unavailable (the caller
 * keeps its static fallback).
 */
export function mountDial(host: HTMLElement, opts: DialOptions = {}): (() => void) | null {
  let renderer: WebGLRenderer;
  try {
    renderer = new WebGLRenderer({ antialias: true, alpha: true, powerPreference: "low-power" });
  } catch {
    return null;
  }
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.toneMapping = ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.domElement.setAttribute("aria-hidden", "true");
  renderer.domElement.style.display = "block";
  host.appendChild(renderer.domElement);

  const scene = new Scene();
  const pmrem = new PMREMGenerator(renderer);
  const env = pmrem.fromScene(new RoomEnvironment(), 0.04).texture;
  scene.environment = env;

  const camera = new PerspectiveCamera(32, 1, 0.1, 50);
  camera.position.set(0, 0.15, DISTANCE);

  // The sun sits behind the glass so the dial refracts it.
  const sunTex = sunTexture();
  const sun = new Sprite(new SpriteMaterial({ map: sunTex, transparent: true, depthWrite: false, toneMapped: false }));
  sun.scale.setScalar(3.6);
  sun.position.set(0.25, -1.05, -2.2);
  scene.add(sun);
  const glowTex = glowTexture();
  const glow = new Sprite(new SpriteMaterial({ map: glowTex, transparent: true, depthWrite: false, blending: AdditiveBlending }));
  glow.scale.setScalar(5.5);
  glow.position.copy(sun.position).setZ(-2.4);
  scene.add(glow);

  const dial = new Group();
  scene.add(dial);

  const glass = new Mesh(
    new CylinderGeometry(1.36, 1.36, 0.24, 128, 1),
    new MeshPhysicalMaterial({
      color: cssColor("--surface", "#ffffff"),
      transparent: true,
      opacity: 0.6,
      depthWrite: false,
      roughness: 0.55,
      clearcoat: 0.6,
      clearcoatRoughness: 0.25,
      envMapIntensity: 0.35,
    }),
  );
  glass.rotation.x = Math.PI / 2;
  dial.add(glass);
  const rim = new Mesh(
    new TorusGeometry(1.36, 0.014, 12, 200),
    new MeshBasicMaterial({ color: 0xffffff, transparent: true, opacity: 0.55 }),
  );
  dial.add(rim);

  const fg = cssColor("--fg", "#14213d");
  const track = new Mesh(
    new TorusGeometry(RING_RADIUS, 0.035, 16, 220),
    new MeshBasicMaterial({ color: fg, transparent: true, opacity: 0.16 }),
  );
  dial.add(track);
  const ring = { uProgress: { value: 0 }, uColor: { value: fg.clone() } };
  const arcMat = ringMaterial(ring);
  const arc = new Mesh(new TorusGeometry(RING_RADIUS, 0.062, 20, 260), arcMat);
  dial.add(arc);
  const beadMat = new MeshBasicMaterial({ color: new Color("#f2a65a"), toneMapped: false });
  const bead = new Mesh(new SphereGeometry(0.11, 32, 32), beadMat);
  dial.add(bead);

  const tickMat = new MeshBasicMaterial({ color: fg, transparent: true, opacity: 0.4 });
  const tickGeo = new BoxGeometry(0.035, 0.16, 0.02);
  for (let i = 0; i < 12; i++) {
    const a = (i / 12) * Math.PI * 2;
    const tick = new Mesh(tickGeo, tickMat);
    tick.position.set(Math.sin(a) * 1.9, Math.cos(a) * 1.9, 0);
    tick.rotation.z = -a;
    dial.add(tick);
  }

  const scheme = window.matchMedia("(prefers-color-scheme: dark)");
  const recolor = () => {
    const c = cssColor("--fg", "#14213d");
    (track.material as MeshBasicMaterial).color.copy(c);
    tickMat.color.copy(c);
    (glass.material as MeshPhysicalMaterial).color.copy(cssColor("--surface", "#ffffff"));
    ring.uColor.value.copy(c);
  };
  scheme.addEventListener("change", recolor);

  const resize = () => {
    const w = host.clientWidth || 1;
    const h = host.clientHeight || 1;
    renderer.setSize(w, h, false);
    renderer.domElement.style.width = `${w}px`;
    renderer.domElement.style.height = `${h}px`;
    camera.aspect = w / h;
    // Keep the whole dial in frame on narrow screens.
    camera.position.z = w / h < 0.9 ? DISTANCE / (w / h / 0.9) : DISTANCE;
    camera.updateProjectionMatrix();
  };
  const ro = new ResizeObserver(resize);
  ro.observe(host);
  resize();

  // Pointer parallax, eased.
  const target = { x: 0, y: 0 };
  const onPointer = (e: PointerEvent) => {
    target.x = (e.clientX / window.innerWidth - 0.5) * 2;
    target.y = (e.clientY / window.innerHeight - 0.5) * 2;
  };
  if (!opts.still) window.addEventListener("pointermove", onPointer, { passive: true });

  const cycle = COUNTDOWN_SECS + PAID_SECS;
  let lastTick = -1;
  let lastPaid = false;

  function frame(t: number) {
    const phase = opts.still ? COUNTDOWN_SECS * 0.72 : t % cycle;
    const paid = phase >= COUNTDOWN_SECS;
    const progress = paid ? 1 : phase / COUNTDOWN_SECS;
    ring.uProgress.value = progress;
    const a = progress * Math.PI * 2;
    bead.position.set(Math.sin(a) * RING_RADIUS, Math.cos(a) * RING_RADIUS, 0.02);

    // The sun climbs as the payment nears, and flares when it lands.
    const flare = paid ? Math.sin(((phase - COUNTDOWN_SECS) / PAID_SECS) * Math.PI) : 0;
    sun.position.y = -1.7 + progress * 0.55;
    glow.position.y = sun.position.y;
    glow.scale.setScalar(5.5 + flare * 1.6);
    beadMat.color.setRGB(0.95 + flare * 0.05, 0.65 + flare * 0.3, 0.35 + flare * 0.4);

    dial.rotation.y += (target.x * 0.28 - dial.rotation.y) * 0.05;
    dial.rotation.x += (target.y * 0.18 - dial.rotation.x) * 0.05;
    dial.position.y = opts.still ? 0 : Math.sin(t * 0.8) * 0.06;

    renderer.render(scene, camera);

    const remaining = paid ? 0 : Math.ceil(COUNTDOWN_SECS - phase);
    if (remaining !== lastTick || paid !== lastPaid) {
      lastTick = remaining;
      lastPaid = paid;
      opts.onTick?.({ remaining, paid });
    }
  }

  let raf = 0;
  let visible = true;
  const start = performance.now();
  const loop = () => {
    frame((performance.now() - start) / 1000);
    raf = requestAnimationFrame(loop);
  };
  const run = () => {
    cancelAnimationFrame(raf);
    if (opts.still) frame(0);
    else if (visible && !document.hidden) raf = requestAnimationFrame(loop);
  };
  const io = new IntersectionObserver(([e]) => {
    visible = !!e?.isIntersecting;
    run();
  });
  io.observe(host);
  document.addEventListener("visibilitychange", run);
  run();

  return () => {
    cancelAnimationFrame(raf);
    io.disconnect();
    ro.disconnect();
    document.removeEventListener("visibilitychange", run);
    window.removeEventListener("pointermove", onPointer);
    scheme.removeEventListener("change", recolor);
    scene.traverse((o) => {
      // A sprite's geometry is shared by three.js itself; only its material is ours.
      if (o instanceof Mesh) o.geometry.dispose();
      if (o instanceof Mesh || o instanceof Sprite) (o.material as { dispose(): void }).dispose();
    });
    sunTex.dispose();
    glowTex.dispose();
    env.dispose();
    pmrem.dispose();
    renderer.dispose();
    renderer.domElement.remove();
  };
}
