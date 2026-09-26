import { cleanupNormalized, CLEANUP_GLSL, WATERFALL_CLEANUP_GLSL } from '../dsp/display-cleanup';
import { SpectrumHistory, MISSING_LEVEL, levelStatistics } from '../dsp/spectrum-history';
import { normalizeTerrain, type TerrainSettings } from '../settings/terrain';

/**
 * Far-field definition controls for the projected surface.
 *
 * The mesh spreads `rows` rows over the upper pane. When the pane is short (or
 * quality/depth ask for more rows than there are device pixels) rows collapse
 * into sub-pixel strips, which is what makes the far/old edge of the surface
 * shimmer and smear. MIN_TERRAIN_ROW_PIXELS bounds that, and the mesh
 * decimates its age window with a peak-preserving maximum so decimation cannot
 * drop a brief signal. Far-field width (the `perspective` setting, default
 * 0.10) replaces the previous hard-coded 0.18 horizontal compression toward
 * the far edge. Definition is geometry only: color must never vary with depth
 * (see scripts/terrain-controlled.mjs).
 */
const MIN_TERRAIN_ROW_PIXELS = 1.25;

const VERTEX = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D levels;
uniform int head, width, columns, rows, capacity, count, span;
uniform float floorDb, ceilingDb, exaggeration, elevation, smoothing, perspective;
uniform bool terrain, outline;
out float level;
out float valid;
${CLEANUP_GLSL}
float sampleLevelAt(int age, int col) {
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
// Peak-preserving maximum across the history rows this mesh row owns, so
// decimating a short pane cannot drop a brief signal.
float sampleLevelRange(int ageLo, int ageHi, int col) {
  float value = -1000.0;
  for(int age = ageLo; age < ageHi; age++) {
    if(age >= count) break;
    value = max(value, sampleLevelAt(age, col));
  }
  return value;
}
void main() {
  if(!terrain) {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(p * 2.0 - 1.0, 0, 1);
    level = 0.0; valid = 1.0; return;
  }
  int col = gl_VertexID % columns;
  int meshRow = outline ? 0 : gl_VertexID / columns;
  float z = float(meshRow) / float(max(1, rows-1));
  // Keep the leading edge at the newest measured row. Spread all older rows
  // across the remaining mesh rows without dropping a short-lived peak.
  int ageLo = meshRow, ageHi = meshRow + 1;
  if(!outline && span >= rows && meshRow > 0) {
    ageLo = 1 + ((span - 1) * (meshRow - 1)) / (rows - 1);
    ageHi = max(ageLo + 1, 1 + ((span - 1) * meshRow) / (rows - 1));
  }
  ageLo = min(ageLo, count);
  ageHi = min(ageHi, count);
  level = sampleLevelRange(ageLo, ageHi, col);
  valid = level > -999.0 ? 1.0 : 0.0;
  float h = displayNormalized(level);
  float w = 1.0 + perspective*z;
  // Front/seam z=0 spans the complete frequency ruler. Older rows recede.
  float x = float(col)/float(columns-1)*2.0-1.0;
  float y = -1.0 + z*(0.5+elevation/100.0) + h*exaggeration*.75;
  gl_Position = vec4(x, y, z*0.8, w);
}`;
const FRAGMENT = `#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D levels;
uniform lowp sampler2D palette;
uniform int head, width, capacity, count, waterfallRowPixels;
uniform float floorDb, ceilingDb, gammaValue;
uniform vec2 viewport;
uniform bool terrain;
in float level;
in float valid;
out vec4 color;
${CLEANUP_GLSL}
${WATERFALL_CLEANUP_GLSL}
void main() {
  float db = level;
  if(terrain) { if(valid < 0.999) discard; }
  else {
    // Top pixel is newest. Every measured row owns an integer-height stripe,
    // so its raster thickness cannot alternate as it descends. Fractionally
    // stretching all 512 rows made horizontal edges shimmer at common DPRs.
    // Older retained rows outside the viewport are intentionally not compressed.
    int top = int(viewport.y) - 1 - int(gl_FragCoord.y);
    int ageLo = top / waterfallRowPixels;
    int ageHi = min(count, ageLo+1);
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
  float t = pow(terrain ? displayNormalized(db) : waterfallNormalized(db),gammaValue);
  // Color is depth-independent by contract: a constant surface must render one
  // color at every age (validated by scripts/terrain-controlled.mjs).
  color = texture(palette,vec2((t*1023.0+0.5)/1024.0,0.5));
}`;

type Tier = 'performance' | 'balanced' | 'high';
const TIERS = {
  performance: { columns: 512, rows: 48, fps: 30 },
  balanced: { columns: 1024, rows: 128, fps: 60 },
  high: { columns: 2048, rows: 256, fps: 60 },
};

export function waterfallRowLayout(height: number, capacity: number): { rowPixels: number; visibleRows: number } {
  const pixels = Math.max(1, Math.floor(Number.isFinite(height) ? height : 1));
  const retained = Math.max(1, Math.floor(Number.isFinite(capacity) ? capacity : 1));
  const rowPixels = Math.max(1, Math.ceil(pixels / retained));
  return { rowPixels, visibleRows: Math.min(retained, Math.ceil(pixels / rowPixels)) };
}

/**
 * Mesh row budget for the projected surface.
 *
 * Quality and the depth control can ask for more rows than the upper pane has
 * device pixels, which collapses the far/old edge into sub-pixel strips. Cap
 * the mesh at the number of rows the pane can actually resolve; the vertex
 * shader then covers the requested history span with a peak-preserving age
 * window instead of drawing sub-pixel shells.
 */
export function terrainRowBudget(
  upperPixels: number,
  tierRows: number,
  depth: number,
): { rows: number; rowPixels: number; resolvable: number } {
  const pixels = Math.max(2, Number.isFinite(upperPixels) ? upperPixels : 2);
  const resolvable = Math.max(2, Math.floor(pixels / MIN_TERRAIN_ROW_PIXELS));
  const rows = Math.max(2, Math.min(tierRows, depth, resolvable));
  return { rows, rowPixels: pixels / Math.max(1, rows - 1), resolvable };
}

/** Source history rows represented by one projected mesh row. */
export function terrainMeshAgeRange(meshRow: number, rows: number, span: number): { start: number; end: number } {
  const row = Math.max(0, Math.floor(meshRow));
  const meshRows = Math.max(2, Math.floor(rows));
  const visibleSpan = Math.max(1, Math.floor(span));
  if (row === 0) return { start: 0, end: 1 };
  if (visibleSpan < meshRows) return { start: Math.min(row, visibleSpan), end: Math.min(visibleSpan, row + 1) };
  const start = 1 + Math.floor((visibleSpan - 1) * (row - 1) / (meshRows - 1));
  const end = Math.max(start + 1, 1 + Math.floor((visibleSpan - 1) * row / (meshRows - 1)));
  return { start, end: Math.min(visibleSpan, end) };
}
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
  private resolutionReason = '';
  private recoveryWait = 20000; private recoveryTrial = false;
  private samples: number[] = []; private intervals: number[] = []; private draws = 0;
  private waterfallRowPixels = 1; private waterfallVisibleRows = 0; private waterfallPixelHeight = 0;
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
  private span = 0;
  /** Device pixels each projected surface row currently owns. */
  private surfaceRowPixels = 0;
  private topology(upperPixels: number): void {
    const tier = TIERS[this.tier];
    const columns = Math.max(2, Math.min(tier.columns, this.history.width || 2));
    // Never ask for more mesh rows than the upper pane can resolve; otherwise
    // rows become sub-pixel strips and the far edge shimmers. The age window
    // keeps the requested history span even when rows are decimated.
    const budget = terrainRowBudget(upperPixels, tier.rows, this.settings.depth);
    const rows = budget.rows;
    this.surfaceRowPixels = budget.rowPixels;
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
    // Keep the operating waterfall at device resolution in every tier. Quality
    // limits terrain topology and refresh; it must not soften the entire canvas.
    const requestedScale = window.devicePixelRatio || 1;
    let scale = requestedScale;
    const pixelBudget = Math.min(8_000_000, Math.max(4, (96 * 1024 * 1024 - history.bytes - history.data.byteLength - 5 * 1024 * 1024) / 12));
    scale = Math.min(scale, Math.sqrt(pixelBudget / Math.max(1, rect.width * rect.height)), this.maxDimension / Math.max(rect.width, rect.height, 1));
    this.resolutionReason = scale < requestedScale ? 'Backing resolution limited by 96 MiB view budget / 8M pixels / GPU dimensions' : '';
    const w = Math.max(2, Math.round(rect.width * scale)), h = Math.max(2, Math.round(rect.height * scale));
    if (this.canvas.width !== w || this.canvas.height !== h) { this.canvas.width = w; this.canvas.height = h; }
    // The upper pane height is needed by the mesh row budget below.
    const upper = Math.max(1, Math.min(h - 1, Math.round(upperCssHeight / Math.max(1, rect.height) * h)));
    gl.bindVertexArray(this.vao); gl.useProgram(this.program); this.topology(upper);
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
    const lower = h - upper;
    const waterfall = waterfallRowLayout(lower, history.capacity);
    this.waterfallRowPixels = waterfall.rowPixels; this.waterfallVisibleRows = waterfall.visibleRows; this.waterfallPixelHeight = lower;
    // Recent history actually on screen, bounded by the depth control.
    this.span = Math.max(1, Math.min(history.count, this.settings.depth));
    const ints = { head: history.head, width: history.width, capacity: history.capacity, count: history.count,
      columns: this.columns, rows: this.rows, span: this.span, waterfallRowPixels: waterfall.rowPixels };
    for (const [key, value] of Object.entries(ints)) gl.uniform1i(this.location(key), value);
    const floats = { floorDb: this.settings.floor, ceilingDb: this.settings.ceiling, gammaValue: this.settings.gamma,
      noiseFloor: this.noiseBaseline, cleanupStrength: this.settings.cleanup,
      waterfallCleanupStrength: this.settings.waterfallCleanup,
      exaggeration: this.settings.height, elevation: this.settings.elevation, smoothing: this.settings.smoothing,
      perspective: this.settings.perspective };
    for (const [key, value] of Object.entries(floats)) gl.uniform1f(this.location(key), value);
    gl.disable(gl.SCISSOR_TEST); gl.clearColor(.004,.012,.047,1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
    gl.viewport(0, 0, w, lower); gl.disable(gl.DEPTH_TEST);
    gl.uniform1i(this.location('terrain'), 0); gl.uniform2f(this.location('viewport'), w, lower);
    gl.drawArrays(gl.TRIANGLES, 0, 3);
    gl.viewport(0, lower, w, upper); gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL);
    gl.uniform1i(this.location('terrain'), 1);
    gl.uniform1i(this.location('outline'), 0);
    gl.drawElements(gl.TRIANGLE_STRIP, this.indexCount, gl.UNSIGNED_INT, 0);
    gl.uniform1i(this.location('outline'), 1);
    // Only the measured leading edge; no opaque wall to the baseline.
    gl.drawArrays(gl.LINE_STRIP, 0, this.columns);
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
      this.recoveryTrial = false; this.reason = 'Auto reduced geometry / refresh after sustained slow frames';
    } else if (this.settings.quality === 'auto' && now - this.lastAdjust > this.recoveryWait && this.slow === 0 && this.tier === 'performance' && elapsed < 8) {
      this.tier = 'balanced'; this.lastAdjust = now; this.recoveryTrial = true; this.reason = `Auto recovery trial after ${this.recoveryWait / 1000} seconds`;
    }
    this.lastDraw = now; return true;
  }
  get noiseBaseline(): number { return this.settings.cleanupBaseline ?? this.history.noiseFloor ?? this.settings.floor; }
  get maxHistoryWidth(): number { return this.maxDimension; }
  waterfallAgeAt(fraction: number): number {
    const pixel = Math.min(Math.max(0, this.waterfallPixelHeight - 1),
      Math.floor(Math.max(0, Math.min(1, fraction)) * this.waterfallPixelHeight));
    return Math.min(Math.max(0, this.waterfallVisibleRows - 1), Math.floor(pixel / this.waterfallRowPixels));
  }
  private meshPeak(meshRow: number, col: number): number {
    const { start, end } = terrainMeshAgeRange(meshRow, this.rows, this.span);
    let peak = MISSING_LEVEL;
    for (let age = start; age < end; age++) {
      peak = Math.max(peak, this.history.peak(age, col, this.columns, this.settings.smoothing));
    }
    return peak;
  }
  /** CPU projection matches the displayed mesh; used for ray/triangle picking. */
  project(meshRow: number, col: number): [number, number, number] {
    const z = meshRow / Math.max(1, this.rows - 1), w = 1 + this.settings.perspective * z;
    const db = this.meshPeak(meshRow, col);
    const h = cleanupNormalized(db, this.settings.floor, this.settings.ceiling, this.noiseBaseline, this.settings.cleanup);
    return [((col / (this.columns - 1) * 2 - 1) / w + 1) / 2,
      (1 - (-1 + z * (.5 + this.settings.elevation / 100) + h * this.settings.height * .75) / w) / 2, z * .8 / w];
  }
  pick(x: number, y: number): { fraction: number; age: number; db: number } | null {
    let best: { fraction: number; age: number; db: number } | null = null, depth = Infinity;
    // Each age strip only spans a few candidate columns at x. No full mesh scan.
    for (let age = 0; age < Math.min(this.rows - 1, this.span - 1); age++) {
      const colAt = (a: number) => ((x * 2 - 1) * (1 + this.settings.perspective * a / (this.rows - 1)) + 1) / 2 * (this.columns - 1);
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
        const corrected = weights.map((t,i) => t/(1+this.settings.perspective*vertices[i]![0]!/(this.rows-1)));
        const sum = corrected.reduce((a,b)=>a+b,0);
        const fraction = corrected.reduce((s,t,i)=>s+t*vertices[i]![1]!/(this.columns-1),0)/sum;
        const meshRow = Math.round(corrected.reduce((s,t,i)=>s+t*vertices[i]![0]!,0)/sum);
        const range = terrainMeshAgeRange(meshRow, this.rows, this.span);
        const bin = Math.min(this.history.width - 1, Math.max(0, Math.round(fraction * (this.history.width - 1))));
        let sourceAge = -1, db = MISSING_LEVEL;
        for (let source = range.start; source < range.end; source++) {
          const raw = this.history.row(source);
          if (raw && raw[bin]! > db) { sourceAge = source; db = raw[bin]!; }
        }
        if (sourceAge < 0) continue;
        best = { fraction, age: sourceAge, db }; depth=d;
      }
    }
    return best;
  }
  diagnostics(): Record<string, unknown> {
    const sorted = [...this.samples].sort((a,b)=>a-b), intervals=[...this.intervals].sort((a,b)=>a-b);
    const p = (a:number[], q:number) => a[Math.min(a.length-1,Math.floor(a.length*q))] ?? 0;
    const mapping = this.history.rows[this.history.head];
    const amplitude = {
      units: mapping?.units ?? 'relative dB',
      source: levelStatistics(this.history.latestRaw, this.settings.floor, this.settings.ceiling),
      storedBucket: levelStatistics(this.history.row(0) ?? new Float32Array(), this.settings.floor, this.settings.ceiling),
      floor: this.settings.floor, ceiling: this.settings.ceiling, colorGamma: this.settings.gamma,
      heightControl: this.settings.height, heightScale: this.settings.height * .75,
      heightOffset: 0, heightMapping: 'clamped dB normalization with optional display-only soft knee; scale is clip-space height',
      cleanupStrength: this.settings.cleanup, cleanupBaseline: this.noiseBaseline,
      waterfallCleanupStrength: this.settings.waterfallCleanup,
      // Far-field definition: mesh rows per pane, the history span they cover,
      // and the framing constants behind the projected surface.
      surfaceMeshRows: this.rows, surfaceMeshColumns: this.columns,
      surfaceHistorySpan: this.span,
      surfaceRowPixels: this.surfaceRowPixels,
      minRowPixels: MIN_TERRAIN_ROW_PIXELS,
      farFieldWidth: this.settings.perspective,
      waterfallMapping: 'pointwise attenuation below held baseline / linear 24 dB shoulder / full levels above shoulder',
      baselineSource: this.settings.cleanupBaseline === null ? 'held first-frame median / explicit re-estimate' : 'manual',
      kneeStartDb: Math.min(this.noiseBaseline+8,this.settings.ceiling)-4, unchangedAboveDb: Math.min(this.noiseBaseline+8,this.settings.ceiling),
      scalarTexture: 'R32F / highp sampler2D / NEAREST texelFetch',
    };
    const rect = this.canvas.getBoundingClientRect();
    return { amplitude, cssSize: `${rect.width} × ${rect.height}`, devicePixelRatio: window.devicePixelRatio,
      effectivePixelRatio: rect.width ? this.canvas.width / rect.width : 0,
      resolutionReason: this.resolutionReason, historyTexture: `${this.history.width} × ${this.history.capacity} R32F`,
      waterfallSampling: 'one history bucket per integer-height pixel stripe / per-pixel bin maxima / no temporal or spatial smoothing',
      waterfallRowPixels: this.waterfallRowPixels, waterfallVisibleRows: this.waterfallVisibleRows,
      waterfallSeconds: Math.max(0, Math.min(this.waterfallVisibleRows, this.history.count) - 1) * this.history.cadenceMs / 1000,
      quality: this.tier, requestedQuality: this.settings.quality, targetFps: TIERS[this.tier].fps,
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
