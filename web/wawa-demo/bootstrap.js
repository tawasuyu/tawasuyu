// wawa · demo web — bootstrap minimo.
//
// Cargamos `wawa-web` (pkg/) — el binario que adentro lleva el host wasmi del
// kernel y la app hello_wasm embebida — y solo le canalizamos canvas y teclado.
// Toda la mecanica del host (capacidades, shielding de combustible, validacion
// de la memoria lineal del huesped) vive en Rust, ejecutandose en wasm.

import init, { WawaWeb } from "./pkg/wawa_web.js";

const estado = document.getElementById("estado");
const canvas = document.getElementById("canvas-hello");

// PS/2 Set 1 — el mismo mapeo que el kernel decodifica desde i8042.
const TECLA_A_SCANCODE = {
  KeyW: 0x11, KeyS: 0x1F, KeyA: 0x1E, KeyD: 0x20,
  ArrowUp: 0x11, ArrowDown: 0x1F, ArrowLeft: 0x1E, ArrowRight: 0x20,
};

try {
  estado.textContent = "instanciando wasmi…";
  await init();

  const wawa = new WawaWeb();
  const ancho = wawa.ancho;
  const alto  = wawa.alto;
  if (canvas.width !== ancho || canvas.height !== alto) {
    canvas.width = ancho;
    canvas.height = alto;
  }

  const ctx = canvas.getContext("2d", { alpha: false });
  const imagen = ctx.createImageData(ancho, alto);

  canvas.addEventListener("keydown", (ev) => {
    const sc = TECLA_A_SCANCODE[ev.code];
    if (sc !== undefined) {
      wawa.enviar_scancode(sc);
      ev.preventDefault();
    }
  });
  canvas.addEventListener("pointerdown", () => canvas.focus());
  canvas.focus();

  estado.textContent = "viva";

  const bucle = () => {
    let pixeles;
    try {
      pixeles = wawa.tick();
    } catch (e) {
      console.error("trampa en hello_wasm — desalojo:", e);
      estado.textContent = "trampa — desalojada";
      estado.style.color = "#ff5555";
      return;
    }
    imagen.data.set(pixeles);
    ctx.putImageData(imagen, 0, 0);
    requestAnimationFrame(bucle);
  };
  requestAnimationFrame(bucle);
} catch (e) {
  console.error(e);
  estado.textContent = `falla: ${e.message ?? e}`;
  estado.style.color = "#ff5555";
}
