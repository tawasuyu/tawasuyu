/*
 * supay-core/scene_export.c — getters de estado interno de doomgeneric
 * para Fase 2 (supay-scene). El `build.rs` lo compila junto con el
 * resto del motor cuando `vendor/doomgeneric/` está presente.
 *
 * El motor C usa fixed-point 16.16 (FRACBITS=16) para coordenadas y
 * `angle_t` (uint32) para ángulos, cubriendo 0..2π linealmente. Acá
 * convertimos a `float` para que el lado Rust no tenga que conocer
 * esos detalles.
 *
 * Convenciones:
 * - Coordenadas y alturas en unidades Doom (1 unit ≈ 1 inch).
 * - Ángulos en radianes, 0 = mirando +X, antihorario.
 * - Light level 0..255 (clamp aplicado acá; el motor a veces sale del
 *   rango durante flickers/sector specials).
 * - Sectores y mobjs sin estado disponible (puntero NULL) devuelven 0
 *   en su getter — el lado Rust toma eso como "skip".
 *
 * Mobjs (sprites) requieren caminar la lista enlazada `thinkercap`. Un
 * mobj es un `thinker_t` cuyo `function.acp1 == P_MobjThinker`.
 * Cacheamos los punteros al consultar `supay_scene_num_sprites`
 * para que `supay_scene_sprite(i)` sea O(1). El cache se reconstruye
 * en cada `num_sprites` — uso normal: una llamada a num + N a get.
 *
 * Race-freedom: todas las funciones se llaman desde el mismo thread
 * que ejecuta `doomgeneric_Tick` (el host las invoca justo después
 * del tick). No hay acceso concurrente al estado del motor.
 */

#include <stddef.h>
#include <stdint.h>
#include <math.h>

#include "doomdef.h"
#include "doomtype.h"
#include "doomstat.h"    /* skyflatnum */
#include "tables.h"      /* angle_t */
#include "m_fixed.h"     /* fixed_t */
#include "r_defs.h"      /* line_t, sector_t, vertex_t, side_t, subsector_t, seg_t */
#include "p_mobj.h"      /* mobj_t */
#include "p_local.h"     /* P_MobjThinker (función action_p1 que distingue mobjs) */
#include "info.h"        /* sprnames[] — array de strings 4-char por spritenum_t */
#include "p_pspr.h"      /* pspdef_t, ps_weapon */
#include "d_player.h"    /* player_t, MAXPLAYERS */
#include "r_state.h"     /* lines/sectors/subsectors/segs/etc. globals */
#include "w_wad.h"       /* lumpinfo[i].name para resolver flats */

/* Globales del motor — declarados en r_state.h pero los re-extern-amos
 * acá por claridad de qué consumimos. */
extern int numlines;
extern line_t *lines;
extern int numsectors;
extern sector_t *sectors;
extern int numsubsectors;
extern subsector_t *subsectors;
extern int numsegs;
extern seg_t *segs;
extern int numnodes;
extern node_t *nodes;
extern player_t players[MAXPLAYERS];
extern int consoleplayer;
extern thinker_t thinkercap;
extern int skyflatnum;
extern int firstflat;
/* `lumpinfo` y `numlumps` están en w_wad.h pero los re-extern-amos
 * para claridad. */
extern int numtextures;
/* `textures` es `texture_t **` — array de punteros a `texture_t`.
 * Cada `texture_t` tiene `char name[8]` (sin nul terminator garantizado)
 * + width/height/patchcount. */
struct texture_s {
    char name[8];
    short width;
    short height;
    int index;
    void *next;
    short patchcount;
    /* patches[] follow; no los tocamos desde acá. */
};
extern struct texture_s **textures;
extern side_t *sides;
extern int numsides;

static inline float ftox(fixed_t v) {
    /* FRACUNIT = 1<<16 = 65536. División por constante el compilador
     * la convierte a multiplicación por 1/65536. */
    return (float)v / 65536.0f;
}

static inline float atorad(angle_t a) {
    /* angle_t cubre 0..2^32 linealmente — escalamos a 0..2π. */
    return (float)a * (6.2831853071795864769f / 4294967296.0f);
}

/* ---- Player ---- */

/* Devuelve 1 si el jugador tiene un mobj asignado (i.e. el mapa cargó
 * y el jugador está vivo en el mundo), 0 si no — el lado Rust deja el
 * snapshot vacío en ese caso. */
int supay_scene_player(float *x, float *y, float *z,
                       float *angle, float *view_height) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        return 0;
    }
    *x = ftox(p->mo->x);
    *y = ftox(p->mo->y);
    *z = ftox(p->mo->z);
    *angle = atorad(p->mo->angle);
    *view_height = ftox(p->viewheight);
    return 1;
}

/* Counters del player para los overlays de pantalla. Doom intercambia
 * PLAYPAL[1..13] cuando algo de esto está activo; nosotros sampleamos
 * siempre con PLAYPAL[0], así que la modernización es overlay alpha
 * sobre el frame final. El lado Rust convierte counters → alphas.
 *
 * - damagecount: 0..100, +N por hp de daño, decae 1/tick.
 * - bonuscount: 0..32, +6/+12 por pickup, decae 1/tick.
 * - power_invuln: tics restantes de invulnerabilidad (>0 = activo).
 * - power_radsuit: tics restantes de suit.
 *
 * Devuelve 0 si el jugador no existe (pre-mapa) — outs quedan en cero
 * y el renderer trata como "sin overlays". */
int supay_scene_player_overlays(int *damagecount, int *bonuscount,
                                int *power_invuln, int *power_radsuit) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        *damagecount = 0;
        *bonuscount = 0;
        *power_invuln = 0;
        *power_radsuit = 0;
        return 0;
    }
    *damagecount = p->damagecount;
    *bonuscount = p->bonuscount;
    *power_invuln = p->powers[pw_invulnerability];
    *power_radsuit = p->powers[pw_ironfeet];
    return 1;
}

/* Variante extendida — Fase 3.16. Suma `power_strength` (berserk pickup)
 * para el red tint que se desvanece a lo largo del nivel.
 *
 * En Doom, pw_strength es un contador que arranca grande (1) y crece
 * por tick (no decae); el tinte se calcula como `12 - (power_strength >> 6)`
 * con clamp — paleta más roja al recién agarrar el berserk, fade-out
 * suave después. */
int supay_scene_player_overlays_ext(int *damagecount, int *bonuscount,
                                    int *power_invuln, int *power_radsuit,
                                    int *power_strength) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        *damagecount = 0;
        *bonuscount = 0;
        *power_invuln = 0;
        *power_radsuit = 0;
        *power_strength = 0;
        return 0;
    }
    *damagecount = p->damagecount;
    *bonuscount = p->bonuscount;
    *power_invuln = p->powers[pw_invulnerability];
    *power_radsuit = p->powers[pw_ironfeet];
    *power_strength = p->powers[pw_strength];
    return 1;
}

/* Estado del segundo psprite — `ps_flash`. Doom lo usa para los muzzle
 * flashes de algunas armas (BFG, plasma, chaingun): un overlay extra
 * sobre `ps_weapon` que dura sólo 1-2 tics y agrega el destello.
 *
 * API espejo de `supay_scene_player_weapon`. Devuelve 0 si state es NULL.
 */
int supay_scene_player_flash(uint16_t *spritenum, uint8_t *frame,
                             float *sx, float *sy) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        return 0;
    }
    pspdef_t *psp = &p->psprites[ps_flash];
    if (!psp->state) {
        return 0;
    }
    *spritenum = (uint16_t)psp->state->sprite;
    int fr = psp->state->frame;
    uint8_t f = (uint8_t)(fr & 0x1F);
    if (fr & 0x8000) {
        f |= 0x80;
    }
    *frame = f;
    *sx = ftox(psp->sx);
    *sy = ftox(psp->sy);
    return 1;
}

/* Estado del psprite del arma del jugador (Fase 3.15). doomgeneric
 * mantiene `players[].psprites[ps_weapon]` con la animación de la
 * pistola/escopeta/etc. que el motor pintaría sobre la vista 2D.
 * El psprite tiene `state*` (None → inactivo), `tics`, y `sx/sy`
 * en coordenadas screen 320x200 (fixed-point 16.16).
 *
 * Devuelve 1 si el state está activo + outs llenados; 0 si está
 * inactivo (player dead, game start) → Rust lo trata como "sin arma".
 */
int supay_scene_player_weapon(uint16_t *spritenum, uint8_t *frame,
                              float *sx, float *sy) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        return 0;
    }
    pspdef_t *psp = &p->psprites[ps_weapon];
    if (!psp->state) {
        return 0;
    }
    *spritenum = (uint16_t)psp->state->sprite;
    /* state->frame puede tener el bit FF_FULLBRIGHT (bit 15) y FF_FRAMEMASK
     * los bits 0..14. Para nuestro `u8` extraemos los bits relevantes:
     * letter en bits 0..4, full-bright en bit 7 (convención existente). */
    int fr = psp->state->frame;
    uint8_t f = (uint8_t)(fr & 0x1F);
    if (fr & 0x8000) {
        f |= 0x80; /* full bright para muzzle flashes */
    }
    *frame = f;
    *sx = ftox(psp->sx);
    *sy = ftox(psp->sy);
    return 1;
}

/* Fase 3.20 — stats vitales del jugador para el HUD inferior.
 *
 * Outs:
 *   - health: 0..200 (puede llegar a 200 con sobrecarga; 0 = muerto).
 *   - armor_points: 0..200, blue = 200 max, green = 100 max.
 *   - armor_type: 0 = ninguno, 1 = green, 2 = blue (Doom convention).
 *   - ready_weapon: enum weapontype_t (wp_fist..wp_supershotgun = 0..8).
 *   - ammo[4]: clip / shell / cell / missile, balas actuales.
 *   - maxammo[4]: máximos con/sin backpack (Doom los actualiza al
 *     levantar la mochila duplicando los slots; el motor mantiene
 *     `maxammo[i]` que es el max efectivo en cada momento).
 *   - cards[6]: blue/yellow/red card + blue/yellow/red skull, en
 *     orden de `it_bluecard..it_redskull`. 0 = no tiene, 1 = tiene.
 *
 * Devuelve 0 si el jugador no existe (pre-mapa) — outs en cero, Rust
 * trata como "sin stats" y el HUD se pinta hueco. */
int supay_scene_player_stats(int *health, int *armor_points, int *armor_type,
                             int *ready_weapon,
                             int ammo[4], int maxammo[4],
                             uint8_t cards[6]) {
    player_t *p = &players[consoleplayer];
    if (!p->mo) {
        *health = 0;
        *armor_points = 0;
        *armor_type = 0;
        *ready_weapon = 0;
        for (int i = 0; i < 4; ++i) { ammo[i] = 0; maxammo[i] = 0; }
        for (int i = 0; i < 6; ++i) { cards[i] = 0; }
        return 0;
    }
    *health = p->health;
    *armor_points = p->armorpoints;
    *armor_type = p->armortype;
    *ready_weapon = (int)p->readyweapon;
    /* Compile-time check: NUMAMMO=4, NUMCARDS=6 a la fecha de doomgeneric;
     * si cambian, fallamos en build antes que en runtime. */
    _Static_assert(NUMAMMO == 4, "scene_export espera NUMAMMO==4");
    _Static_assert(NUMCARDS == 6, "scene_export espera NUMCARDS==6");
    for (int i = 0; i < 4; ++i) {
        ammo[i] = p->ammo[i];
        maxammo[i] = p->maxammo[i];
    }
    for (int i = 0; i < 6; ++i) {
        cards[i] = p->cards[i] ? 1 : 0;
    }
    return 1;
}

/* ---- Walls (linedefs) ---- */

int supay_scene_num_walls(void) {
    return numlines;
}

int supay_scene_wall(int i,
                     float *x1, float *y1, float *x2, float *y2,
                     uint32_t *front, uint32_t *back, uint32_t *flags) {
    if (i < 0 || i >= numlines || !lines) {
        return 0;
    }
    line_t *l = &lines[i];
    *x1 = ftox(l->v1->x);
    *y1 = ftox(l->v1->y);
    *x2 = ftox(l->v2->x);
    *y2 = ftox(l->v2->y);
    *front = l->frontsector
        ? (uint32_t)(l->frontsector - sectors)
        : 0xFFFFFFFFu;
    *back = l->backsector
        ? (uint32_t)(l->backsector - sectors)
        : 0xFFFFFFFFu;
    *flags = (uint32_t)l->flags;
    return 1;
}

/* ---- Sectors ---- */

int supay_scene_num_sectors(void) {
    return numsectors;
}

int supay_scene_sector(int i,
                       float *floor, float *ceiling, uint8_t *light,
                       uint16_t *floor_pic, uint16_t *ceiling_pic) {
    if (i < 0 || i >= numsectors || !sectors) {
        return 0;
    }
    sector_t *s = &sectors[i];
    *floor = ftox(s->floorheight);
    *ceiling = ftox(s->ceilingheight);
    int ll = s->lightlevel;
    if (ll < 0) ll = 0;
    if (ll > 255) ll = 255;
    *light = (uint8_t)ll;
    /* Aplica flattranslation cuando la tabla existe (post-R_InitFlats).
     * Doom anima los flats vía P_UpdateSpecials cada ~8 ticks rotando
     * flattranslation[base_pic] entre los frames de la familia
     * (NUKAGE1→NUKAGE2→NUKAGE3, FIREBLU1↔FIREBLU2, etc.). Devolvemos el
     * pic actual: el renderer Rust ve un floor_pic distinto cada ciclo
     * y resuelve el nombre del lump aparte vía DoomEngine::flat_name. */
    int fp = s->floorpic;
    int cp = s->ceilingpic;
    if (flattranslation) {
        fp = flattranslation[fp];
        cp = flattranslation[cp];
    }
    *floor_pic = (uint16_t)fp;
    *ceiling_pic = (uint16_t)cp;
    return 1;
}

/* ---- Mobjs (sprites visibles) ----
 *
 * El motor mantiene los thinkers en una lista doblemente enlazada
 * circular cuyo head/tail es `thinkercap`. Los mobjs son los thinkers
 * cuyo callback es `P_MobjThinker`. Cacheamos sus punteros para
 * permitir indexado O(1).
 */

#define SUPAY_MOBJ_CACHE_CAP 8192

static mobj_t *supay_mobj_cache[SUPAY_MOBJ_CACHE_CAP];
static int supay_mobj_cache_len = 0;

static void supay_mobj_cache_rebuild(void) {
    supay_mobj_cache_len = 0;
    thinker_t *th = thinkercap.next;
    while (th && th != &thinkercap
           && supay_mobj_cache_len < SUPAY_MOBJ_CACHE_CAP) {
        if (th->function.acp1 == (actionf_p1)P_MobjThinker) {
            supay_mobj_cache[supay_mobj_cache_len++] = (mobj_t *)th;
        }
        th = th->next;
    }
}

int supay_scene_num_sprites(void) {
    supay_mobj_cache_rebuild();
    return supay_mobj_cache_len;
}

/* ---- Subsectors + segs (Fase 3.2) ----
 *
 * Cada subsector es una hoja convexa del BSP que referencia un sector y
 * una corrida contigua de segs (`firstline`, `numlines`). Los segs son
 * los linesegs visibles que bordean la hoja — algunos lados pueden ser
 * particiones BSP internas y no tener seg, por lo que la cadena de segs
 * a veces no cierra el polígono completo (el lado faltante lo cubre el
 * subsector vecino del mismo sector). El renderer 3D (Fase 3.2) usa
 * estos polígonos para pintar pisos y techos reales por subsector.
 */

int supay_scene_num_subsectors(void) {
    return numsubsectors;
}

int supay_scene_subsector(int i, uint32_t *sector,
                          uint32_t *first_seg, uint32_t *num_segs) {
    if (i < 0 || i >= numsubsectors || !subsectors) {
        return 0;
    }
    subsector_t *ss = &subsectors[i];
    *sector = ss->sector ? (uint32_t)(ss->sector - sectors) : 0xFFFFFFFFu;
    *first_seg = (uint32_t)ss->firstline;
    *num_segs = (uint32_t)ss->numlines;
    return 1;
}

int supay_scene_num_segs(void) {
    return numsegs;
}

int supay_scene_seg(int i,
                    float *x1, float *y1, float *x2, float *y2) {
    if (i < 0 || i >= numsegs || !segs) {
        return 0;
    }
    seg_t *s = &segs[i];
    *x1 = ftox(s->v1->x);
    *y1 = ftox(s->v1->y);
    *x2 = ftox(s->v2->x);
    *y2 = ftox(s->v2->y);
    return 1;
}

/* ---- BSP nodes (Fase 3.13) ----
 *
 * El motor mantiene el árbol BSP en `nodes[]`. Cada nodo tiene una línea
 * de partición (origin x/y + dx/dy) y dos hijos. Cada hijo es:
 *   - subsector si `child & NF_SUBSECTOR` (0x8000), índice = `child & ~NF_SUBSECTOR`
 *   - nodo interno si no, índice = `child`
 * La raíz es `nodes[numnodes - 1]`.
 *
 * El renderer Rust camina el árbol back-to-front desde la posición del
 * jugador para asignar orden de painter's correcto a los polígonos de
 * subsector. La función supay_scene_node devuelve todos los campos en una
 * sola llamada para minimizar el costo de FFI por nodo (numnodes típicos
 * ≈ #subsectors - 1, ~200-500 en E1Mx).
 *
 * Convención: el lado del frente del partición está al **lado +z del
 * cross product** `(dx, dy) × (px - x, py - y)`, i.e. el viewer está al
 * front si `dx·(py - y) - dy·(px - x) > 0`. Es la misma convención que
 * `R_PointOnSide` en r_main.c. Cuando el viewer está al front, children[0]
 * es el subtree del front y children[1] es el subtree del back.
 */

int supay_scene_num_nodes(void) {
    return numnodes;
}

int supay_scene_node(int i,
                     float *x, float *y, float *dx, float *dy,
                     uint16_t *child_front, uint16_t *child_back) {
    if (i < 0 || i >= numnodes || !nodes) {
        return 0;
    }
    node_t *n = &nodes[i];
    *x = ftox(n->x);
    *y = ftox(n->y);
    *dx = ftox(n->dx);
    *dy = ftox(n->dy);
    *child_front = n->children[0];
    *child_back = n->children[1];
    return 1;
}

/* Índice del "sky flat" (ceiling_pic == this → renderear cielo). El motor
 * lo resuelve en R_InitFlats vía W_GetNumForName("F_SKY1"). Antes de que
 * el mapa cargue, vale -1 — devolvemos 0xFFFF como sentinel. */
uint16_t supay_scene_sky_pic(void) {
    return skyflatnum < 0 ? 0xFFFFu : (uint16_t)skyflatnum;
}

/* Resuelve `pic_idx` (índice relativo a la tabla de flats del motor) al
 * nombre del lump (8 chars + nul terminator). Devuelve 1 si éxito, 0 si
 * el índice está fuera de rango o `lumpinfo` aún no fue inicializado.
 *
 * Doomgeneric mantiene los flats como lumps consecutivos a partir de
 * `firstflat` (set en R_InitFlats). El nombre de un flat con índice
 * relativo `i` está en `lumpinfo[firstflat + i].name`. El renderer
 * Rust lo usa para cachear el color promedio del flat resuelto contra
 * la paleta PLAYPAL del WAD que parsea aparte (supay-wad).
 */
/* Resuelve `spritenum` al string 4-char de `sprnames[]`
 * (e.g. spritenum_t SPR_TROO=29 → "TROO"). El renderer combina ese
 * nombre con el `frame` letter + ángulo para encontrar el lump del
 * sprite (e.g. "TROOA1"). Devuelve 1 si OK, 0 si fuera de rango.
 *
 * Nota: NUMSPRITES (info.h) marca el fin del array; sprnames[NUMSPRITES]
 * es NULL como terminador, así también verificamos eso. */
int supay_scene_sprite_name(uint16_t spritenum, char out[5]) {
    if (spritenum >= NUMSPRITES) {
        return 0;
    }
    const char *src = sprnames[spritenum];
    if (!src) {
        return 0;
    }
    for (int i = 0; i < 4; i++) {
        out[i] = src[i];
        if (!src[i]) {
            /* sprnames son siempre 4 chars (DDDD); padding por si acaso. */
            for (int j = i; j < 4; j++) out[j] = '\0';
            break;
        }
    }
    out[4] = '\0';
    return 1;
}

/* Resuelve una textura de pared al nombre del lump TEXTURE1.
 *
 * `wall_idx` = índice en `lines[]`.
 * `side`     = 0=front (sidenum[0]), 1=back (sidenum[1]).
 * `kind`     = 0=middle, 1=upper, 2=lower.
 *
 * Devuelve 1 si OK + nombre escrito a `out` (8 chars + nul); 0 si:
 *   - wall fuera de rango, motor sin mapa cargado, sidedef inexistente
 *   - textura id 0 ("no texture", convencion Doom para slots vacíos)
 *
 * Notas:
 *   - sidenum[1] == -1 cuando la pared es one-sided.
 *   - El renderer Rust prueba el nombre directamente como texture
 *     compuesta (supay-wad::texture); si no existe, cae al color.
 */
int supay_scene_wall_texture(int wall_idx, int side, int kind, char out[9]) {
    if (!lines || wall_idx < 0 || wall_idx >= numlines) {
        return 0;
    }
    if (side != 0 && side != 1) {
        return 0;
    }
    line_t *l = &lines[wall_idx];
    short sn = l->sidenum[side];
    if (sn < 0 || sn >= numsides || !sides) {
        return 0;
    }
    side_t *sd = &sides[sn];
    short tex_id;
    switch (kind) {
        case 0: tex_id = sd->midtexture; break;
        case 1: tex_id = sd->toptexture; break;
        case 2: tex_id = sd->bottomtexture; break;
        default: return 0;
    }
    if (tex_id <= 0 || tex_id >= numtextures || !textures || !textures[tex_id]) {
        return 0;
    }
    /* Aplica texturetranslation cuando la tabla existe — los switches
     * activan/desactivan vía P_ChangeSwitchTexture, que setea esta
     * tabla. Asegura que un switch presionado refleje su version
     * "off"/"on" en el renderer (familias SW1xxx y SW2xxx). */
    if (texturetranslation) {
        short t2 = (short)texturetranslation[tex_id];
        if (t2 > 0 && t2 < numtextures && textures[t2]) {
            tex_id = t2;
        }
    }
    char *src = textures[tex_id]->name;
    for (int i = 0; i < 8; i++) {
        out[i] = src[i];
        if (!src[i]) {
            for (int j = i; j < 8; j++) out[j] = '\0';
            break;
        }
    }
    out[8] = '\0';
    return 1;
}

/* Resuelve los offsets de textura (`sidedef.textureoffset` y
 * `sidedef.rowoffset`) para `(wall_idx, side)`. Doom guarda ambos en
 * fixed-point 16.16; los convertimos a float (división por 65536).
 *
 * side = 0 (front) o 1 (back).
 *
 * Devuelve 1 si OK + valores escritos a `*xoff`/`*yoff`; 0 si fuera de
 * rango o sidedef inexistente. En ese caso ambos quedan en 0 — el
 * renderer hace `[f32; 2]` con default cero, equivalente al
 * comportamiento Doom de "sin offset".
 */
int supay_scene_wall_offsets(int wall_idx, int side, float *xoff, float *yoff) {
    if (!lines || wall_idx < 0 || wall_idx >= numlines) {
        return 0;
    }
    if (side != 0 && side != 1) {
        return 0;
    }
    line_t *l = &lines[wall_idx];
    short sn = l->sidenum[side];
    if (sn < 0 || sn >= numsides || !sides) {
        return 0;
    }
    side_t *sd = &sides[sn];
    *xoff = ftox(sd->textureoffset);
    *yoff = ftox(sd->rowoffset);
    return 1;
}

int supay_scene_flat_name(uint16_t pic_idx, char out[9]) {
    if (!lumpinfo || firstflat <= 0) {
        return 0;
    }
    unsigned int lump = (unsigned int)firstflat + (unsigned int)pic_idx;
    if (lump >= numlumps) {
        return 0;
    }
    /* lumpinfo[].name son 8 chars sin nul terminator garantizado;
     * copiamos y agregamos el nul al final. */
    for (int i = 0; i < 8; i++) {
        out[i] = lumpinfo[lump].name[i];
    }
    out[8] = '\0';
    return 1;
}

int supay_scene_sprite(int i,
                       float *x, float *y, float *z, float *angle,
                       uint16_t *sprite, uint8_t *frame, uint32_t *sector) {
    if (i < 0 || i >= supay_mobj_cache_len) {
        return 0;
    }
    mobj_t *m = supay_mobj_cache[i];
    *x = ftox(m->x);
    *y = ftox(m->y);
    *z = ftox(m->z);
    *angle = atorad(m->angle);
    *sprite = (uint16_t)m->sprite;
    *frame = (uint8_t)m->frame;
    /* mobj.subsector apunta al subsector donde cae; subsector.sector
     * apunta al sector que lo contiene. En frames antes de que P_SetThingPosition
     * corra puede ser NULL — devolvemos 0 (índice del primer sector). */
    *sector = (m->subsector && m->subsector->sector)
        ? (uint32_t)(m->subsector->sector - sectors)
        : 0u;
    return 1;
}
