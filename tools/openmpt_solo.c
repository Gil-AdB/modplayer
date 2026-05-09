/* openmpt_solo: render a tracker module via libopenmpt with optional
 * per-channel mute/solo. Output is 48 kHz stereo float32 WAV — same
 * format as our `render_wav` so the sweep harness can compare voice
 * by voice.
 *
 * Why a standalone tool: openmpt123 has no CLI flag to mute or solo
 * channels. The libopenmpt extension interface exposes
 * `set_channel_mute_status` per channel, and `openmpt_module_ext`
 * gets us there in a few calls. ~150 lines, no build system needed.
 *
 * Usage:
 *   openmpt_solo <module> <output.wav> [--solo CH] [--mute CH,CH...]
 *                                       [--end-time SEC] [--vu-trace FILE]
 *
 *   --solo CH        Mute every channel EXCEPT CH (0-indexed). May be
 *                    combined with --mute (mutes are applied last).
 *   --mute LIST      Comma-separated 0-indexed channel indices to mute.
 *   --end-time SEC   Stop after SEC seconds (default: full song).
 *   --vu-trace FILE  Write per-channel VU readings to a TSV file. One
 *                    row per render block (~85 ms): frame, t_seconds,
 *                    then VU per channel (mono). The "vol" column for
 *                    a muted channel goes to 0; for live channels it's
 *                    a 0..1 float. Useful to diff against our
 *                    state_dump's `Vraw` per-tick trace.
 *
 * Build: see tools/build_openmpt_solo.sh
 */

#include <libopenmpt/libopenmpt.h>
#include <libopenmpt/libopenmpt_ext.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#define RATE 48000
#define BUFLEN 4096

static void log_func(const char *msg, void *user) { (void)user; fprintf(stderr, "openmpt: %s\n", msg); }

/* Minimal WAV writer for IEEE float32 stereo. Same on-disk shape as
 * `xmplayer/src/bin/render_wav.rs::write_wav_header` so downstream
 * tools (scripts/sweep.py / scripts/fft_compare.py / ffmpeg) treat
 * the two engines' outputs identically. */
static int write_wav_header(FILE *f, uint32_t total_frames) {
    uint32_t data_bytes = total_frames * 2 * 4;
    uint32_t byte_rate = RATE * 2 * 4;
    uint16_t block_align = 2 * 4;
    uint16_t bps = 32;
    uint16_t fmt_code = 3; /* IEEE float */
    uint16_t channels = 2;
    uint32_t fmt_size = 16;
    uint32_t riff_size = 36 + data_bytes;
    uint32_t omt_rate = RATE;
    if (fwrite("RIFF", 1, 4, f) != 4) return -1;
    if (fwrite(&riff_size, 4, 1, f) != 1) return -1;
    if (fwrite("WAVE", 1, 4, f) != 4) return -1;
    if (fwrite("fmt ", 1, 4, f) != 4) return -1;
    if (fwrite(&fmt_size, 4, 1, f) != 1) return -1;
    if (fwrite(&fmt_code, 2, 1, f) != 1) return -1;
    if (fwrite(&channels, 2, 1, f) != 1) return -1;
    if (fwrite(&omt_rate, 4, 1, f) != 1) return -1;
    if (fwrite(&byte_rate, 4, 1, f) != 1) return -1;
    if (fwrite(&block_align, 2, 1, f) != 1) return -1;
    if (fwrite(&bps, 2, 1, f) != 1) return -1;
    if (fwrite("data", 1, 4, f) != 4) return -1;
    if (fwrite(&data_bytes, 4, 1, f) != 1) return -1;
    return 0;
}

/* Parse "1,3,5" into out[]; return count, -1 on error. */
static int parse_channel_list(const char *s, int *out, int max) {
    int n = 0;
    char *copy = strdup(s);
    char *tok = strtok(copy, ",");
    while (tok && n < max) {
        char *end;
        long v = strtol(tok, &end, 10);
        if (end == tok || v < 0) { free(copy); return -1; }
        out[n++] = (int)v;
        tok = strtok(NULL, ",");
    }
    free(copy);
    return n;
}

int main(int argc, char **argv) {
    if (argc < 3) {
        fprintf(stderr, "usage: %s <module> <output.wav> [--solo CH] [--mute LIST] [--end-time SEC]\n", argv[0]);
        return 2;
    }
    const char *mod_path = argv[1];
    const char *out_path = argv[2];
    int solo_channel = -1;
    int mute_list[64];
    int mute_count = 0;
    double end_time = 0.0;
    const char *vu_trace_path = NULL;
    for (int i = 3; i < argc; i++) {
        if (!strcmp(argv[i], "--solo") && i + 1 < argc) {
            solo_channel = atoi(argv[++i]);
        } else if (!strcmp(argv[i], "--mute") && i + 1 < argc) {
            mute_count = parse_channel_list(argv[++i], mute_list, 64);
            if (mute_count < 0) { fprintf(stderr, "bad --mute list\n"); return 2; }
        } else if (!strcmp(argv[i], "--end-time") && i + 1 < argc) {
            end_time = atof(argv[++i]);
        } else if (!strcmp(argv[i], "--vu-trace") && i + 1 < argc) {
            vu_trace_path = argv[++i];
        } else {
            fprintf(stderr, "unknown flag: %s\n", argv[i]);
            return 2;
        }
    }

    FILE *fmod = fopen(mod_path, "rb");
    if (!fmod) { perror("open module"); return 1; }
    fseek(fmod, 0, SEEK_END);
    long size = ftell(fmod);
    rewind(fmod);
    void *buf = malloc(size);
    if (!buf || fread(buf, 1, size, fmod) != (size_t)size) { perror("read module"); fclose(fmod); return 1; }
    fclose(fmod);

    int err = OPENMPT_ERROR_OK;
    const char *err_msg = NULL;
    openmpt_module_ext *ext = openmpt_module_ext_create_from_memory(
        buf, size, log_func, NULL, NULL, NULL, &err, &err_msg, NULL);
    free(buf);
    if (!ext) {
        fprintf(stderr, "openmpt_module_ext_create_from_memory failed: %s\n", err_msg ? err_msg : "(no msg)");
        return 1;
    }

    openmpt_module *mod = openmpt_module_ext_get_module(ext);
    int n_channels = openmpt_module_get_num_channels(mod);

    /* Pull the interactive interface so we can flip mute bits per channel. */
    openmpt_module_ext_interface_interactive iface;
    if (!openmpt_module_ext_get_interface(ext, LIBOPENMPT_EXT_C_INTERFACE_INTERACTIVE, &iface, sizeof(iface))) {
        fprintf(stderr, "interactive interface unavailable\n");
        openmpt_module_ext_destroy(ext);
        return 1;
    }

    /* Silence channels via set_channel_volume(0) instead of
     * set_channel_mute_status(true). Critical reason: OpenMPT's S3M
     * playback ignores effects on MUTED channels entirely
     * (kST3NoMutedChannels flag, Snd_fx.cpp:571-576 — "not even effects
     * are processed on muted S3M channels"). With set_channel_mute_status,
     * if any muted channel had a SetSpeed/SetBpm/PatternBreak effect,
     * the song timing in solo would drift from the full mix's timing —
     * solo and full would no longer represent the same song timeline,
     * breaking the diagnostic.
     *
     * set_channel_volume(0) is a post-mix multiplier: the channel still
     * processes ALL effects (including global ones), only its audio
     * contribution to the output is zeroed. This gives a clean
     * "what does this channel contribute, with everything else's
     * effects intact" measurement that's directly comparable between
     * our engine's force_off behavior and OpenMPT's. */
    if (solo_channel >= 0) {
        for (int c = 0; c < n_channels; c++) {
            if (c != solo_channel) {
                iface.set_channel_volume(ext, c, 0.0);
            }
        }
    }
    for (int i = 0; i < mute_count; i++) {
        int c = mute_list[i];
        if (c >= 0 && c < n_channels) {
            iface.set_channel_volume(ext, c, 0.0);
        }
    }

    fprintf(stderr, "channels=%d solo=%d mutes=%d\n", n_channels, solo_channel, mute_count);

    /* Open the output and reserve the header (patched at the end). */
    FILE *fout = fopen(out_path, "wb");
    if (!fout) { perror("open output"); openmpt_module_ext_destroy(ext); return 1; }
    if (write_wav_header(fout, 0) < 0) { perror("write hdr"); fclose(fout); openmpt_module_ext_destroy(ext); return 1; }

    /* Optional VU trace: open a TSV with one column per channel. The
     * VU meter reports the post-mix sample-energy estimate per channel
     * — not the same as the engine-side `nVolume` slot, but it gives
     * us a directly-comparable curve that we can diff against our
     * `voice.volume.volume` over time. The two will agree on
     * macro-shape (silent vs loud) which is what we care about for
     * the t=120s investigation. Dumping with a smaller render-block
     * size sharpens the time resolution. */
    FILE *vu = NULL;
    if (vu_trace_path) {
        vu = fopen(vu_trace_path, "wb");
        if (!vu) { perror("open vu-trace"); fclose(fout); openmpt_module_ext_destroy(ext); return 1; }
        fprintf(vu, "frame\tt_sec");
        for (int c = 0; c < n_channels; c++) fprintf(vu, "\tch%d", c);
        fprintf(vu, "\n");
    }

    float left[BUFLEN], right[BUFLEN];
    uint64_t total_frames = 0;
    uint64_t max_frames = (end_time > 0) ? (uint64_t)(end_time * RATE) : UINT64_MAX;

    /* For VU traces we render in tighter blocks so each row of TSV is
     * ~5 ms (~240 frames at 48 kHz). The same block size for non-VU
     * mode is fine — the WAV output is identical either way. */
    size_t block = vu_trace_path ? 240 : BUFLEN;

    while (total_frames < max_frames) {
        size_t want = block;
        if (max_frames - total_frames < want) want = max_frames - total_frames;
        size_t got = openmpt_module_read_float_stereo(mod, RATE, want, left, right);
        if (got == 0) break;
        for (size_t i = 0; i < got; i++) {
            float pair[2] = { left[i], right[i] };
            if (fwrite(pair, sizeof(float), 2, fout) != 2) { perror("write"); fclose(fout); openmpt_module_ext_destroy(ext); return 1; }
        }
        total_frames += got;
        if (vu) {
            fprintf(vu, "%llu\t%.6f", (unsigned long long)total_frames, (double)total_frames / RATE);
            for (int c = 0; c < n_channels; c++) {
                float v = openmpt_module_get_current_channel_vu_mono(mod, c);
                fprintf(vu, "\t%.5f", v);
            }
            fprintf(vu, "\n");
        }
    }
    if (vu) fclose(vu);

    /* Patch the WAV size fields. */
    uint32_t data_bytes = (uint32_t)(total_frames * 2 * 4);
    uint32_t riff_size = 36 + data_bytes;
    fseek(fout, 4, SEEK_SET);
    fwrite(&riff_size, 4, 1, fout);
    fseek(fout, 40, SEEK_SET);
    fwrite(&data_bytes, 4, 1, fout);
    fclose(fout);

    fprintf(stderr, "wrote %.2fs (%llu frames) to %s\n",
        (double)total_frames / RATE, (unsigned long long)total_frames, out_path);

    openmpt_module_ext_destroy(ext);
    return 0;
}
