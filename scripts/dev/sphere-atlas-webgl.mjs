// 零依赖 WebGL2 真 360° 全向可拨转「凝光折射晶核」三维体积内雕渲染引擎
// - 5 层体积切片步进 (Volumetric Extruded Slab)：正面清透放大、45° 立体晶雕厚度、90° 侧视发光晶棱、180° 背透翻转，全向 360° 无死角
// - 光学凸透镜折射 + 2px RGB 棱镜微色散 + 内腔焦散聚光 + 浅色明快彩宝配色 / 深色深空夜光晶核
export const SPHERE_ATLAS_WEBGL_SOURCE = `
function createSphereAtlasRenderer() {
  const CELL = 176;
  const COLS = 4;
  const ROWS = 3;
  const canvas = document.createElement("canvas");
  canvas.width = CELL * COLS;
  canvas.height = CELL * ROWS;

  const gl = canvas.getContext("webgl2", {
    alpha: true,
    antialias: true,
    premultipliedAlpha: true,
    preserveDrawingBuffer: true,
    powerPreference: "high-performance",
  });
  if (!gl) return null;

  function compile(type, src) {
    const sh = gl.createShader(type);
    gl.shaderSource(sh, src);
    gl.compileShader(sh);
    if (!gl.getShaderParameter(sh, gl.COMPILE_STATUS)) {
      console.warn("Sphere shader error:", gl.getShaderInfoLog(sh));
      gl.deleteShader(sh);
      return null;
    }
    return sh;
  }

  const vsSrc = \`#version 300 es
    precision highp float;
    layout(location = 0) in vec2 aPos;
    out vec2 vUv;
    void main() {
      vUv = aPos * 0.5 + 0.5;
      gl_Position = vec4(aPos, 0.0, 1.0);
    }
  \`;

  const fsSrc = \`#version 300 es
    precision highp float;
    in vec2 vUv;

    uniform sampler2D uGlyphTex;       // R:锐利轮廓, G:外扩光晕, B:三维浮雕坡面
    uniform sampler2D uColorLightTex;  // 浅色主题全彩品牌原色图腾
    uniform sampler2D uColorDarkTex;   // 深色主题全彩品牌原色图腾
    uniform float uHasGlyph;
    uniform float uGlyphSlot;

    uniform float uTime;
    uniform float uSpin;       // 水平 360° 无限自转/拨转角
    uniform float uPitchSpin;  // 垂直 360° 无限俯仰拨转角
    uniform float uTilt;
    uniform float uCamYaw;
    uniform float uCamPitch;
    uniform float uSeed;
    uniform float uIsCore;
    uniform float uDark;
    uniform float uTone;       // 0: normal, 1: warn, 2: danger
    uniform float uFocus;
    uniform vec3 uColor;
    uniform vec3 uLightDir;

    out vec4 outColor;

    float hash31(vec3 p) {
      p = fract(p * vec3(0.1031, 0.1030, 0.0973));
      p += dot(p, p.yxz + 33.33);
      return fract((p.x + p.y) * p.z);
    }

    float vnoise3(vec3 p) {
      vec3 i = floor(p);
      vec3 f = fract(p);
      f = f * f * (3.0 - 2.0 * f);
      float n000 = hash31(i);
      float n100 = hash31(i + vec3(1.0, 0.0, 0.0));
      float n010 = hash31(i + vec3(0.0, 1.0, 0.0));
      float n110 = hash31(i + vec3(1.0, 1.0, 0.0));
      float n001 = hash31(i + vec3(0.0, 0.0, 1.0));
      float n101 = hash31(i + vec3(1.0, 0.0, 1.0));
      float n011 = hash31(i + vec3(0.0, 1.0, 1.0));
      float n111 = hash31(i + vec3(1.0, 1.0, 1.0));
      vec4 nx0 = mix(vec4(n000, n010, n001, n011), vec4(n100, n110, n101, n111), f.x);
      vec2 nxy = mix(nx0.xy, nx0.zw, f.y);
      return mix(nxy.x, nxy.y, f.z);
    }

    float softFbm(vec3 p) {
      float sum = 0.0;
      float amp = 0.56;
      for (int i = 0; i < 3; i++) {
        sum += amp * vnoise3(p);
        p = p * 2.02 + vec3(5.3, 11.7, 3.1);
        amp *= 0.46;
      }
      return sum;
    }

    vec3 rotY(vec3 v, float a) { float c = cos(a), s = sin(a); return vec3(v.x * c - v.z * s, v.y, v.x * s + v.z * c); }
    vec3 rotX(vec3 v, float a) { float c = cos(a), s = sin(a); return vec3(v.x, v.y * c - v.z * s, v.y * s + v.z * c); }
    vec3 rotZ(vec3 v, float a) { float c = cos(a), s = sin(a); return vec3(v.x * c - v.y * s, v.x * s + v.y * c, v.z); }

    // 完整 360° × 360° 无死角三维体空间变换（叠加轻柔呼吸摆动、手动全向 360° 拨转角与 3D 相机环绕角）
    vec3 toCrystalBody(vec3 v) {
      float idleSwayY = sin(uTime * 0.85 + uSeed * 1.7) * 0.15;
      float idleSwayX = cos(uTime * 0.68 + uSeed * 1.3) * 0.08;
      v = rotX(v, -uPitchSpin - idleSwayX - (uCamPitch - 0.44) * 0.85);
      v = rotY(v, uSpin + idleSwayY - uCamYaw);
      v = rotZ(v, uTilt * 0.28);
      return v;
    }

    vec4 sampleGlyphPack(vec2 localUv, out vec3 brandCol) {
      brandCol = uColor;
      if (uHasGlyph < 0.5) return vec4(0.0);
      if (localUv.x < 0.02 || localUv.x > 0.98 || localUv.y < 0.02 || localUv.y > 0.98) return vec4(0.0);
      float slot = floor(uGlyphSlot + 0.5);
      float col = mod(slot, 4.0);
      float row = floor(slot / 4.0);
      vec2 atlasUv = (vec2(col, row) + clamp(localUv, 0.01, 0.99)) / vec2(4.0, 3.0);
      float vignette = smoothstep(0.49, 0.39, length(localUv - 0.5));
      vec3 sdf = texture(uGlyphTex, atlasUv).rgb * vignette;
      vec3 cCol = uDark > 0.5 ? texture(uColorDarkTex, atlasUv).rgb : texture(uColorLightTex, atlasUv).rgb;
      brandCol = cCol;
      return vec4(sdf, 1.0);
    }

    vec3 sampleCrystalUV(vec3 Q, float halfSpan, out vec3 brandCol) {
      vec2 uv = vec2(0.5 + Q.x / (halfSpan * 2.0), 0.5 - Q.y / (halfSpan * 2.0));
      return sampleGlyphPack(uv, brandCol).rgb;
    }

    void main() {
      vec2 p = vUv * 2.0 - 1.0;
      float R_SOLID = 0.84;
      float r = length(p);
      if (r > 1.0) discard;

      vec3 baseHue = uColor;
      vec3 rimAlert = baseHue;
      if (uTone > 1.5) rimAlert = vec3(0.95, 0.32, 0.37);
      else if (uTone > 0.5) rimAlert = mix(baseHue, vec3(0.96, 0.68, 0.16), 0.65);

      vec3 L = normalize(uLightDir);
      vec3 V = vec3(0.0, 0.0, 1.0);

      // ── 球外柔和晶石光晕 / Jolu 翡翠星冕 ──
      if (r > R_SOLID) {
        float rimDist = (r - R_SOLID) / (1.0 - R_SOLID);
        float falloff = pow(max(0.0, 1.0 - rimDist), 2.55);
        vec2 dir2 = p / max(1e-4, r);
        if (uIsCore > 0.5) {
          float ang = atan(p.y, p.x);
          float rays = 0.88 + 0.12 * sin(ang * 6.0 + uTime * 0.45);
          vec3 coronaCol = uDark > 0.5 ? vec3(0.24, 0.96, 0.64) : vec3(0.10, 0.76, 0.48);
          float ca = falloff * rays * (uDark > 0.5 ? 0.76 : 0.34);
          outColor = vec4(coronaCol * ca, ca);
        } else {
          float sunSide = 0.42 + 0.58 * smoothstep(-0.35, 0.95, dot(vec3(dir2, 0.0), normalize(L + vec3(0.0, 0.0, 0.35))));
          vec3 haloCol = uDark > 0.5 ? mix(rimAlert, vec3(0.86, 0.97, 1.0), 0.25) : rimAlert;
          float aa = falloff * sunSide * ((uDark > 0.5 ? 0.58 : 0.26) + uFocus * 0.20);
          outColor = vec4(haloCol * aa, aa);
        }
        return;
      }

      // ── 球面几何与真实 Snell 折射光线贯穿整颗晶球 ──
      vec2 sp = p / R_SOLID;
      float rSq = dot(sp, sp);
      float z = sqrt(max(0.0, 1.0 - rSq));
      vec3 N = normalize(vec3(sp.x, sp.y, z));
      float edgeAA = smoothstep(R_SOLID, R_SOLID - 0.020, r);
      float fresnel = pow(max(0.0, 1.0 - z), 2.85);

      // 视线经凸球面透镜折射进入晶球内部 (IOR ≈ 1.45)
      vec3 I = vec3(0.0, 0.0, -1.0);
      vec3 refrG = refract(I, N, 1.0 / 1.45);
      vec3 refrR = refract(I, N, 1.0 / 1.39);
      vec3 refrB = refract(I, N, 1.0 / 1.52);

      // 解析式三维水晶晶片求交 [-slabHalf, +slabHalf]：正面零锯齿矢量锐度、45° 真实双面晶雕厚度、90° 纤薄导光晶棱、180° 背透翻转！
      float tMaxSphere = max(0.08, -2.0 * dot(N, refrG));
      vec3 O_body = toCrystalBody(N);
      vec3 D_bodyG = toCrystalBody(refrG);
      vec3 D_bodyR = toCrystalBody(refrR);
      vec3 D_bodyB = toCrystalBody(refrB);

      float halfSpan = uIsCore > 0.5 ? 0.57 : 0.53;
      float slabHalf = 0.046;

      float safeDz = abs(D_bodyG.z) < 1e-4 ? (D_bodyG.z >= 0.0 ? 1e-4 : -1e-4) : D_bodyG.z;
      float tA = (-slabHalf - O_body.z) / safeDz;
      float tB = ( slabHalf - O_body.z) / safeDz;
      float tEnter = max(0.0, min(tA, tB));
      float tExit  = min(tMaxSphere, max(tA, tB));

      float maskG = 0.0;
      float maskR = 0.0;
      float maskB = 0.0;
      float halo  = 0.0;
      float bevelShade = 0.0;
      float faceWeight = 1.0;
      vec3  logoHue = uColor;

      if (tEnter <= tExit) {
        float facing = abs(D_bodyG.z);
        float tMid = 0.5 * (tEnter + tExit);

        vec3 unusedC;
        vec3 sFront  = sampleCrystalUV(O_body + D_bodyG * tEnter, halfSpan, unusedC);
        vec3 sMid1   = sampleCrystalUV(O_body + D_bodyG * mix(tEnter, tExit, 0.25), halfSpan, unusedC);
        vec3 sCenter = sampleCrystalUV(O_body + D_bodyG * tMid, halfSpan, logoHue);
        vec3 sMid2   = sampleCrystalUV(O_body + D_bodyG * mix(tEnter, tExit, 0.75), halfSpan, unusedC);
        vec3 sBack   = sampleCrystalUV(O_body + D_bodyG * tExit, halfSpan, unusedC);

        float fFace = smoothstep(0.36, 0.64, sFront.r);
        float cFace = smoothstep(0.36, 0.64, sCenter.r);
        float bFace = smoothstep(0.36, 0.64, sBack.r);
        float wall  = max(max(smoothstep(0.38, 0.66, sMid1.r), smoothstep(0.38, 0.66, sMid2.r)), max(fFace, bFace));

        faceWeight = smoothstep(0.06, 0.38, facing);
        maskG = mix(wall * 0.64, max(cFace, wall * 0.74), faceWeight);

        bevelShade = (sFront.b - sBack.b) * 0.85 + (fFace - bFace) * 0.32;
        halo = max(sCenter.g, max(sFront.g, sBack.g)) * mix(0.45, 1.0, faceWeight);

        float cR = smoothstep(0.36, 0.64, sampleCrystalUV(O_body + D_bodyR * tMid, halfSpan, unusedC).r);
        float cB = smoothstep(0.36, 0.64, sampleCrystalUV(O_body + D_bodyB * tMid, halfSpan, unusedC).r);
        maskR = mix(maskG, max(cR, wall * 0.68), faceWeight * 0.68);
        maskB = mix(maskG, max(cB, wall * 0.68), faceWeight * 0.68);
      }
      halo = clamp(halo * 1.08, 0.0, 1.0);

      // 随球体 360° 同步旋转的内部丝绸流光与星尘晶点（即使转到侧棱也能直观感受球体 360° 自由翻滚！）
      vec3 B_surf = toCrystalBody(N);
      vec3 seedVec = vec3(uSeed * 1.19, uSeed * 2.41, uSeed * 0.83);
      float silk = softFbm(B_surf * 2.1 + vec3(uTime * 0.06) + seedVec);
      float dustNoise = vnoise3(B_surf * 11.5 + seedVec * 2.0);
      float crystalSparkle = smoothstep(0.86, 0.97, dustNoise) * (0.5 + 0.5 * sin(uTime * 2.6 + dustNoise * 18.0)) * z;

      // ── 光学光照项：次表面散射 (SSS) + 内壁底部焦散聚光 (Caustic) + 贴肩弯月玻璃高光 ──
      float ndotl = dot(N, L);
      float sss = pow(max(0.0, ndotl * 0.55 + 0.45), 1.42);
      float caustic = pow(max(0.0, dot(N, normalize(vec3(-L.x * 0.75, -L.y * 0.75, 0.55)))), 3.4) * (1.0 - z * 0.42);

      // 将顶部弯月与玻璃反光点推向左上球肩，透出中心清澈晶窗以100%忠实呈现各厂商全彩 Logo
      vec3 H_top = normalize(vec3(-0.54, 0.78, 0.46) + V);
      float domeSpec = pow(max(0.0, dot(N, H_top)), 78.0);
      float domeCrescent = smoothstep(0.60, 0.92, sp.y * 0.80 - sp.x * 0.42)
                         * smoothstep(0.99, 0.68, length(sp))
                         * smoothstep(0.46, 0.74, length(sp));
      vec3 H_star = normalize(L + V);
      float starSpec = pow(max(0.0, dot(N, H_star)), 42.0) * 0.24;

      // 三维立体珐琅/彩釉内雕着色：正面保留原厂真实色相，旋转斜侧露出稍深的立体侧棱壁厚，边沿微带棱镜色散光彩
      vec3 wallCol = logoHue * 0.78 + vec3(clamp(bevelShade * 0.24, -0.08, 0.22));
      vec3 faceCol = logoHue + vec3(clamp(bevelShade * 0.14, -0.05, 0.14));
      vec3 emblemCol = clamp(mix(wallCol, faceCol, faceWeight), 0.0, 1.0);
      vec3 prismEdge = clamp(vec3(maskR - maskG, 0.0, maskB - maskG), 0.0, 1.0);

      vec3 col;
      float alpha;

      if (uDark > 0.5) {
        // ── 深色模式 (R1-08-V2)：深邃烟晶光学玻璃内嵌霓虹背光全彩厂商徽标 ──
        vec3 centerSmoked = mix(vec3(0.025, 0.048, 0.045), baseHue * 0.34, 0.34 + silk * 0.26);
        vec3 outerShell   = mix(baseHue * 0.68, vec3(0.75, 0.94, 1.0), 0.14) * (0.45 + 0.55 * sss);
        col = mix(centerSmoked, outerShell, smoothstep(0.08, 0.76, fresnel * 0.85 + (1.0 - z) * 0.28));
        col += mix(baseHue, vec3(1.0), 0.40) * (caustic * 0.32 + crystalSparkle * 0.42);

        // 微弱彩色水晶背光垫层，确保深蓝等高饱和冷色品牌标识在暗晶内同样晶莹剔透
        col += mix(logoHue, vec3(0.92, 0.98, 1.0), 0.35) * clamp((halo - maskG * 0.55) * 0.50, 0.0, 0.42);
        col = mix(col, emblemCol, clamp(maskG * 0.96, 0.0, 1.0));
        col += prismEdge * 0.38;

        col += mix(rimAlert, vec3(0.88, 0.98, 1.0), 0.30) * fresnel * (0.86 + 0.28 * sss);
        col += vec3(0.96, 0.99, 1.0) * (domeSpec * 0.72 + domeCrescent * 0.28 + starSpec);
        alpha = mix(0.84, 0.97, fresnel) * edgeAA;
      } else {
        // ── 浅色模式 (R1-08-V1)：清澈珠光白水晶中心透视窗口 + 饱和彩宝外圈包边，完美衬托各厂商真实品牌色彩！ ──
        vec3 clearCenter = mix(vec3(0.975, 0.992, 0.985), baseHue, 0.09 + silk * 0.04);
        vec3 gemMeniscus = mix(baseHue, pow(clamp(baseHue, 0.02, 1.0), vec3(1.24)) * 0.92, 0.44);
        float shellGrad  = smoothstep(0.06, 0.74, fresnel * 0.78 + (1.0 - z) * 0.50);
        col = mix(clearCenter, gemMeniscus, shellGrad);
        // 底部琉璃焦散透光池与星尘晶点
        vec3 causticTint = mix(baseHue, vec3(1.0, 0.99, 0.94), 0.64);
        col = mix(col, causticTint, caustic * 0.36);
        col += vec3(1.0) * crystalSparkle * 0.34;

        // 徽标周围一圈洁净白晶柔光垫底，使多色矢量边沿在彩宝壳内保持 100% 锐度
        col = mix(col, vec3(0.99, 1.0, 0.995), clamp((halo - maskG * 0.50) * 0.55, 0.0, 0.42));
        col = mix(col, emblemCol, clamp(maskG * 0.96, 0.0, 1.0));
        col += prismEdge * 0.24;

        // 鲜明彩宝玻璃外圈轮廓（在白底星盘上形成珠宝般立体圆润边沿）+ 贴肩玻璃反光
        vec3 lightRim = uTone > 0.5 ? rimAlert : mix(baseHue * 0.84, vec3(1.0), 0.16);
        col = mix(col, lightRim, fresnel * 0.72);
        col += vec3(1.0) * (domeSpec * 0.68 + domeCrescent * 0.30 + starSpec * 0.65);
        alpha = mix(0.92, 0.98, fresnel) * edgeAA;
      }

      outColor = vec4(clamp(col, 0.0, 1.0) * alpha, alpha);
    }
  \`;

  const vs = compile(gl.VERTEX_SHADER, vsSrc);
  const fs = compile(gl.FRAGMENT_SHADER, fsSrc);
  if (!vs || !fs) return null;
  const prog = gl.createProgram();
  gl.attachShader(prog, vs);
  gl.attachShader(prog, fs);
  gl.linkProgram(prog);
  if (!gl.getProgramParameter(prog, gl.LINK_STATUS)) return null;

  const vao = gl.createVertexArray();
  gl.bindVertexArray(vao);
  const buf = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, buf);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1, 1,-1, 1,1, -1,-1, 1,1, -1,1]), gl.STATIC_DRAW);
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  gl.bindVertexArray(null);

  function createAtlasTex() {
    const tex = gl.createTexture();
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, 4, 4, 0, gl.RGBA, gl.UNSIGNED_BYTE, new Uint8Array(64));
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    return tex;
  }
  const glyphTex = createAtlasTex();
  const colorLightTex = createAtlasTex();
  const colorDarkTex = createAtlasTex();
  let hasGlyphTex = false;

  const locs = {
    uGlyphTex: gl.getUniformLocation(prog, "uGlyphTex"),
    uColorLightTex: gl.getUniformLocation(prog, "uColorLightTex"),
    uColorDarkTex: gl.getUniformLocation(prog, "uColorDarkTex"),
    uHasGlyph: gl.getUniformLocation(prog, "uHasGlyph"),
    uGlyphSlot: gl.getUniformLocation(prog, "uGlyphSlot"),
    uTime: gl.getUniformLocation(prog, "uTime"),
    uSpin: gl.getUniformLocation(prog, "uSpin"),
    uPitchSpin: gl.getUniformLocation(prog, "uPitchSpin"),
    uTilt: gl.getUniformLocation(prog, "uTilt"),
    uCamYaw: gl.getUniformLocation(prog, "uCamYaw"),
    uCamPitch: gl.getUniformLocation(prog, "uCamPitch"),
    uSeed: gl.getUniformLocation(prog, "uSeed"),
    uIsCore: gl.getUniformLocation(prog, "uIsCore"),
    uDark: gl.getUniformLocation(prog, "uDark"),
    uTone: gl.getUniformLocation(prog, "uTone"),
    uFocus: gl.getUniformLocation(prog, "uFocus"),
    uColor: gl.getUniformLocation(prog, "uColor"),
    uLightDir: gl.getUniformLocation(prog, "uLightDir"),
  };

  function hexToRgb(hex) {
    const h = hex.replace("#", "");
    const v = parseInt(h.length === 3 ? h.replace(/./g, "$&$&") : h, 16);
    return [(v >> 16 & 255) / 255, (v >> 8 & 255) / 255, (v & 255) / 255];
  }

  function uploadTex(tex, imgCanvas) {
    gl.bindTexture(gl.TEXTURE_2D, tex);
    gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
    gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
    gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, imgCanvas);
  }

  return {
    canvas,
    cellSize: CELL,
    cols: COLS,
    uploadGlyphAtlas(sdfAtlas, lightColorAtlas, darkColorAtlas) {
      uploadTex(glyphTex, sdfAtlas);
      uploadTex(colorLightTex, lightColorAtlas || sdfAtlas);
      uploadTex(colorDarkTex, darkColorAtlas || lightColorAtlas || sdfAtlas);
      hasGlyphTex = true;
    },
    render(items, camState, time, isDark) {
      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 0);
      gl.clear(gl.COLOR_BUFFER_BIT);
      gl.disable(gl.DEPTH_TEST);
      gl.disable(gl.BLEND);

      gl.useProgram(prog);
      gl.bindVertexArray(vao);

      gl.activeTexture(gl.TEXTURE0);
      gl.bindTexture(gl.TEXTURE_2D, glyphTex);
      gl.uniform1i(locs.uGlyphTex, 0);

      gl.activeTexture(gl.TEXTURE1);
      gl.bindTexture(gl.TEXTURE_2D, colorLightTex);
      gl.uniform1i(locs.uColorLightTex, 1);

      gl.activeTexture(gl.TEXTURE2);
      gl.bindTexture(gl.TEXTURE_2D, colorDarkTex);
      gl.uniform1i(locs.uColorDarkTex, 2);

      gl.uniform1f(locs.uHasGlyph, hasGlyphTex ? 1.0 : 0.0);

      gl.uniform1f(locs.uTime, time);
      gl.uniform1f(locs.uCamYaw, camState.yaw);
      gl.uniform1f(locs.uCamPitch, camState.pitch);
      gl.uniform1f(locs.uDark, isDark ? 1.0 : 0.0);

      const count = Math.min(COLS * ROWS, items.length);
      for (let i = 0; i < count; i++) {
        const it = items[i];
        const col = i % COLS;
        const row = ROWS - 1 - Math.floor(i / COLS);
        gl.viewport(col * CELL, row * CELL, CELL, CELL);
        gl.uniform1f(locs.uGlyphSlot, it.glyphSlot ?? i);
        gl.uniform1f(locs.uSpin, it.spin);
        gl.uniform1f(locs.uPitchSpin, it.pitchSpin ?? 0);
        gl.uniform1f(locs.uTilt, it.tilt);
        gl.uniform1f(locs.uSeed, it.seed);
        gl.uniform1f(locs.uIsCore, it.isCore ? 1.0 : 0.0);
        gl.uniform1f(locs.uTone, it.toneNum);
        gl.uniform1f(locs.uFocus, it.focus ? 1.0 : 0.0);
        const [cr, cg, cb] = hexToRgb(it.color);
        gl.uniform3f(locs.uColor, cr, cg, cb);
        gl.uniform3f(locs.uLightDir, it.lx, it.ly, it.lz);
        gl.drawArrays(gl.TRIANGLES, 0, 6);
      }
      gl.bindVertexArray(null);
    },
    blitTile(idx, targetCtx, sizePx) {
      const col = idx % COLS;
      const row = Math.floor(idx / COLS);
      targetCtx.clearRect(0, 0, sizePx, sizePx);
      targetCtx.drawImage(canvas, col * CELL, row * CELL, CELL, CELL, 0, 0, sizePx, sizePx);
    },
  };
}
`;
