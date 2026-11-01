/**
 * Procedural ambience in WebAudio. No files. A low pad, a little wind, and short cues the
 * chapters fire. Off until the visitor presses the sound button, as browsers require.
 */
export type CueName = "tick" | "thud" | "chime" | "whoosh";

export interface Sound {
  readonly enabled: boolean;
  toggle(): Promise<boolean>;
  cue(name: CueName): void;
  dispose(): void;
}

export function createSound(): Sound {
  let ctx: AudioContext | null = null;
  let master: GainNode | null = null;
  let enabled = false;
  let padNodes: AudioNode[] = [];

  const build = () => {
    const AC = window.AudioContext ?? (window as unknown as { webkitAudioContext: typeof AudioContext }).webkitAudioContext;
    ctx = new AC();
    master = ctx.createGain();
    master.gain.value = 0;
    const comp = ctx.createDynamicsCompressor();
    comp.threshold.value = -18;
    comp.ratio.value = 4;
    master.connect(comp).connect(ctx.destination);

    // Pad: three detuned voices through a slowly moving low-pass.
    const lp = ctx.createBiquadFilter();
    lp.type = "lowpass";
    lp.frequency.value = 320;
    lp.Q.value = 0.8;
    const padGain = ctx.createGain();
    padGain.gain.value = 0.16;
    lp.connect(padGain).connect(master);
    const voices: Array<[number, OscillatorType, number]> = [
      [55, "sine", 0],
      [82.41, "triangle", 3],
      [110, "sine", -4],
      [164.81, "triangle", 6],
    ];
    for (const [f, type, detune] of voices) {
      const osc = ctx.createOscillator();
      osc.type = type;
      osc.frequency.value = f;
      osc.detune.value = detune;
      const g = ctx.createGain();
      g.gain.value = f > 150 ? 0.12 : 0.3;
      osc.connect(g).connect(lp);
      osc.start();
      padNodes.push(osc, g);
    }
    const lfo = ctx.createOscillator();
    lfo.frequency.value = 0.045;
    const lfoGain = ctx.createGain();
    lfoGain.gain.value = 140;
    lfo.connect(lfoGain).connect(lp.frequency);
    lfo.start();
    padNodes.push(lfo, lfoGain);

    // Wind: brown noise through a band-pass with a slow wobble.
    const seconds = 4;
    const buffer = ctx.createBuffer(1, ctx.sampleRate * seconds, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    let last = 0;
    for (let i = 0; i < data.length; i++) {
      const white = Math.random() * 2 - 1;
      last = (last + 0.02 * white) / 1.02;
      data[i] = last * 3.5;
    }
    const noise = ctx.createBufferSource();
    noise.buffer = buffer;
    noise.loop = true;
    const bp = ctx.createBiquadFilter();
    bp.type = "bandpass";
    bp.frequency.value = 420;
    bp.Q.value = 0.6;
    const windGain = ctx.createGain();
    windGain.gain.value = 0.05;
    noise.connect(bp).connect(windGain).connect(master);
    noise.start();
    const wobble = ctx.createOscillator();
    wobble.frequency.value = 0.07;
    const wobbleGain = ctx.createGain();
    wobbleGain.gain.value = 180;
    wobble.connect(wobbleGain).connect(bp.frequency);
    wobble.start();
    padNodes.push(noise, bp, windGain, wobble, wobbleGain);
  };

  const env = (g: GainNode, peak: number, attack: number, decay: number) => {
    const t = ctx!.currentTime;
    g.gain.cancelScheduledValues(t);
    g.gain.setValueAtTime(0.0001, t);
    g.gain.exponentialRampToValueAtTime(peak, t + attack);
    g.gain.exponentialRampToValueAtTime(0.0001, t + attack + decay);
  };

  const cues: Record<CueName, () => void> = {
    tick() {
      const osc = ctx!.createOscillator();
      const g = ctx!.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(1400, ctx!.currentTime);
      osc.frequency.exponentialRampToValueAtTime(320, ctx!.currentTime + 0.08);
      osc.connect(g).connect(master!);
      env(g, 0.18, 0.004, 0.12);
      osc.start();
      osc.stop(ctx!.currentTime + 0.2);
    },
    chime() {
      for (const [f, d] of [
        [523.25, 0],
        [783.99, 0.09],
      ] as const) {
        const osc = ctx!.createOscillator();
        const g = ctx!.createGain();
        osc.type = "sine";
        osc.frequency.value = f;
        osc.connect(g).connect(master!);
        const t = ctx!.currentTime + d;
        g.gain.setValueAtTime(0.0001, t);
        g.gain.exponentialRampToValueAtTime(0.12, t + 0.01);
        g.gain.exponentialRampToValueAtTime(0.0001, t + 0.9);
        osc.start(t);
        osc.stop(t + 1);
      }
    },
    thud() {
      const osc = ctx!.createOscillator();
      const g = ctx!.createGain();
      osc.type = "sine";
      osc.frequency.setValueAtTime(95, ctx!.currentTime);
      osc.frequency.exponentialRampToValueAtTime(28, ctx!.currentTime + 0.5);
      osc.connect(g).connect(master!);
      env(g, 0.9, 0.006, 0.7);
      osc.start();
      osc.stop(ctx!.currentTime + 0.8);
      // Impact noise.
      const n = ctx!.createBufferSource();
      const b = ctx!.createBuffer(1, ctx!.sampleRate * 0.3, ctx!.sampleRate);
      const d = b.getChannelData(0);
      for (let i = 0; i < d.length; i++) d[i] = (Math.random() * 2 - 1) * (1 - i / d.length) ** 2;
      n.buffer = b;
      const lp = ctx!.createBiquadFilter();
      lp.type = "lowpass";
      lp.frequency.value = 900;
      const ng = ctx!.createGain();
      n.connect(lp).connect(ng).connect(master!);
      env(ng, 0.5, 0.003, 0.3);
      n.start();
    },
    whoosh() {
      const n = ctx!.createBufferSource();
      const b = ctx!.createBuffer(1, ctx!.sampleRate * 0.8, ctx!.sampleRate);
      const d = b.getChannelData(0);
      for (let i = 0; i < d.length; i++) d[i] = Math.random() * 2 - 1;
      n.buffer = b;
      const bp = ctx!.createBiquadFilter();
      bp.type = "bandpass";
      bp.Q.value = 1.2;
      bp.frequency.setValueAtTime(300, ctx!.currentTime);
      bp.frequency.exponentialRampToValueAtTime(2400, ctx!.currentTime + 0.5);
      const g = ctx!.createGain();
      n.connect(bp).connect(g).connect(master!);
      env(g, 0.14, 0.2, 0.5);
      n.start();
    },
  };

  return {
    get enabled() {
      return enabled;
    },
    async toggle() {
      if (!ctx) build();
      if (ctx!.state === "suspended") await ctx!.resume();
      enabled = !enabled;
      const t = ctx!.currentTime;
      master!.gain.cancelScheduledValues(t);
      master!.gain.setValueAtTime(master!.gain.value, t);
      master!.gain.linearRampToValueAtTime(enabled ? 0.8 : 0, t + 0.8);
      return enabled;
    },
    cue(name) {
      if (!enabled || !ctx || !master) return;
      try {
        cues[name]();
      } catch {}
    },
    dispose() {
      for (const n of padNodes) {
        if ("stop" in n) {
          try {
            (n as OscillatorNode).stop();
          } catch {}
        }
        n.disconnect();
      }
      padNodes = [];
      void ctx?.close();
      ctx = null;
    },
  };
}
