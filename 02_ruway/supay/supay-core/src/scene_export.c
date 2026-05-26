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
#include "d_player.h"    /* player_t, MAXPLAYERS */
#include "r_state.h"     /* lines/sectors/subsectors/segs/etc. globals */

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
extern player_t players[MAXPLAYERS];
extern int consoleplayer;
extern thinker_t thinkercap;
extern int skyflatnum;

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
    *floor_pic = (uint16_t)s->floorpic;
    *ceiling_pic = (uint16_t)s->ceilingpic;
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

/* Índice del "sky flat" (ceiling_pic == this → renderear cielo). El motor
 * lo resuelve en R_InitFlats vía W_GetNumForName("F_SKY1"). Antes de que
 * el mapa cargue, vale -1 — devolvemos 0xFFFF como sentinel. */
uint16_t supay_scene_sky_pic(void) {
    return skyflatnum < 0 ? 0xFFFFu : (uint16_t)skyflatnum;
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
