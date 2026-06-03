// wawa · demo web — host JS que rehostea las apps WASM del userspace de wawa.
//
// El kernel real (03_ukupacha/wawa/wawa-kernel/src/wasm/) le inyecta al modulo
// dos capacidades por el modulo de imports "renaser":
//
//   sys_render_frame(ptr, len)   -> composita un fotograma de la memoria lineal
//   sys_get_scancode() -> u32    -> siguiente scancode crudo del canal propio
//
// Aqui se replica esa misma ABI desde el navegador. Pixel format en memoria:
// u32 little-endian (0x00_RR_GG_BB), o sea bytes [B, G, R, 0]. Hay que swizzlear
// a RGBA antes de pintarlo en el ImageData del canvas.

const APPS = [
  {
    nombre:  "hello_wasm",
    wasm:    "dist/hello_wasm.wasm",
    ancho:   480,
    alto:    560,
    canvas:  "canvas-hello",
  },
];

// --- Cola de scancodes (PS/2 Set 1) por app. ---
// El kernel real expone uno por aplicacion; en el demo, cada canvas con foco
// alimenta su propio canal. Tope de cola = 32 para no acumular eventos viejos.
const TECLA_A_SCANCODE = {
  KeyW: 0x11, KeyS: 0x1F, KeyA: 0x1E, KeyD: 0x20,
  ArrowUp: 0x11, ArrowDown: 0x1F, ArrowLeft: 0x1E, ArrowRight: 0x20,
};

function crearCanalTeclado(canvas) {
  const cola = [];
  const enqueue = (sc) => { if (cola.length < 32) cola.push(sc); };
  canvas.addEventListener("keydown", (ev) => {
    const sc = TECLA_A_SCANCODE[ev.code];
    if (sc !== undefined) {
      enqueue(sc);
      ev.preventDefault();
    }
  });
  // foco automatico al hacer click: la app necesita teclado.
  canvas.addEventListener("pointerdown", () => canvas.focus());
  return {
    pop: () => (cola.length ? cola.shift() : 0),
  };
}

// --- Compositor: lee el fotograma de la memoria lineal y lo pinta. ---
// El kernel real exige len == ancho*alto*4; aqui se valida igual y se aborta
// el fotograma si miente (la app dejaria de pintar pero no rompe al host).
function crearCompositor(canvas, ancho, alto) {
  const ctx = canvas.getContext("2d", { alpha: false });
  const imagen = ctx.createImageData(ancho, alto);
  const esperado = ancho * alto * 4;

  return function presentar(memoria, ptr, len) {
    if (len !== esperado) {
      console.warn(`fotograma de ${len} bytes, esperaba ${esperado} — descartado`);
      return;
    }
    if (ptr + len > memoria.buffer.byteLength) {
      console.warn("fotograma desborda la memoria lineal — descartado");
      return;
    }
    const fuente = new Uint32Array(memoria.buffer, ptr, len / 4);
    const salida = imagen.data;
    for (let i = 0; i < fuente.length; i++) {
      const p = fuente[i];
      const k = i * 4;
      salida[k    ] = (p >> 16) & 0xFF; // R
      salida[k + 1] = (p >>  8) & 0xFF; // G
      salida[k + 2] =  p        & 0xFF; // B
      salida[k + 3] = 0xFF;              // A
    }
    ctx.putImageData(imagen, 0, 0);
  };
}

// --- Instanciar una app: replica el flujo del kernel (init una vez, tick por frame). ---
async function arrancarApp(spec) {
  const canvas = document.getElementById(spec.canvas);
  const canal = crearCanalTeclado(canvas);
  const presentar = crearCompositor(canvas, spec.ancho, spec.alto);

  let memoria = null; // se resuelve tras instanciar

  const imports = {
    renaser: {
      sys_render_frame: (ptr, len) => presentar(memoria, ptr >>> 0, len >>> 0),
      sys_get_scancode: () => canal.pop() >>> 0,
    },
  };

  const respuesta = await fetch(spec.wasm);
  if (!respuesta.ok) throw new Error(`no se pudo bajar ${spec.wasm}: ${respuesta.status}`);
  const { instance } = await WebAssembly.instantiateStreaming(respuesta, imports);

  memoria = instance.exports.memory;
  if (!memoria) throw new Error(`${spec.nombre} no exporta su memoria lineal`);

  // Arranque: un unico init, luego un tick por requestAnimationFrame.
  instance.exports.init();
  const tick = instance.exports.tick;
  const bucle = () => {
    try {
      tick();
    } catch (e) {
      console.error(`${spec.nombre} disparo una trampa, se desaloja:`, e);
      return; // no agendar mas frames — emula el desalojo del kernel.
    }
    requestAnimationFrame(bucle);
  };
  requestAnimationFrame(bucle);

  // Foco inicial para que el teclado entre sin un click previo.
  canvas.focus();

  return instance;
}

// --- Boot del demo. ---
const estado = document.getElementById("estado");
try {
  estado.textContent = "instanciando…";
  await Promise.all(APPS.map(arrancarApp));
  estado.textContent = "viva";
} catch (e) {
  console.error(e);
  estado.textContent = `falla: ${e.message}`;
  estado.style.color = "#ff5555";
}
