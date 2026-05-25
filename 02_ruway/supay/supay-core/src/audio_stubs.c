/*
 * supay-core/audio_stubs.c — stubs no-op de la API de audio que
 * `i_sound.c` proveería si lo compiláramos. Como `i_sound.c` arrastra
 * `<SDL_mixer.h>` no podemos compilarlo sin SDL en el sistema, así
 * que proveemos acá las funciones que d_main.c / s_sound.c esperan
 * resolver. El motor corre silencioso pero completo.
 *
 * Cuando agreguemos audio real (`cpal` desde Rust, o un backend
 * propio), reemplazamos estos stubs por llamadas a callbacks.
 */

#include <stddef.h>

typedef int boolean;
typedef struct sfxinfo_struct sfxinfo_t;
typedef void (*I_BindVariable_fn)(const char *, void *);

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
    (void)sfxinfo; (void)channel; (void)vol; (void)sep;
    return -1;
}
void I_StopSound(int channel) { (void)channel; }
boolean I_SoundIsPlaying(int channel) { (void)channel; return 0; }
void I_PrecacheSounds(sfxinfo_t *sounds, int num_sounds) {
    (void)sounds; (void)num_sounds;
}

/* ---- Music API ---- */
void  I_InitMusic(void) {}
void  I_ShutdownMusic(void) {}
void  I_SetMusicVolume(int volume) { (void)volume; }
void  I_PauseSong(void) {}
void  I_ResumeSong(void) {}
void *I_RegisterSong(void *data, int len) {
    (void)data; (void)len;
    return NULL;
}
void  I_UnRegisterSong(void *handle) { (void)handle; }
void  I_PlaySong(void *handle, boolean looping) {
    (void)handle; (void)looping;
}
void  I_StopSong(void) {}
boolean I_MusicIsPlaying(void) { return 0; }
