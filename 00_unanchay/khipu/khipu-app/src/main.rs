//! `khipu-app` — cuaderno de notas sobre Llimphi.
//!
//! Tres regiones, todas en la misma ventana, sin modal:
//! - **Lista** (izquierda, 240 px): notas en orden de creación.
//!   Click selecciona. Botón `+ nueva` arriba.
//! - **Editor** (centro): título (input), cuerpo (text-editor con
//!   wiki-links `[[...]]`), etiquetas (input). Edición directa — la
//!   nota seleccionada se modifica al teclear, sin botón guardar.
//! - **Gravedad** (derecha): canvas vello que pinta las posiciones 2D
//!   con que `khipu_gravity::SemanticField::anchor_new` ancló cada nota
//!   al crearse (baricentro semántico). Color por clúster (umbral 0.55),
//!   la seleccionada va resaltada con borde acento.
//!
//! **Embeddings**: si hay un `verbo-daemon` corriendo en el socket por
//! defecto (`$XDG_RUNTIME_DIR/verbo.sock`) los vectores son reales
//! (fastembed e5, etc.) — clústeres y vecinos se vuelven semánticos de
//! verdad. Sin daemon caemos al hash trigram → R^16 local: determinista,
//! offline, idéntico al comportamiento histórico.
//!
//! **Persistencia**: cada mutación graba `$XDG_DATA_HOME/khipu/notes.bin`
//! con postcard. Al arrancar, si el archivo existe se carga; si el espacio
//! cambió los vectores se recalculan. Sin archivo se siembra el cuaderno
//! demo (siete notas en español).
//!
//! ## Organización de módulos
//!
//! | módulo    | contenido                                                  |
//! |-----------|-------------------------------------------------------------|
//! | `modelo`  | tipos (`Embedder`, `Focus`, `Msg`, `Model`, constantes)     |
//! | `estado`  | persistencia, embeddings, helpers de modelo (select, etc.) |
//! | `menu`    | menú principal + menú de edición contextual                 |
//! | `app`     | `KhipuApp` + implementación `App` (init/update/view/on_key)|
//! | `map`     | lienzo de pensamientos: geometría, cámara, pintado         |
//! | `panels`  | paneles de flujo: cabecera, cajón, editor                  |
//! | `net`     | lado P2P: exportar/importar/publicar/recibir               |

mod modelo;
mod estado;
mod menu;
mod app;
mod map;
mod net;
mod panels;

use app::KhipuApp;

fn main() {
    llimphi_ui::run::<KhipuApp>();
}
