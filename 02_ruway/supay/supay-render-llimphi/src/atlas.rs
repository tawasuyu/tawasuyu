use super::*;

/// Atlas de assets resueltos desde el WAD para que el renderer no
/// tenga que hablar con `supay-wad` por frame. Construir con
/// [`WadAtlas::new`] una vez al inicio del host y compartir por `Arc`.
///
/// El cache de colores por nombre de flat es interno y lazy — la
/// primera vez que un flat se consulta calculamos su `flat_average_color`
/// y lo guardamos.
pub struct WadAtlas {
    wad: supay_wad::Wad,
    palette: [(u8, u8, u8); supay_wad::PALETTE_ENTRIES],
    /// Estado mutable interior — flat_names + color_cache bajo un
    /// único `Mutex` para que el host pueda registrar pic_idx nuevos
    /// (`set_flat_name`) sin tener que clonar/reconstruir el Arc
    /// compartido con el renderer.
    inner: Mutex<AtlasInner>,
}

#[derive(Default)]
pub(crate) struct AtlasInner {
    /// Lookup pic_idx (u16) → nombre del flat. Se llena on-demand
    /// vía `DoomEngine::flat_name(i)` la primera vez que el host ve
    /// un pic_idx en algún sector.
    flat_names: HashMap<u16, String>,
    /// Cache lazy: pic_idx → color promedio resuelto.
    color_cache: HashMap<u16, Option<(u8, u8, u8)>>,
    /// Lookup spritenum (u16) → 4-char base name del sprite (e.g.
    /// "TROO"). Llenado por el host con `DoomEngine::sprite_name(n)`
    /// la primera vez que el host ve un `SpriteSnap` con ese sprite.
    sprite_names: HashMap<u16, String>,
    /// Cache de patches decodificados por (spritenum, frame_letter,
    /// angle). `frame_letter` viene del bit 0..4 del `frame` del mobj
    /// (A..Z = 0..25); `angle` es 1..8 (Doom convention: 1=front,
    /// 5=back). Valor: `Option<(Arc<Patch>, mirror_flag)>` — mirror
    /// indica que el patch corresponde a un lump combinado tipo
    /// `TROOA2A8` y debe pintarse horizontalmente espejado.
    sprite_patches: HashMap<(u16, u8, u8), Option<(Arc<supay_wad::Patch>, bool)>>,
    /// Cache de texturas de pared compuestas por nombre. `None` para
    /// nombres que no resuelven en TEXTURE1.
    wall_textures: HashMap<String, Option<Arc<supay_wad::Texture>>>,
    /// Cache de flats expandidos a RGBA8 (64×64×4 = 16 KB) por pic_idx.
    flat_rgbas: HashMap<u16, Option<Arc<Vec<u8>>>>,
}

impl std::fmt::Debug for WadAtlas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names = self.inner.lock().map(|i| i.flat_names.len()).unwrap_or(0);
        f.debug_struct("WadAtlas")
            .field("lumps", &self.wad.len())
            .field("flat_names", &names)
            .finish()
    }
}

impl WadAtlas {
    /// Construye el atlas desde un WAD ya parseado. El mapa
    /// `pic_idx → flat_name` arranca vacío; el host lo va llenando
    /// con [`Self::set_flat_name`] conforme el motor expone los
    /// pic_idx del mapa cargado.
    pub fn new(wad: supay_wad::Wad, flat_names: HashMap<u16, String>) -> Self {
        let palette = wad.palette();
        Self {
            wad,
            palette,
            inner: Mutex::new(AtlasInner {
                flat_names,
                color_cache: HashMap::new(),
                sprite_names: HashMap::new(),
                sprite_patches: HashMap::new(),
                wall_textures: HashMap::new(),
                flat_rgbas: HashMap::new(),
            }),
        }
    }

    /// Recupera el color promedio para un `pic_idx`. Devuelve `None`
    /// si el nombre del flat no está mapeado o si el flat no existe
    /// en el WAD (e.g. el placeholder `F_SKY1` que no tiene bytes).
    pub fn flat_color(&self, pic_idx: u16) -> Option<(u8, u8, u8)> {
        let Ok(mut inner) = self.inner.lock() else {
            return None;
        };
        if let Some(&cached) = inner.color_cache.get(&pic_idx) {
            return cached;
        }
        let resolved = inner
            .flat_names
            .get(&pic_idx)
            .and_then(|n| self.wad.flat_average_color(n, &self.palette));
        inner.color_cache.insert(pic_idx, resolved);
        resolved
    }

    /// Registra (o sobreescribe) el nombre del flat para `pic_idx`.
    /// Invalida la entrada cacheada para ese índice. Toma `&self` —
    /// la interior mutability permite hacerlo desde un `Arc<Self>`
    /// compartido con el renderer.
    pub fn set_flat_name(&self, pic_idx: u16, name: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.flat_names.insert(pic_idx, name);
            inner.color_cache.remove(&pic_idx);
            inner.flat_rgbas.remove(&pic_idx);
        }
    }

    /// `true` si `pic_idx` ya fue registrado vía `set_flat_name`.
    pub fn has_flat_name(&self, pic_idx: u16) -> bool {
        self.inner
            .lock()
            .map(|i| i.flat_names.contains_key(&pic_idx))
            .unwrap_or(false)
    }

    /// Nombre del flat para `pic_idx` (registrado vía `set_flat_name`), o
    /// `None` si todavía no se vio. Lo usa el renderer wgpu para detectar
    /// flats líquidos (NUKAGE/FWATER/LAVA/BLOOD/SLIME…) y animarlos.
    pub fn flat_name(&self, pic_idx: u16) -> Option<String> {
        self.inner
            .lock()
            .ok()
            .and_then(|i| i.flat_names.get(&pic_idx).cloned())
    }

    /// Registra el 4-char name del sprite para un `spritenum`. Usado
    /// por el host análogo a [`Self::set_flat_name`]. Invalida los
    /// patches cacheados para ese spritenum (por si los frames
    /// dependían del nombre viejo).
    pub fn set_sprite_name(&self, spritenum: u16, name: String) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.sprite_names.insert(spritenum, name);
            inner.sprite_patches.retain(|(s, _, _), _| *s != spritenum);
        }
    }

    pub fn has_sprite_name(&self, spritenum: u16) -> bool {
        self.inner
            .lock()
            .map(|i| i.sprite_names.contains_key(&spritenum))
            .unwrap_or(false)
    }

    /// Devuelve el 4-char name del sprite si fue registrado vía
    /// [`Self::set_sprite_name`]. Usado por el renderer para resolver
    /// el tinte característico de cada mobj FF_FULLBRIGHT (Fase 3.27).
    pub fn sprite_name(&self, spritenum: u16) -> Option<String> {
        self.inner
            .lock()
            .ok()?
            .sprite_names
            .get(&spritenum)
            .cloned()
    }

    /// Recupera (decodificando si hace falta y cacheando) el patch
    /// RGBA para el sprite `spritenum` en `frame` (bits 0..4 = letter
    /// A..Z; bit 7 = full bright, ignorado por ahora) y `angle` (1..8).
    ///
    /// Devuelve `Some((patch, mirror))` o `None` si no se encuentra
    /// ningún lump razonable. `mirror=true` indica que el lump
    /// corresponde a un combinado tipo `TROOA2A8` y debe pintarse
    /// horizontalmente espejado.
    pub fn sprite_patch(
        &self,
        spritenum: u16,
        frame: u8,
        angle: u8,
    ) -> Option<(Arc<supay_wad::Patch>, bool)> {
        let letter = frame & 0x1F;
        let angle = angle.clamp(1, 8);
        let key = (spritenum, letter, angle);
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.sprite_patches.get(&key) {
                return cached.clone();
            }
        }
        let name = {
            let inner = self.inner.lock().ok()?;
            inner.sprite_names.get(&spritenum).cloned()?
        };
        let frame_char = (b'A' + letter) as char;
        // `sprite_lump` cubre los tres casos de naming + mirror.
        let resolved = self.wad.sprite_lump(&name, frame_char, angle);
        let decoded: Option<(Arc<supay_wad::Patch>, bool)> = resolved.and_then(|(lump_name, mirror)| {
            self.wad
                .patch_rgba(&lump_name, &self.palette)
                .map(|p| (Arc::new(p), mirror))
        });
        if let Ok(mut inner) = self.inner.lock() {
            inner.sprite_patches.insert(key, decoded.clone());
        }
        decoded
    }

    /// Recupera (decodificando + cacheando) el RGBA del flat 64×64
    /// para `pic_idx`. Devuelve `None` si el nombre del flat no está
    /// mapeado o no existe en el WAD (e.g. F_SKY1 placeholder).
    /// El renderer usa esto para texturizar pisos/techos.
    pub fn flat_rgba(&self, pic_idx: u16) -> Option<Arc<Vec<u8>>> {
        // Reusamos el color_cache para evitar duplicar lookups; lo
        // dejamos sin tocar porque el RGBA es ortogonal al color.
        // Cache propia para flats: el HashMap nuevo `flat_rgbas`.
        // De momento simplificamos: re-decodificamos por idx — son
        // 64×64=4 KB por flat resuelto, y `inner.flat_rgbas` cachea.
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.flat_rgbas.get(&pic_idx) {
                return cached.clone();
            }
        }
        let name = {
            let inner = self.inner.lock().ok()?;
            inner.flat_names.get(&pic_idx).cloned()
        }?;
        let decoded = self.wad.flat_rgba(&name, &self.palette).map(Arc::new);
        if let Ok(mut inner) = self.inner.lock() {
            inner.flat_rgbas.insert(pic_idx, decoded.clone());
        }
        decoded
    }

    /// Recupera (decodificando + cacheando) la textura de pared
    /// compuesta `name` (de TEXTURE1). Devuelve `None` si no existe
    /// o no parsea. Cache: `Some(Arc<Texture>)` o `None` para misses.
    pub fn wall_texture(&self, name: &str) -> Option<Arc<supay_wad::Texture>> {
        let key = name.to_ascii_uppercase();
        if let Ok(inner) = self.inner.lock() {
            if let Some(cached) = inner.wall_textures.get(&key) {
                return cached.clone();
            }
        }
        let decoded = self.wad.texture(&key, &self.palette).map(Arc::new);
        if let Ok(mut inner) = self.inner.lock() {
            inner.wall_textures.insert(key, decoded.clone());
        }
        decoded
    }

    /// Acceso al WAD interno (para features futuras como wall
    /// texturing samplear patches sin reabrir).
    pub fn wad(&self) -> &supay_wad::Wad {
        &self.wad
    }
}
