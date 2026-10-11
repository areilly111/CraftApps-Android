// FilmCraft web audio output (AudioWorklet). The UI thread mixes audio ahead and posts
// interleaved stereo Float32Array blocks; this processor plays them in order and reports how many
// frames it actually played (with its currentTime), which drives the playback clock.
class FilmcraftOutput extends AudioWorkletProcessor {
  constructor() {
    super();
    this.queue = [];
    this.offset = 0;
    this.gen = 0;
    this.played = 0;
    this.quanta = 0;
    this.port.onmessage = (e) => {
      const m = e.data;
      if (m.type === "reset") {
        this.gen = m.gen;
        this.queue = [];
        this.offset = 0;
        this.played = 0;
        this.port.postMessage({ gen: this.gen, played: 0, time: currentTime });
      } else if (m.type === "data" && m.gen === this.gen) {
        this.queue.push(m.samples);
      }
    };
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const left = out[0];
    const right = out[1] || out[0];
    const n = left.length;
    let i = 0;
    while (i < n && this.queue.length) {
      const b = this.queue[0];
      const k = Math.min(b.length / 2 - this.offset, n - i);
      for (let j = 0; j < k; j++) {
        left[i + j] = b[2 * (this.offset + j)];
        right[i + j] = b[2 * (this.offset + j) + 1];
      }
      i += k;
      this.offset += k;
      if (this.offset * 2 >= b.length) {
        this.queue.shift();
        this.offset = 0;
      }
    }
    // only real frames advance the clock; a starved queue plays silence and holds it
    this.played += i;
    for (; i < n; i++) {
      left[i] = 0;
      right[i] = 0;
    }
    if (++this.quanta % 4 === 0) {
      this.port.postMessage({ gen: this.gen, played: this.played, time: currentTime });
    }
    return true;
  }
}

registerProcessor("filmcraft-output", FilmcraftOutput);
