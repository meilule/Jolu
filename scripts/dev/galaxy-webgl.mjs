// 零依赖 WebGL2 深空星系、连续旋臂星云盘与体积恒星渲染引擎
export const GALAXY_WEBGL_SOURCE = `
function createGalaxyWebGLRenderer(canvas) {
  const gl = canvas.getContext("webgl2", {
    alpha: true,
    antialias: true,
    premultipliedAlpha: false,
    powerPreference: "high-performance",
  });
  if (!gl) return null;

  function compile(type, src) {
    const sh = gl.createShader(type);
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
      console.warn("WebGL shader error:", gl.getShaderInfoLog(sh));
      gl.deleteShader(sh);
      return null;
    }
    return sh;
  }

  function link(vsSrc, fsSrc) {
    const vs = compile(gl.VERTEX_SHADER, vsSrc);
    const fs = compile(gl.FRAGMENT_SHADER, fsSrc);
    if (!vs || !fs) return null;
    const prog = gl.createProgram();
    gl.attachShader(prog, vs);
    gl.attachShader(prog, fs);
    gl.linkProgram(prog);
    if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) {
      console.warn("WebGL link error:", gl.getProgramInfoLog(prog));
      return null;
    }
    return prog;
  }

  const COMMON_PROJ_GLSL = \`
    uniform vec2 uRes;
    uniform vec2 uCenter;
    uniform vec3 uFocus;
    uniform vec3 uCam;   // yaw, pitch, zoom
    uniform float uDpr;
    uniform float uFov;
    uniform float uTime;
    uniform float uDiskSpin;
    uniform float uDark;
    uniform float uSeed;

    vec4 projectGalaxyPoint(vec3 wpos, float followFocus) {
      vec3 d = wpos - uFocus * followFocus;
      float cy = cos(uCam.x), sy = sin(uCam.x);
      float cp = cos(uCam.y), sp = sin(uCam.y);

      float x1 = d.x * cy - d.z * sy;
      float z1 = d.x * sy + d.z * cy;
      float y2 = d.y * cp - z1 * sp;
      float z2 = d.y * sp + z1 * cp;

      float denom = uFov + z2;
      if (denom < 75.0) {
        return vec4(9.0, 9.0, 9.0, -1.0);
      }
      float scale = (uFov / denom) * uCam.z;
      float sx = uCenter.x + x1 * scale;
      float syScr = uCenter.y + y2 * scale;
      float ndcX = (sx / uRes.x) * 2.0 - 1.0;
      float ndcY = 1.0 - (syScr / uRes.y) * 2.0;
      float ndcZ = clamp(z2 / 2400.0, -0.95, 0.95);
      return vec4(ndcX, ndcY, ndcZ, scale);
    }
  \`;

  // ── Pass 1: 连续三维银河盘面 FBM 对数旋臂 + 暗尘埃吸收带（解决粒子稀疏感，打造真星系云雾骨架） ──
  const diskVS = \`#version 300 es
    precision highp float;
    \${COMMON_PROJ_GLSL}
    layout(location = 0) in vec2 aDiskXZ;
    out vec2 vXZ;
    out float vValid;

    void main() {
      // 给外盘增加极轻微的银河翘曲 (Galactic Warp)，立体视角下更像真实盘状星系
      float rNorm = length(aDiskXZ) / 480.0;
      float warpY = sin(atan(aDiskXZ.y, aDiskXZ.x) * 2.0 + rNorm * 1.4) * 9.0 * pow(rNorm, 1.5);
      vec4 p = projectGalaxyPoint(vec3(aDiskXZ.x, warpY, aDiskXZ.y), 1.0);
      if (p.w <= 0.0) {
        gl_Position = vec4(0.0, 0.0, 2.0, 1.0);
        vValid = 0.0;
      } else {
        gl_Position = vec4(p.xyz, 1.0);
        vValid = 1.0;
      }
      vXZ = aDiskXZ;
    }
  \`;

  const diskFS = \`#version 300 es
    precision highp float;
    in vec2 vXZ;
    in float vValid;
    uniform float uTime;
    uniform float uDiskSpin;
    uniform float uDark;
    uniform float uSeed;
    out vec4 outColor;

    float hash21(vec2 p) {
      p = fract(p * vec2(127.1, 311.7));
      p += dot(p, p + 34.23);
      return fract(p.x * p.y);
    }

    float vnoise(vec2 p) {
      vec2 i = floor(p);
      vec2 f = fract(p);
      f = f * f * (3.0 - 2.0 * f);
      return mix(
        mix(hash21(i), hash21(i + vec2(1.0, 0.0)), f.x),
        mix(hash21(i + vec2(0.0, 1.0)), hash21(i + vec2(1.0, 1.0)), f.x),
        f.y
      );
    }

    float fbm(vec2 p) {
      float s = 0.0;
      float a = 0.52;
      mat2 rot = mat2(0.80, -0.60, 0.60, 0.80);
      for (int i = 0; i < 4; i++) {
        s += a * vnoise(p);
        p = rot * p * 2.04 + vec2(13.1, 7.7);
        a *= 0.52;
      }
      return s;
    }

    void main() {
      if (vValid < 0.5) discard;
      float rWorld = length(vXZ);
      float r = rWorld / 365.0;
      if (r > 1.38) discard;

      // 差异自转 + 种子偏置
      float spin = uDiskSpin * (0.28 + 0.34 / (r + 0.35));
      float ca = cos(spin), sa = sin(spin);
      vec2 pos = vec2(vXZ.x * ca - vXZ.y * sa, vXZ.x * sa + vXZ.y * ca) / 145.0;
      vec2 seedOff = vec2(sin(uSeed * 0.71) * 4.2, cos(uSeed * 0.43) * 4.2);

      // 域扭曲 FBM，制造真实电离星云拉丝纹理
      vec2 warp = vec2(
        fbm(pos * 1.15 + seedOff),
        fbm(pos * 1.15 + seedOff + vec2(5.2, 1.3))
      );
      float neb = fbm(pos * 1.9 + warp * 1.45);
      float fine = fbm(pos * 4.2 - warp * 1.8);

      float theta = atan(pos.y, pos.x);
      float logR = log(r + 0.14);

      // 2 条主对数旋臂 + 2 条次级羽翼支臂
      float phase2 = 2.0 * (theta - logR * 2.38) + (warp.x - 0.5) * 1.85;
      float armMajor = pow(max(0.0, 0.5 + 0.5 * cos(phase2)), 1.75);
      float phase4 = 4.0 * (theta - logR * 2.15 + 0.65) + (warp.y - 0.5) * 2.1;
      float armSpur = pow(max(0.0, 0.5 + 0.5 * cos(phase4)), 2.2) * 0.52;

      float radialFade = exp(-r * r * 1.55) * smoothstep(1.36, 0.72, r);
      float bulge = exp(-r * r * 16.5);
      float arms = (armMajor + armSpur) * radialFade;

      // 暗尘埃丝带（深色下在旋臂内侧压出强明暗层次）
      float dustLane = pow(max(0.0, 0.5 + 0.5 * cos(phase2 + 0.85)), 3.2) * smoothstep(0.25, 0.9, neb) * radialFade;

      float cloudMask = clamp(arms * (0.42 + 1.05 * neb) + bulge * 0.92, 0.0, 1.35);
      float knot = smoothstep(0.56, 0.88, fine) * armMajor * radialFade;

      // 多光谱星云设色：深色高光炽辉 vs 浅色清透青瓷/晴空蓝/鸢尾紫水彩星云（R1-08-V1 明净配色，绝不发灰发脏）
      vec3 cCore = uDark > 0.5 ? vec3(0.86, 0.99, 0.90) : vec3(0.08, 0.74, 0.50);
      vec3 cEmerald = uDark > 0.5 ? vec3(0.12, 0.88, 0.55) : vec3(0.10, 0.72, 0.56);
      vec3 cCyan = uDark > 0.5 ? vec3(0.18, 0.58, 0.98) : vec3(0.22, 0.58, 0.96);
      vec3 cRose = uDark > 0.5 ? vec3(0.76, 0.30, 0.82) : vec3(0.56, 0.44, 0.94);

      vec3 gas = mix(cEmerald, cCyan, smoothstep(0.20, 0.88, r + (neb - 0.5) * 0.35));
      gas = mix(gas, cRose, clamp(knot * 1.35, 0.0, 0.65));
      gas = mix(gas, cCore, clamp(bulge * 1.15, 0.0, 1.0));

      if (uDark > 0.5) {
        gas *= (1.0 - dustLane * 0.58);
        float alpha = clamp(cloudMask * 0.56 + bulge * 0.34, 0.0, 0.88) * smoothstep(1.36, 0.88, r);
        outColor = vec4(gas, alpha);
      } else {
        // 浅色下用纯净瓷白负空间切分旋臂丝带，保持水彩矿物图鉴般明快通透
        gas = mix(gas, vec3(0.97, 1.0, 0.985), dustLane * 0.50);
        float alpha = clamp(cloudMask * 0.30 + bulge * 0.20 - dustLane * 0.06, 0.0, 0.40) * smoothstep(1.36, 0.86, r);
        outColor = vec4(gas, alpha);
      }
    }
  \`;

  // ── Pass 2: 11,200 颗三维恒星 & 银核星云粒子 ──
  const starVS = \`#version 300 es
    precision highp float;
    \${COMMON_PROJ_GLSL}

    layout(location = 0) in vec3 aBasePos;
    layout(location = 1) in vec4 aParams; // x:size, y:spinRate, z:phase, w:kind (0=disk, 1=core, 2=sky)
    layout(location = 2) in vec3 aColor;
    layout(location = 3) in float aAlpha;

    out vec3 vColor;
    out float vAlpha;
    out float vFlare;

    void main() {
      vec3 pos = aBasePos;
      float isSky = step(1.5, aParams.w);
      if (isSky < 0.5) {
        float ang = uDiskSpin * aParams.y;
        float ca = cos(ang), sa = sin(ang);
        pos.xz = vec2(pos.x * ca - pos.z * sa, pos.x * sa + pos.z * ca);
      }

      vec4 p = projectGalaxyPoint(pos, 1.0 - isSky * 0.90);
      if (p.w <= 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        gl_PointSize = 0.0;
        return;
      }

      gl_Position = vec4(p.xyz, 1.0);
      float twinkle = 0.72 + 0.28 * sin(uTime * (1.2 + aParams.y * 1.4) + aParams.z);
      float distScale = isSky > 0.5 ? (0.85 + uCam.z * 0.14) : clamp(p.w, 0.36, 2.65);
      gl_PointSize = clamp(aParams.x * distScale * uDpr, 1.0, 44.0 * uDpr);

      vColor = aColor;
      vAlpha = aAlpha * twinkle;
      vFlare = smoothstep(5.4, 13.5, aParams.x);
    }
  \`;

  const starFS = \`#version 300 es
    precision highp float;
    in vec3 vColor;
    in float vAlpha;
    in float vFlare;
    uniform float uDark;
    out vec4 outColor;

    void main() {
      vec2 uv = gl_PointCoord * 2.0 - 1.0;
      float r = length(uv);
      if (r > 1.0) discard;

      float core = exp(-r * r * 26.0);
      float halo = exp(-r * 3.8) * (1.0 - r);
      float spikeH = exp(-abs(uv.y) * 14.0) * exp(-abs(uv.x) * 3.0);
      float spikeV = exp(-abs(uv.x) * 14.0) * exp(-abs(uv.y) * 3.0);
      float flare = (spikeH + spikeV) * 0.72 * vFlare;

      float intensity = core * 1.2 + halo * 0.62 + flare;
      vec3 col = mix(vColor, vec3(1.0), core * (uDark > 0.5 ? 0.72 : 0.10));
      float a = clamp(intensity * vAlpha, 0.0, 1.0);
      if (uDark < 0.5) {
        // 浅色下将近白核心星映射为清透翡翠/晴空蓝星芒，彩色旋臂星提纯宝石饱和度，彻底告别浅底灰点
        float luma = dot(vColor, vec3(0.299, 0.587, 0.114));
        vec3 chroma = vColor - vec3(luma);
        float sat = length(chroma);
        vec3 jewelFallback = mix(vec3(0.08, 0.68, 0.48), vec3(0.20, 0.56, 0.92), clamp(vColor.b - vColor.r + 0.5, 0.0, 1.0));
        vec3 saturatedStar = clamp(vec3(luma * 0.62) + chroma * 1.65, 0.05, 0.95);
        col = mix(jewelFallback, saturatedStar, smoothstep(0.08, 0.28, sat));
        col = mix(col, vec3(1.0), core * vFlare * 0.45);
        a = clamp(intensity * vAlpha * 0.58, 0.0, 0.64);
      }
      outColor = vec4(col, a);
    }
  \`;

  // ── Pass 3: 三维立体起伏星云团块（让侧面 3D 旋转也有垂直云团层次） ──
  const nebulaVS = \`#version 300 es
    precision highp float;
    \${COMMON_PROJ_GLSL}

    layout(location = 0) in vec3 aBasePos;
    layout(location = 1) in vec4 aParams;
    layout(location = 2) in vec3 aColor;
    layout(location = 3) in float aAlpha;

    out vec3 vColor;
    out float vAlpha;
    out float vSeed;
    out float vRot;

    void main() {
      vec3 pos = aBasePos;
      float ang = uDiskSpin * aParams.y;
      float ca = cos(ang), sa = sin(ang);
      pos.xz = vec2(pos.x * ca - pos.z * sa, pos.x * sa + pos.z * ca);

      vec4 p = projectGalaxyPoint(pos, 1.0);
      if (p.w <= 0.0) {
        gl_Position = vec4(2.0, 2.0, 2.0, 1.0);
        gl_PointSize = 0.0;
        return;
      }

      gl_Position = vec4(p.xyz, 1.0);
      gl_PointSize = clamp(aParams.x * p.w * uDpr, 6.0, 460.0 * uDpr);
      vColor = aColor;
      vAlpha = aAlpha;
      vSeed = aParams.z;
      vRot = aParams.w + uTime * 0.015 * aParams.y;
    }
  \`;

  const nebulaFS = \`#version 300 es
    precision highp float;
    in vec3 vColor;
    in float vAlpha;
    in float vSeed;
    in float vRot;
    uniform float uDark;
    out vec4 outColor;

    float hash21(vec2 p) {
      p = fract(p * vec2(123.34, 456.21));
      p += dot(p, p + 45.32);
      return fract(p.x * p.y);
    }

    float vnoise(vec2 p) {
      vec2 i = floor(p), f = fract(p);
      f = f * f * (3.0 - 2.0 * f);
      return mix(mix(hash21(i), hash21(i + vec2(1.0, 0.0)), f.x), mix(hash21(i + vec2(0.0, 1.0)), hash21(i + vec2(1.0, 1.0)), f.x), f.y);
    }

    float fbm(vec2 p) {
      float sum = 0.0, amp = 0.54;
      mat2 m = mat2(0.80, -0.60, 0.60, 0.80);
      for (int i = 0; i < 3; i++) { sum += amp * vnoise(p); p = m * p * 2.06 + 11.7; amp *= 0.52; }
      return sum;
    }

    void main() {
      vec2 uv = gl_PointCoord * 2.0 - 1.0;
      float c = cos(vRot), s = sin(vRot);
      vec2 ruv = vec2(uv.x * c - uv.y * s, uv.x * s + uv.y * c);
      ruv.y *= 1.38;
      float dist = length(ruv);
      if (dist > 1.0) discard;

      float mask = pow(max(0.0, 1.0 - dist * dist), 1.65);
      float cloud = fbm(ruv * 2.25 + vec2(vSeed * 7.3, vSeed * 13.1));
      float density = mask * smoothstep(0.12, 0.74, cloud);

      vec3 col = uDark > 0.5 ? vColor : mix(vColor, vec3(0.20, 0.72, 0.78), 0.32);
      float alpha = density * vAlpha * (uDark > 0.5 ? 1.0 : 0.42);
      outColor = vec4(col, clamp(alpha, 0.0, 0.90));
    }
  \`;

  const diskProg = link(diskVS, diskFS);
  const starProg = link(starVS, starFS);
  const nebProg = link(nebulaVS, nebulaFS);
  if (!diskProg || !starProg || !nebProg) return null;

  // 创建 24x24 细分 3D 银河盘面网格
  const GRID_N = 24;
  const diskVerts = [];
  const DISK_EXTENT = 510;
  for (let iy = 0; iy < GRID_N; iy++) {
    for (let ix = 0; ix < GRID_N; ix++) {
      const x0 = -DISK_EXTENT + (ix / GRID_N) * DISK_EXTENT * 2;
      const x1 = -DISK_EXTENT + ((ix + 1) / GRID_N) * DISK_EXTENT * 2;
      const z0 = -DISK_EXTENT + (iy / GRID_N) * DISK_EXTENT * 2;
      const z1 = -DISK_EXTENT + ((iy + 1) / GRID_N) * DISK_EXTENT * 2;
      diskVerts.push(x0, z0, x1, z0, x1, z1, x0, z0, x1, z1, x0, z1);
    }
  }
  const diskVao = gl.createVertexArray();
  gl.bindVertexArray(diskVao);
  const diskBuf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, diskBuf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array(diskVerts), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  gl.bindVertexArray(null);
  const diskVertexCount = diskVerts.length / 2;

  function mulberry32(seed) {
    let a = seed >>> 0;
    return () => {
      a |= 0; a = (a + 0x6D2B79F5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }
  function gauss(rng) {
    const u1 = Math.max(1e-5, rng());
    return Math.sqrt(-2.0 * Math.log(u1)) * Math.cos(6.2831853 * rng());
  }

  const STAR_SPAN = 355;
  let diskStarsCount = 0, emissionNebCount = 0, currentSeed = 29;
  let starVao = null, starBuf = null, nebVao = null, nebBuf = null;

  function buildGalaxyBuffers(seed = 29) {
    currentSeed = seed;
    const rng = mulberry32(seed * 919 + 1337);
    const TOTAL_STARS = 11200;
    const starData = new Float32Array(TOTAL_STARS * 11);
    let si = 0;

    function pushStar(x, y, z, size, spin, phase, kind, r, g, b, a) {
      starData[si++] = x; starData[si++] = y; starData[si++] = z;
      starData[si++] = size; starData[si++] = spin; starData[si++] = phase; starData[si++] = kind;
      starData[si++] = r; starData[si++] = g; starData[si++] = b; starData[si++] = a;
    }

    // 1) 银核三维球状核晕（1,600 颗炽热核心星）
    for (let i = 0; i < 1600; i++) {
      const rr = Math.pow(rng(), 1.8) * 114;
      const th = rng() * Math.PI * 2;
      const phi = Math.acos(2 * rng() - 1);
      const x = Math.sin(phi) * Math.cos(th) * rr * 1.18;
      const y = Math.cos(phi) * rr * 0.45;
      const z = Math.sin(phi) * Math.sin(th) * rr * 0.92;
      const t = rr / 114;
      pushStar(x, y, z, 2.2 + rng() * 5.2, 0.95 - t * 0.25, rng() * 6.28, 1, 1.0 - t * 0.40, 0.98 - t * 0.04, 0.86 - t * 0.14, 0.46 + (1 - t) * 0.44);
    }

    // 2) 对数旋臂主盘恒星（7,400 颗）
    for (let i = 0; i < 7400; i++) {
      const armIdx = i % 4;
      const isMajor = armIdx < 2;
      const rad = 40 + Math.pow(rng(), 0.78) * (STAR_SPAN * (isMajor ? 1.24 : 1.06));
      const u = rad / STAR_SPAN;
      const theta = (armIdx * Math.PI * 2) / 4 + Math.log(1 + rad * 0.036) * 2.15 + gauss(rng) * (0.22 + u * 0.34) * (isMajor ? 0.85 : 1.35);
      const thickness = 18 * Math.exp(-u * 1.1) + 7 + (rng() > 0.92 ? 32 : 0);
      const y = gauss(rng) * thickness + Math.sin(theta * 2.0) * 12 * u;
      const x = Math.cos(theta) * rad;
      const z = Math.sin(theta) * rad;

      const bright = rng() > 0.962;
      const size = bright ? (7.5 + rng() * 8.5) : (1.9 + rng() * 3.8);
      const spin = Math.max(0.28, 1.05 / Math.pow(Math.max(0.25, u), 0.7));

      const cPick = rng();
      let cr, cg, cb;
      if (cPick < 0.46) { cr = 0.22; cg = 0.94; cb = 0.64; }
      else if (cPick < 0.76) { cr = 0.34; cg = 0.74; cb = 1.0; }
      else if (cPick < 0.90) { cr = 0.96; cg = 0.90; cb = 0.74; }
      else if (cPick < 0.96) { cr = 0.98; cg = 0.56; cb = 0.40; }
      else { cr = 0.88; cg = 0.48; cb = 0.96; }

      pushStar(x, y, z, size, spin, rng() * 6.28, 0, cr, cg, cb, bright ? 0.94 : (0.34 + rng() * 0.48));
    }

    // 3) 远景深空星穹（2,200 颗球壳星）
    for (let i = 0; i < 2200; i++) {
      const rr = 880 + rng() * 640;
      const th = rng() * Math.PI * 2;
      const phi = Math.acos(2 * rng() - 1);
      const x = Math.sin(phi) * Math.cos(th) * rr;
      const y = Math.cos(phi) * rr * 0.82;
      const z = Math.sin(phi) * Math.sin(th) * rr;
      const bright = rng() > 0.968;
      const cPick = rng();
      pushStar(x, y, z, bright ? 6.5 + rng() * 5.0 : 1.4 + rng() * 2.6, 0.08, rng() * 6.28, 2, cPick < 0.5 ? 0.76 : 0.96, cPick < 0.5 ? 0.88 : 0.96, cPick < 0.5 ? 1.0 : 0.88, bright ? 0.86 : (0.24 + rng() * 0.40));
    }
    diskStarsCount = TOTAL_STARS;

    // 4) 立体星云蓬松云团（260 团：叠加在连续星云盘之上，产生真正 3D 上下凸起的气体体积感）
    const EM_NEB = 260;
    const emData = new Float32Array(EM_NEB * 11);
    let ei = 0;
    for (let i = 0; i < EM_NEB; i++) {
      const armIdx = i % 4;
      const rad = 22 + Math.pow(rng(), 0.86) * (STAR_SPAN * 1.08);
      const u = rad / STAR_SPAN;
      const theta = (armIdx * Math.PI * 2) / 4 + Math.log(1 + rad * 0.036) * 2.15 + gauss(rng) * (0.24 + u * 0.22);
      const x = Math.cos(theta) * rad;
      const z = Math.sin(theta) * rad;
      const y = gauss(rng) * 18 + Math.sin(theta * 2) * 10 * u;

      const radSize = (95 + rng() * 145) * (0.74 + u * 0.50);
      const spin = Math.max(0.26, 0.9 / Math.pow(Math.max(0.25, u), 0.7));
      const pPick = rng();
      let cr, cg, cb;
      if (u < 0.24) { cr = 0.28; cg = 0.96; cb = 0.64; }
      else if (pPick < 0.46) { cr = 0.14; cg = 0.82; cb = 0.58; }
      else if (pPick < 0.78) { cr = 0.20; cg = 0.58; cb = 0.98; }
      else if (pPick < 0.92) { cr = 0.72; cg = 0.30; cb = 0.84; }
      else { cr = 0.92; cg = 0.54; cb = 0.26; }

      emData[ei++] = x; emData[ei++] = y; emData[ei++] = z;
      emData[ei++] = radSize; emData[ei++] = spin; emData[ei++] = rng() * 100; emData[ei++] = theta;
      emData[ei++] = cr; emData[ei++] = cg; emData[ei++] = cb;
      emData[ei++] = (0.22 + rng() * 0.16) * (1.0 - u * 0.28);
    }
    emissionNebCount = EM_NEB;

    function upload(data) {
      const vao = gl.createVertexArray();
      gl.bindVertexArray(vao);
      const buf = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, buf);
      gl.bufferData(gl.ARRAY_BUFFER, data, gl.STATIC_DRAW);
      const stride = 11 * 4;
      gl.enableVertexAttribArray(0); gl.vertexAttribPointer(0, 3, gl.FLOAT, false, stride, 0);
      gl.enableVertexAttribArray(1); gl.vertexAttribPointer(1, 4, gl.FLOAT, false, stride, 3 * 4);
      gl.enableVertexAttribArray(2); gl.vertexAttribPointer(2, 3, gl.FLOAT, false, stride, 7 * 4);
      gl.enableVertexAttribArray(3); gl.vertexAttribPointer(3, 1, gl.FLOAT, false, stride, 10 * 4);
      gl.bindVertexArray(null);
      return { vao, buf };
    }

    if (starBuf) gl.deleteBuffer(starBuf);
    if (nebBuf) gl.deleteBuffer(nebBuf);
    if (starVao) gl.deleteVertexArray(starVao);
    if (nebVao) gl.deleteVertexArray(nebVao);

    ({ vao: starVao, buf: starBuf } = upload(starData));
    ({ vao: nebVao, buf: nebBuf } = upload(emData));
  }

  buildGalaxyBuffers(29);

  function setCommonUniforms(prog, st) {
    gl.uniform2f(gl.getUniformLocation(prog, "uRes"), st.width, st.height);
    gl.uniform2f(gl.getUniformLocation(prog, "uCenter"), st.cx, st.cy);
    gl.uniform3f(gl.getUniformLocation(prog, "uFocus"), st.fx, st.fy, st.fz);
    gl.uniform3f(gl.getUniformLocation(prog, "uCam"), st.yaw, st.pitch, st.zoom);
    gl.uniform1f(gl.getUniformLocation(prog, "uDpr"), st.dpr);
    gl.uniform1f(gl.getUniformLocation(prog, "uFov"), st.fov);
    gl.uniform1f(gl.getUniformLocation(prog, "uTime"), st.time);
    gl.uniform1f(gl.getUniformLocation(prog, "uDiskSpin"), st.diskSpin);
    gl.uniform1f(gl.getUniformLocation(prog, "uDark"), st.dark ? 1.0 : 0.0);
    gl.uniform1f(gl.getUniformLocation(prog, "uSeed"), currentSeed);
  }

  return {
    reseed(seed) { buildGalaxyBuffers(seed); },
    render(st) {
      const pw = Math.round(st.width * st.dpr);
      const ph = Math.round(st.height * st.dpr);
      if (canvas.width !== pw || canvas.height !== ph) {
        canvas.width = pw;
        canvas.height = ph;
      }
      gl.viewport(0, 0, pw, ph);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.disable(gl.DEPTH_TEST);
      gl.enable(gl.BLEND);

      // 1) 连续三维对数旋臂 & 暗尘埃吸收带星云盘
      gl.useProgram(diskProg);
      setCommonUniforms(diskProg, st);
      gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.bindVertexArray(diskVao);
      gl.drawArrays(gl.TRIANGLES, 0, diskVertexCount);

      // 2) 立体起伏 FBM 星云团块
      gl.useProgram(nebProg);
      setCommonUniforms(nebProg, st);
      if (st.dark) gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
      else gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.bindVertexArray(nebVao);
      gl.drawArrays(gl.POINTS, 0, emissionNebCount);

      // 3) 11,200 颗光谱恒星 & 银核星云
      gl.useProgram(starProg);
      setCommonUniforms(starProg, st);
      if (st.dark) gl.blendFunc(gl.SRC_ALPHA, gl.ONE);
      else gl.blendFunc(gl.SRC_ALPHA, gl.ONE_MINUS_SRC_ALPHA);
      gl.bindVertexArray(starVao);
      gl.drawArrays(gl.POINTS, 0, diskStarsCount);

      gl.bindVertexArray(null);
    },
  };
}
`;
