/*
 * supay-core/audio_stubs.c — implementación de la API de audio que
 * `i_sound.c` proveería si lo compiláramos. Como `i_sound.c` arrastra
 * `<SDL_mixer.h>` no podemos compilarlo sin SDL en el sistema, así
 * que proveemos acá las funciones que d_main.c / s_sound.c esperan
 * resolver.
 *
 * Fase 4.0: en vez de no-op, `I_StartSound` graba el evento (lump
 * `name`, vol, sep) en un ring buffer que Rust drena con
 * `supay_sound_poll` cada tick, resuelve `DS<name>` del WAD y lo
 * mezcla con cpal (`supay-audio`). El motor ya llama I_StartSound en
 * los momentos correctos del gameplay — sólo interceptamos el evento.
 * La música (MUS/MIDI) sigue no-op (Fase 4.1+).
 */

#include <stddef.h>
#include <string.h>

typedef int boolean;

/* Mirror mínimo del prefijo de `struct sfxinfo_struct` (i_sound.h).
 * Sólo leemos `name` (el lump base, e.g. "pistol"), que va justo tras
 * `char *tagname`, así que este layout parcial coincide con el real
 * para leer a través del puntero que el motor nos pasa. El resto de
 * la struct (priority, link, ...) no nos importa. */
struct sfxinfo_struct {
    char *tagname;
    char  name[9];
};
typedef struct sfxinfo_struct sfxinfo_t;
typedef void (*I_BindVariable_fn)(const char *, void *);

/* ---- Puente de eventos de sonido hacia Rust ---- */
typedef struct {
    char name[9];
    int  vol;   /* 0..127 */
    int  sep;   /* 0..255, 128 ≈ centro */
} supay_snd_event;

#define SUPAY_SND_RING 64
static supay_snd_event supay_snd_ring[SUPAY_SND_RING];
static int supay_snd_head = 0; /* índice de escritura */
static int supay_snd_tail = 0; /* índice de lectura  */

static void supay_snd_push(sfxinfo_t *sfxinfo, int vol, int sep) {
    int next = (supay_snd_head + 1) % SUPAY_SND_RING;
    if (next == supay_snd_tail) {
        /* Ring lleno: dropeamos el más viejo (avanzamos tail). El
         * audio es best-effort; perder un sfx en un pico es aceptable. */
        supay_snd_tail = (supay_snd_tail + 1) % SUPAY_SND_RING;
    }
    supay_snd_event *e = &supay_snd_ring[supay_snd_head];
    if (sfxinfo) {
        memcpy(e->name, sfxinfo->name, 9);
        e->name[8] = '\0';
    } else {
        e->name[0] = '\0';
    }
    e->vol = vol;
    e->sep = sep;
    supay_snd_head = next;
}

/* Drena hasta `max` eventos al array `out`. Devuelve cuántos copió.
 * Mismo thread que el tick → sin sincronización. */
int supay_sound_poll(supay_snd_event *out, int max) {
    int n = 0;
    while (supay_snd_tail != supay_snd_head && n < max) {
        out[n++] = supay_snd_ring[supay_snd_tail];
        supay_snd_tail = (supay_snd_tail + 1) % SUPAY_SND_RING;
    }
    return n;
}

/* ---- variables de configuración que d_main.c y otros leen. ---- */
int snd_sfxdevice    = 0;
int snd_musicdevice  = 0;
int snd_samplerate   = 0;
int snd_cachesize    = 0;
int snd_maxslicetime_ms = 0;
char *snd_musiccmd   = "";
char *snd_dmxoption  = "";
int use_libsamplerate = 0;
float libsamplerate_scale = 1.0f;

/* M_BindIntVariable / M_BindStringVariable son del motor; el stub
 * existe sólo para que I_BindSoundVariables no rompa el enlace. */
void I_BindSoundVariables(void) {}

/* ---- Sound API ---- */
void I_InitSound(boolean use_sfx_prefix) { (void)use_sfx_prefix; }
void I_ShutdownSound(void) {}
int  I_GetSfxLumpNum(sfxinfo_t *sfxinfo) { (void)sfxinfo; return 0; }
void I_UpdateSound(void) {}
void I_UpdateSoundParams(int channel, int vol, int sep) {
    (void)channel; (void)vol; (void)sep;
}
int  I_StartSound(sfxinfo_t *sfxinfo, int channel, int vol, int sep) {
    supay_snd_push(sfxinfo, vol, sep);
    /* Devolvemos el `channel` como handle. I_SoundIsPlaying siempre
     * dice "no suena" (one-shot fire-and-forget en Rust), así que el
     * motor recicla el canal de inmediato — correcto para sfx cortos. */
    return channel;
}
void I_StopSound(int channel) { (void)channel; }
boolean I_SoundIsPlaying(int channel) { (void)channel; return 0; }
void I_PrecacheSounds(sfxinfo_t *sounds, int num_sounds) {
    (void)sounds; (void)num_sounds;
}

/* ---- Music API (Fase 4.1) ----
 * `I_RegisterSong` recibe el lump MUS crudo cacheado por s_sound.c.
 * Lo copiamos a un buffer estático; `I_PlaySong`/`I_StopSong` cambian
 * el estado + un contador de generación. Rust consulta `supay_music_gen`
 * (barato) cada tick y, si cambió, drena el buffer con `supay_music_poll`
 * para arrancar/parar la música en el synth de `supay-audio`. */
#define SUPAY_MUS_MAX (256 * 1024)
static unsigned char supay_mus_buf[SUPAY_MUS_MAX];
static int          supay_mus_len  = 0;
static int          supay_mus_play = 0;
static int          supay_mus_loop = 0;
static unsigned int supay_mus_gen  = 0;

void  I_InitMusic(void) {}
void  I_ShutdownMusic(void) {}
void  I_SetMusicVolume(int volume) { (void)volume; }
void  I_PauseSong(void) {}   /* MVP: la música sigue (sin pausa real) */
void  I_ResumeSong(void) {}
void *I_RegisterSong(void *data, int len) {
    int n = len;
    if (n < 0) n = 0;
    if (n > SUPAY_MUS_MAX) n = SUPAY_MUS_MAX;
    if (data && n > 0) {
        memcpy(supay_mus_buf, data, n);
    }
    supay_mus_len = n;
    return (void *)1; /* handle no-nulo: el motor lo trata como válido */
}
void  I_UnRegisterSong(void *handle) { (void)handle; }
void  I_PlaySong(void *handle, boolean looping) {
    (void)handle;
    supay_mus_play = 1;
    supay_mus_loop = looping ? 1 : 0;
    supay_mus_gen++;
}
void  I_StopSong(void) {
    supay_mus_play = 0;
    supay_mus_gen++;
}
boolean I_MusicIsPlaying(void) { return supay_mus_play; }

/* Contador de generación: cambia en cada PlaySong/StopSong. Rust lo
 * compara con el último visto para detectar cambios sin copiar el buffer. */
unsigned int supay_music_gen(void) { return supay_mus_gen; }

/* Drena el estado de música: copia hasta `max` bytes del lump a `out`
 * (sólo si está sonando), setea len/play/loop. Devuelve la generación. */
unsigned int supay_music_poll(unsigned char *out, int max,
                              int *out_len, int *out_play, int *out_loop) {
    int n = supay_mus_len;
    if (n > max) n = max;
    if (out_play) *out_play = supay_mus_play;
    if (out_loop) *out_loop = supay_mus_loop;
    if (supay_mus_play && out && n > 0) {
        memcpy(out, supay_mus_buf, n);
    }
    if (out_len) *out_len = n;
    return supay_mus_gen;
}
