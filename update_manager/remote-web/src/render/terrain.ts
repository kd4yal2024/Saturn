import { SpectrumHistory, MISSING_LEVEL } from '../dsp/spectrum-history';
import { normalizeTerrain, type TerrainSettings } from '../settings/terrain';

const VERTEX = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D levels;
uniform int head, width, columns, rows, capacity, count;
uniform float floorDb, ceilingDb, exaggeration, elevation, smoothing;
uniform bool terrain, skirt;
out float level;
out float valid;
float sampleLevel(int age, int col) {
  if(age >= count) return -1000.0;
  int physical = (head - age + capacity) % capacity;
  int lo = col * width / columns;
  int hi = max(lo + 1, (col + 1) * width / columns);
  float value = -1000.0;
  for(int i = lo; i < hi; i++) value = max(value, texelFetch(levels, ivec2(i, physical), 0).r);
  if(smoothing > 0.0 && value > -999.0) {
    float a = texelFetch(levels, ivec2(max(0,lo-1),physical),0).r;
    float b = texelFetch(levels, ivec2(min(width-1,hi),physical),0).r;
    value = mix(value, (a+value+b)/3.0, smoothing);
  }
  return value;
}
void main() {
  if(!terrain) {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0, 1); level = 0.0; valid = 1.0; return;
  }
  int col = skirt ? gl_VertexID / 2 : gl_VertexID % columns;
  int age = skirt ? 0 : gl_VertexID / columns;
  float z = float(age) / float(rows-1);
  level = sampleLevel(age, col);
  valid = level > -999.0 ? 1.0 : 0.0;
  float h = clamp((level-floorDb)/(ceilingDb-floorDb),0.0,1.0);
  float w = 1.0 + 0.18*z;
  // Front/seam z=0 spans the complete frequency ruler. Older rows recede.
  float x = float(col)/float(columns-1)*2.0-1.0;
  if(skirt && (gl_VertexID % 2) == 0) h = 0.0;
  float y = -1.0 + z*(0.5+elevation/100.0) + h*exaggeration*1.25;
  gl_Position = vec4(x, y, z*0.8, w);
}`;
const FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform sampler2D levels, palette;
uniform int head, width, capacity, count;
uniform float floorDb, ceilingDb, gammaValue;
uniform vec2 viewport;
uniform bool terrain;
in float level;
in float valid;
out vec4 color;
void main() {
  float db = level;
  if(terrain) { if(valid < 0.999) discard; }
  else {
    // A pixel covers an explicit time interval. Peak reduction over every
    // intersected bucket preserves one-row impulses when history is compressed.
    // Any missing bucket marks that pixel unknown; never interpolate over gaps.
    int ageLo = max(0, int(floor((1.0-(gl_FragCoord.y+0.5)/viewport.y)*float(capacity))));
    int ageHi = min(count, max(ageLo+1, int(ceil((1.0-(gl_FragCoord.y-0.5)/viewport.y)*float(capacity)))));
    if(ageLo >= count) { color=vec4(0.004,0.012,0.047,1); return; }
    int lo = int(floor(gl_FragCoord.x-0.5)*float(width)/viewport.x);
    int hi = max(lo+1,int(floor(gl_FragCoord.x+0.5)*float(width)/viewport.x));
    db = -1000.0;
    bool missing = false;
    for(int age=ageLo;age<ageHi;age++) {
      int physical = (head-age+capacity)%capacity;
      for(int i=lo;i<min(width,hi);i++) {
        float sampleDb = texelFetch(levels,ivec2(i,physical),0).r;
        missing = missing || sampleDb < -999.0;
        db = max(db,sampleDb);
      }
    }
    if(missing || db < -999.0) { color=vec4(0.08,0.085,0.10,1); return; }
  }
  float t = pow(clamp((db-floorDb)/(ceilingDb-floorDb),0.0,1.0),gammaValue);
  color = texture(palette,vec2((t*1023.0+0.5)/1024.0,0.5));
}`;

type Tier = 'performance' | 'balanced' | 'high';
const TIERS = {
  performance: { columns: 512, rows: 48, dpr: 1, fps: 30, pixels: 1_500_000 },
  balanced: { columns: 1024, rows: 128, dpr: 1.5, fps: 60, pixels: 3_000_000 },
  high: { columns: 2048, rows: 256, dpr: 2, fps: 60, pixels: 5_000_000 },
};
/** Shared R32F cache. Float nearest sampling needs neither float filtering nor float render targets. */
export class TerrainRenderer {
  readonly gl: WebGL2RenderingContext;
  private program!: WebGLProgram; private texture!: WebGLTexture; private lut!: WebGLTexture;
  private indices!: WebGLBuffer; private vao!: WebGLVertexArrayObject;
  private locations = new Map<string, WebGLUniformLocation | null>();
  private uploaded = new Float64Array(512); private textureWidth = 0; private indexCount = 0;
  private paletteKey = ''; private maxDimension: number; private settings = normalizeTerrain(null);
  private indexBytes = 0; private lastDraw = -Infinity; private active = true; private disposed = false;
  private tier: Tier = 'balanced'; private lastAdjust = 0; private slow = 0;
  private recoveryWait = 20000; private recoveryTrial = false;
  private samples: number[] = []; private intervals: number[] = []; private draws = 0;
  columns = 0; rows = 0; skipped = 0; missedRenderDeadlines = 0; reason = ''; uploadRows = 0;
  private lost = (event: Event) => {
    event.preventDefault(); this.active = false; this.onFailure('3D context lost; Traditional is available. Select 3D to retry.');
  };
  constructor(readonly canvas: HTMLCanvasElement, readonly history: SpectrumHistory,
    private color: (t: number, palette: string) => number[], private onFailure: (reason: string) => void) {
    const gl = canvas.getContext('webgl2', { alpha: false, antialias: false, depth: true });
    if (!gl) throw new Error('WebGL2 unavailable');
    this.gl = gl;
    this.maxDimension = Math.min(gl.getParameter(gl.MAX_TEXTURE_SIZE), gl.getParameter(gl.MAX_RENDERBUFFER_SIZE));
    if (this.maxDimension < 1024 || gl.getParameter(gl.MAX_VERTEX_TEXTURE_IMAGE_UNITS) < 1 ||
      !gl.getShaderPrecisionFormat(gl.VERTEX_SHADER, gl.HIGH_FLOAT)?.precision) throw new Error('Insufficient WebGL2 texture/precision limits');
    try { this.setup(); } catch (error) { this.dispose(); throw error; }
    canvas.addEventListener('webglcontextlost', this.lost);
  }
  private setup(): void {
    const gl = this.gl, shaders: WebGLShader[] = [];
    try {
      for (const [kind, source] of [[gl.VERTEX_SHADER, VERTEX], [gl.FRAGMENT_SHADER, FRAGMENT]] as const) {
        const shader = gl.createShader(kind)!; shaders.push(shader); gl.shaderSource(shader, source); gl.compileShader(shader);
        if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) || '3D shader failed');
      }
      this.program = gl.createProgram()!;
      shaders.forEach(shader => gl.attachShader(this.program, shader)); gl.linkProgram(this.program);
      if (!gl.getProgramParameter(this.program, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(this.program) || '3D link failed');
    } finally { shaders.forEach(shader => gl.deleteShader(shader)); }
    this.texture = gl.createTexture()!; this.lut = gl.createTexture()!;
    this.indices = gl.createBuffer()!; this.vao = gl.createVertexArray()!;
    gl.bindVertexArray(this.vao); gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.indices);
    for (const texture of [this.texture, this.lut]) {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, texture === this.lut ? gl.LINEAR : gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, texture === this.lut ? gl.LINEAR : gl.NEAREST);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    }
    gl.useProgram(this.program);
    gl.uniform1i(this.location('levels'), 0); gl.uniform1i(this.location('palette'), 1);
  }
  private location(name: string): WebGLUniformLocation | null {
    if (!this.locations.has(name)) this.locations.set(name, this.gl.getUniformLocation(this.program, name));
    return this.locations.get(name)!;
  }
  configure(settings: TerrainSettings): void {
    this.settings = normalizeTerrain(settings);
    if (settings.quality !== 'auto') this.tier = settings.quality;
  }
  setActive(active: boolean): void { this.active = active; this.lastDraw = -Infinity; }
  private topology(): void {
    const tier = TIERS[this.tier];
    const columns = Math.max(2, Math.min(tier.columns, this.history.width || 2));
    const rows = Math.min(tier.rows, this.settings.depth);
    if (columns === this.columns && rows === this.rows) return;
    this.columns = columns; this.rows = rows;
    const data = new Uint32Array((rows - 1) * (columns * 2 + 1));
    let offset = 0;
    for (let age = 0; age < rows - 1; age++) {
      for (let col = 0; col < columns; col++) {
        data[offset++] = age * columns + col; data[offset++] = (age + 1) * columns + col;
      }
      data[offset++] = 0xffffffff; // fixed restart: never connect unrelated strips
    }
    const gl = this.gl;
    gl.bindBuffer(gl.ELEMENT_ARRAY_BUFFER, this.indices); gl.bufferData(gl.ELEMENT_ARRAY_BUFFER, data, gl.STATIC_DRAW);
    this.indexCount = data.length; this.indexBytes = data.byteLength;
  }
  render(now: number, upperCssHeight: number, force = false): boolean {
    if (this.disposed || !this.active || document.hidden) return false;
    const tier = TIERS[this.tier];
    if (!force && now - this.lastDraw < 1000 / tier.fps - 1) { this.skipped++; return false; }
    const started = performance.now(), history = this.history, gl = this.gl;
    if (!history.width || history.head < 0) { gl.clearColor(.004,.012,.047,1); gl.clear(gl.COLOR_BUFFER_BIT); return true; }
    if (history.width > this.maxDimension) throw new Error(`Source ${history.width} bins exceeds GPU texture limit ${this.maxDimension}`);
    const rect = this.canvas.getBoundingClientRect();
    let scale = Math.min(window.devicePixelRatio || 1, tier.dpr);
    const pixelBudget = Math.min(tier.pixels, Math.max(4, (96 * 1024 * 1024 - history.bytes - history.data.byteLength - 5 * 1024 * 1024) / 12));
    scale = Math.min(scale, Math.sqrt(pixelBudget / Math.max(1, rect.width * rect.height)), this.maxDimension / Math.max(rect.width, rect.height, 1));
    const w = Math.max(2, Math.round(rect.width * scale)), h = Math.max(2, Math.round(rect.height * scale));
    if (this.canvas.width !== w || this.canvas.height !== h) { this.canvas.width = w; this.canvas.height = h; }
    gl.bindVertexArray(this.vao); gl.useProgram(this.program); this.topology();
    gl.activeTexture(gl.TEXTURE0); gl.bindTexture(gl.TEXTURE_2D, this.texture);
    if (this.textureWidth !== history.width) {
      this.textureWidth = history.width;
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.R32F, history.width, history.capacity, 0, gl.RED, gl.FLOAT, history.data);
      this.uploaded.set(history.versions); this.uploadRows += history.capacity;
      if (gl.getError() !== gl.NO_ERROR) throw new Error('Numerical history allocation failed');
    } else {
      for (let p = 0; p < history.capacity; p++) if (this.uploaded[p] !== history.versions[p]) {
        gl.texSubImage2D(gl.TEXTURE_2D, 0, 0, p, history.width, 1, gl.RED, gl.FLOAT, history.data.subarray(p * history.width, (p + 1) * history.width));
        this.uploaded[p] = history.versions[p]!; this.uploadRows++;
      }
    }
    gl.activeTexture(gl.TEXTURE1); gl.bindTexture(gl.TEXTURE_2D, this.lut);
    if (this.paletteKey !== this.settings.palette) {
      const bytes = new Uint8Array(4096);
      for (let i = 0; i < 1024; i++) { bytes.set(this.color(i / 1023, this.settings.palette), i * 4); bytes[i * 4 + 3] = 255; }
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA8, 1024, 1, 0, gl.RGBA, gl.UNSIGNED_BYTE, bytes);
      this.paletteKey = this.settings.palette;
    }
    const ints = { head: history.head, width: history.width, capacity: history.capacity, count: history.count, columns: this.columns, rows: this.rows };
    for (const [key, value] of Object.entries(ints)) gl.uniform1i(this.location(key), value);
    const floats = { floorDb: this.settings.floor, ceilingDb: this.settings.ceiling, gammaValue: this.settings.gamma,
      exaggeration: this.settings.height, elevation: this.settings.elevation, smoothing: this.settings.smoothing };
    for (const [key, value] of Object.entries(floats)) gl.uniform1f(this.location(key), value);
    gl.disable(gl.SCISSOR_TEST); gl.clearColor(.004,.012,.047,1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    const upper = Math.max(1, Math.min(h - 1, Math.round(upperCssHeight / Math.max(1, rect.height) * h))), lower = h - upper;
    gl.viewport(0, 0, w, lower); gl.disable(gl.DEPTH_TEST);
    gl.uniform1i(this.location('terrain'), 0); gl.uniform2f(this.location('viewport'), w, lower);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.viewport(0, lower, w, upper); gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL);
    gl.uniform1i(this.location('terrain'), 1);
    gl.uniform1i(this.location('skirt'), 0);
    gl.drawElements(gl.TRIANGLE_STRIP, this.indexCount, gl.UNSIGNED_INT, 0);
    gl.uniform1i(this.location('skirt'), 1);
    gl.drawArrays(gl.TRIANGLE_STRIP, 0, this.columns * 2);
    const elapsed = performance.now() - started;
    this.samples.push(elapsed); if (this.samples.length > 240) this.samples.shift();
    if (Number.isFinite(this.lastDraw)) { this.intervals.push(now - this.lastDraw); if (this.intervals.length > 240) this.intervals.shift(); }
    this.draws++;
    // Hysteresis: demote on sustained slow frames; cautiously recover after 20s.
    const gap = now - this.lastDraw;
    if (Number.isFinite(this.lastDraw) && gap < 1000) this.missedRenderDeadlines += Math.max(0, Math.floor(gap / (1000 / tier.fps)) - 1);
    this.slow = (elapsed > 16 || (gap > 45 && gap < 500)) ? this.slow + 1 : Math.max(0, this.slow - 1);
    if (this.settings.quality === 'auto' && now - this.lastAdjust > 5000 && this.slow > 20 && this.tier !== 'performance') {
      this.tier = this.tier === 'high' ? 'balanced' : 'performance'; this.lastAdjust = now; this.slow = 0;
      if (this.recoveryTrial) this.recoveryWait = Math.min(300000, this.recoveryWait * 2);
      this.recoveryTrial = false; this.reason = 'Auto reduced geometry / pixels / refresh after sustained slow frames';
    } else if (this.settings.quality === 'auto' && now - this.lastAdjust > this.recoveryWait && this.slow === 0 && this.tier === 'performance' && elapsed < 8) {
      this.tier = 'balanced'; this.lastAdjust = now; this.recoveryTrial = true; this.reason = `Auto recovery trial after ${this.recoveryWait / 1000} seconds`;
    }
    this.lastDraw = now; return true;
  }
  /** CPU projection exactly matches the vertex shader; used for ray/triangle picking. */
  project(age: number, col: number): [number, number, number] {
    const z = age / Math.max(1, this.rows - 1), w = 1 + .18 * z;
    const db = this.history.peak(age, col, this.columns, this.settings.smoothing);
    const h = Math.max(0, Math.min(1, (db - this.settings.floor) / (this.settings.ceiling - this.settings.floor)));
    return [((col / (this.columns - 1) * 2 - 1) / w + 1) / 2,
      (1 - (-1 + z * (.5 + this.settings.elevation / 100) + h * this.settings.height * 1.25) / w) / 2, z * .8 / w];
  }
  pick(x: number, y: number): { fraction: number; age: number; db: number } | null {
    let best: { fraction: number; age: number; db: number } | null = null, depth = Infinity;
    // Each age strip only spans a few candidate columns at x. No full mesh scan.
    for (let age = 0; age < Math.min(this.rows - 1, this.history.count - 1); age++) {
      if (!this.history.row(age) || !this.history.row(age + 1)) continue;
      const colAt = (a: number) => ((x * 2 - 1) * (1 + .18 * a / (this.rows - 1)) + 1) / 2 * (this.columns - 1);
      const lo = Math.max(0, Math.floor(Math.min(colAt(age), colAt(age + 1))) - 1);
      const hi = Math.min(this.columns - 2, Math.ceil(Math.max(colAt(age), colAt(age + 1))));
      for (let col = lo; col <= hi; col++) for (const vertices of [ [[age,col],[age+1,col],[age,col+1]], [[age,col+1],[age+1,col],[age+1,col+1]] ]) {
        const points = vertices.map(v => this.project(v[0]!, v[1]!));
        const a=points[0]!, b=points[1]!, c=points[2]!;
        const det = (b[1]!-c[1]!)*(a[0]!-c[0]!) + (c[0]!-b[0]!)*(a[1]!-c[1]!);
        if (Math.abs(det) < 1e-10) continue;
        const u = ((b[1]!-c[1]!)*(x-c[0]!) + (c[0]!-b[0]!)*(y-c[1]!))/det;
        const v = ((c[1]!-a[1]!)*(x-c[0]!) + (a[0]!-c[0]!)*(y-c[1]!))/det, weights=[u,v,1-u-v];
        if (weights.some(t => t < -1e-6)) continue;
        const d = weights.reduce((s,t,i) => s+t*points[i]![2],0);
        if (d >= depth) continue;
        // Perspective-correct interpolation of frequency and age.
        const corrected = weights.map((t,i) => t/(1+.18*vertices[i]![0]!/(this.rows-1)));
        const sum = corrected.reduce((a,b)=>a+b,0);
        const fraction = corrected.reduce((s,t,i)=>s+t*vertices[i]![1]!/(this.columns-1),0)/sum;
        const rowAge = Math.round(corrected.reduce((s,t,i)=>s+t*vertices[i]![0]!,0)/sum);
        const raw = this.history.row(rowAge);
        if (!raw) continue;
        best = { fraction, age: rowAge, db: raw[Math.min(raw.length-1,Math.max(0,Math.round(fraction*(raw.length-1))))]! }; depth=d;
      }
    }
    if (!best && this.history.row(0) && x >= 0 && x <= 1) {
      const col = Math.min(this.columns - 2, Math.floor(x * (this.columns - 1)));
      const a = this.project(0, col), b = this.project(0, col + 1);
      const top = a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0]);
      if (y >= top && y <= 1) {
        const raw = this.history.row(0)!;
        best = { fraction: x, age: 0, db: raw[Math.round(x * (raw.length - 1))]! };
      }
    }
    return best;
  }
  diagnostics(): Record<string, unknown> {
    const sorted = [...this.samples].sort((a,b)=>a-b), intervals=[...this.intervals].sort((a,b)=>a-b);
    const p = (a:number[], q:number) => a[Math.min(a.length-1,Math.floor(a.length*q))] ?? 0;
    return { quality: this.tier, requestedQuality: this.settings.quality, targetFps: TIERS[this.tier].fps,
      backingStore: `${this.canvas.width} × ${this.canvas.height}`, maxTextureDimension: this.maxDimension,
      recentSeconds: Math.max(0, Math.min(this.rows, this.history.count) - 1) * this.history.cadenceMs / 1000,
      historyEpoch: this.history.epoch, historyBoundary: this.history.boundary, reason: this.reason, sourceBins: this.history.rows[this.history.head]?.sourceBins ?? 0,
      sourceUpdateRateHz: this.history.sourceRateHz, coalescedSourceUpdates: this.history.coalescedSourceUpdates,
      geometry: `${this.columns} × ${this.rows}`, rowIntervalMs: this.history.cadenceMs, retainedSeconds: this.history.retainedMs/1000,
      cpuSubmitP50Ms: p(sorted,.5), cpuSubmitP95Ms: p(sorted,.95), frameP50Ms: p(intervals,.5), frameP95Ms: p(intervals,.95), frameP99Ms: p(intervals,.99),
      fps: this.intervals.length ? 1000*this.intervals.length/this.intervals.reduce((a,b)=>a+b,0) : 0,
      draws: this.draws, missedRenderDeadlines: this.missedRenderDeadlines, skippedRenderCallbacks: this.skipped, aggregated: this.history.aggregated, missingRows: this.history.missing,
      bytes: this.history.bytes + this.history.data.byteLength + this.uploaded.byteLength + this.indexBytes + 4096 + this.canvas.width*this.canvas.height*12,
      memoryNote: 'CPU history + GPU levels + indices + LUT + estimated double color/depth; excludes driver and Traditional caches',
      uploadRows: this.uploadRows };
  }
  dispose(): void {
    if (this.disposed) return; this.disposed = true; this.active = false;
    this.canvas.removeEventListener('webglcontextlost', this.lost);
    const gl=this.gl;
    gl.deleteTexture(this.texture); gl.deleteTexture(this.lut); gl.deleteBuffer(this.indices); gl.deleteVertexArray(this.vao); gl.deleteProgram(this.program);
    // Detached canvas relinquishes its context too; the numerical ring survives.
    gl.getExtension('WEBGL_lose_context')?.loseContext();
  }
}
