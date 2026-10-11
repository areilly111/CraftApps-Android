// EffectCraft audio output (AudioWorklet): plays interleaved stereo blocks posted by the page
// (src/audio.rs feeds them from the preview mixdown). Running dry plays silence, reported back
// as underrun frames.
class EffectcraftOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.stopped = false;
    this.port.onmessage = (e) => {
      if (e.data === "stop") this.stopped = true;
      else this.queue.push(e.data);
    };
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const l = out[0];
    const r = out[1] ?? out[0];
    let i = 0;
    while (i < l.length && this.queue.length) {
      const b = this.queue[0];
      const n = Math.min(l.length - i, (b.length - this.offset) >> 1);
      for (let k = 0; k < n; k++) {
        l[i + k] = b[this.offset + 2 * k];
        r[i + k] = b[this.offset + 2 * k + 1];
      }
      i += n;
      this.offset += 2 * n;
      if (this.offset >= b.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    if (i < l.length) {
      l.fill(0, i);
      r.fill(0, i);
      if (!this.stopped) this.port.postMessage({ underrun: l.length - i });
    }
    return !this.stopped;
  }
}

registerProcessor("effectcraft-output", EffectcraftOutput);
