/*
 * Mirror Stream (USB) — OBS source plugin.
 *
 * Reads decoded BGRA frames from the shared memory segment written by the
 * ScreenMirror desktop backend (Rust, shared_mem.rs) and mono f32/48 kHz
 * audio from a companion ring buffer (obs_feed.rs).
 *
 * Cross-platform: POSIX shared memory on Linux/macOS, named file mappings
 * on Windows. The Windows mapping names match what the Rust side creates:
 *   video: "obs_mirror_buffer"   (shared_memory crate, os_id verbatim)
 *   audio: "mirror_obs_audio"    (CreateFileMappingA in obs_feed.rs)
 */

#include <obs-module.h>
#include <util/bmem.h>
#include <util/platform.h>
#include <util/threading.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#ifdef _WIN32
#  include <windows.h>
#else
#  include <sys/mman.h>
#  include <sys/stat.h>
#  include <fcntl.h>
#  include <unistd.h>
#endif

OBS_DECLARE_MODULE()
OBS_MODULE_AUTHOR("Mirror Team")
OBS_MODULE_USE_DEFAULT_LOCALE("mirror-source", "en-US")

/* ── Shared memory contract — MUST match shared_mem.rs / obs_feed.rs ────── */
#define SHM_NAME             "obs_mirror_buffer"
#ifdef _WIN32
#  define AUDIO_SHM_NAME     "mirror_obs_audio"
#else
#  define AUDIO_SHM_NAME     "/mirror_obs_audio"
#endif
#define CONTROL_SIZE         64
#define SLOT_HEADER_SIZE     64
#define MAX_WIDTH            3840
#define MAX_HEIGHT           2160
#define MAX_FRAME_SIZE       (MAX_WIDTH * MAX_HEIGHT * 4)
#define SLOT_SIZE            (SLOT_HEADER_SIZE + MAX_FRAME_SIZE)
#define TOTAL_SHM_SIZE       (CONTROL_SIZE + 3 * SLOT_SIZE)
#define AUDIO_BUFFER_SAMPLES 96000
#define AUDIO_SAMPLE_RATE    48000

struct shm_control {
    char     magic[4];       /* "MPRO" */
    volatile int32_t latest_index; /* -1, 0, 1, or 2 */
    uint64_t session;        /* changes whenever the backend restarts */
    uint8_t  _pad[48];
};

/*
 * Per-slot header. The `seq` field is a seqlock: the writer increments it to
 * an odd value before writing and to the next even value when done. A reader
 * that observes an odd value, or a different value after copying, discards
 * the copy — this is what prevents tearing when the writer laps the reader.
 */
struct mpro_frame_header {
    char     magic[4];        /* "MIRR" */
    volatile uint32_t seq;    /* seqlock: odd = write in progress */
    uint32_t width;
    uint32_t height;
    uint64_t timestamp;
    uint32_t data_size;
    uint8_t  _pad[36];        /* header occupies the full 64-byte slot header */
};

/*
 * `written` is a running total of samples ever produced, not a ring index.
 * That is what lets this reader detect being lapped: if the writer has
 * advanced by more than the ring holds since our last read, the samples we
 * were about to consume have already been overwritten.
 */
struct audio_shm_header {
    char     magic[4];          /* "MIRA" */
    uint32_t _pad0;
    volatile uint64_t written;  /* total samples written */
    volatile uint64_t session;  /* changes whenever the backend restarts */
    uint64_t _pad1;
};

/* Compile-time layout verification — catches mismatches between Rust and C */
#if defined(_MSC_VER)
static_assert(sizeof(struct shm_control) == 64, "shm_control must be 64 bytes");
static_assert(sizeof(struct mpro_frame_header) == 64, "mpro_frame_header must be 64 bytes");
#else
_Static_assert(sizeof(struct shm_control) == 64, "shm_control must be 64 bytes");
_Static_assert(sizeof(struct mpro_frame_header) == 64, "mpro_frame_header must be 64 bytes");
_Static_assert(sizeof(struct audio_shm_header) == 32, "audio_shm_header must be 32 bytes");
#endif

#define AUDIO_HEADER_SIZE 32
#define AUDIO_SHM_SIZE (AUDIO_HEADER_SIZE + (AUDIO_BUFFER_SAMPLES * sizeof(float)))

/* Re-open a mapping that has produced nothing for this long: the backend may
 * have restarted, which on POSIX unlinks the old segment and creates a new
 * one. Our existing mapping stays alive but frozen, so without this the
 * source shows the last frame forever until it is removed and re-added. */
#define STALE_REMAP_NS 3000000000ULL /* 3 s */

/* ── Portable atomics (x86/ARM acquire loads + full fence) ─────────────── */
#if defined(_MSC_VER)
#  define LOAD_ACQ_U32(p)  ((uint32_t)InterlockedCompareExchange((volatile LONG *)(p), 0, 0))
#  define LOAD_ACQ_I32(p)  ((int32_t)InterlockedCompareExchange((volatile LONG *)(p), 0, 0))
#  define LOAD_ACQ_U64(p)  ((uint64_t)InterlockedCompareExchange64((volatile LONG64 *)(p), 0, 0))
#  define FULL_FENCE()     MemoryBarrier()
#else
#  define LOAD_ACQ_U32(p)  __atomic_load_n((p), __ATOMIC_ACQUIRE)
#  define LOAD_ACQ_I32(p)  __atomic_load_n((p), __ATOMIC_ACQUIRE)
#  define LOAD_ACQ_U64(p)  __atomic_load_n((p), __ATOMIC_ACQUIRE)
#  define FULL_FENCE()     __atomic_thread_fence(__ATOMIC_SEQ_CST)
#endif

/* ── Platform shared-memory helpers ─────────────────────────────────────── */

struct shm_map {
    uint8_t *ptr;
    size_t   size;
#ifdef _WIN32
    HANDLE   handle;
#else
    int      fd;
#endif
    bool     open;
};

static bool shm_map_open(struct shm_map *m, const char *name, size_t expected_size)
{
#ifdef _WIN32
    HANDLE h = OpenFileMappingA(FILE_MAP_READ, FALSE, name);
    if (!h)
        return false;
    void *ptr = MapViewOfFile(h, FILE_MAP_READ, 0, 0, expected_size);
    if (!ptr) {
        CloseHandle(h);
        return false;
    }
    m->ptr = (uint8_t *)ptr;
    m->size = expected_size;
    m->handle = h;
    m->open = true;
    return true;
#else
    int fd = shm_open(name, O_RDONLY, 0);
    if (fd < 0)
        return false;
    struct stat st;
    size_t size = expected_size;
    if (fstat(fd, &st) == 0 && (size_t)st.st_size >= expected_size)
        size = (size_t)st.st_size;
    else if (fstat(fd, &st) == 0 && (size_t)st.st_size < expected_size) {
        close(fd);
        return false;
    }
    void *ptr = mmap(NULL, size, PROT_READ, MAP_SHARED, fd, 0);
    if (ptr == MAP_FAILED) {
        close(fd);
        return false;
    }
    m->ptr = (uint8_t *)ptr;
    m->size = size;
    m->fd = fd;
    m->open = true;
    return true;
#endif
}

static void shm_map_close(struct shm_map *m)
{
    if (!m->open)
        return;
#ifdef _WIN32
    UnmapViewOfFile(m->ptr);
    CloseHandle(m->handle);
#else
    munmap(m->ptr, m->size);
    close(m->fd);
#endif
    m->ptr = NULL;
    m->size = 0;
    m->open = false;
}

/* ── Source state ───────────────────────────────────────────── */

struct mirror_source {
    obs_source_t *source;
    char         *shm_name;
    bool          advanced;

    struct shm_map video_shm;
    struct shm_map audio_shm;

    /* Local pixel buffer — allocated once in mirror_create at max 4K size. */
    uint8_t      *pixel_buf;
    size_t        pixel_buf_size;

    /*
     * The audio ring is opened and closed from the OBS tick/update thread but
     * read from audio_thread, so every access to `audio_shm` (and the tail
     * cursor derived from it) is serialised by this lock. Without it the audio
     * thread could observe `open == true` before `ptr` was published and
     * dereference a garbage pointer.
     */
    pthread_mutex_t audio_lock;
    uint64_t      audio_consumed;  /* samples handed to OBS so far */
    uint64_t      audio_session;
    bool          audio_synced;

    uint64_t      video_session;
    uint64_t      last_progress_ns; /* when we last saw a new frame */

    gs_texture_t *texture;
    uint32_t      tex_width;
    uint32_t      tex_height;
    uint64_t      last_timestamp;
    uint32_t      last_w;
    uint32_t      last_h;
    enum gs_color_format current_fmt;

    bool          use_unorm;
    bool          srgb_render;

    pthread_t     audio_thread;
    int32_t       last_slot_idx;
    uint32_t      last_slot_seq;
    volatile bool thread_active;
};

/* ── Forward declarations ─────────────────────────────────── */
static const char *mirror_get_name(void *unused);
static void       *mirror_create(obs_data_t *settings, obs_source_t *source);
static void        mirror_destroy(void *data);
static void        mirror_update(void *data, obs_data_t *settings);
static uint32_t    mirror_get_width(void *data);
static uint32_t    mirror_get_height(void *data);
static obs_properties_t *mirror_get_properties(void *data);
static void        mirror_video_tick(void *data, float seconds);
static void        mirror_video_render(void *data, gs_effect_t *effect);

/* ── Source info registration ─────────────────────────────── */
static struct obs_source_info mirror_source_info = {
    .id             = "mirror_stream_source",
    .type           = OBS_SOURCE_TYPE_INPUT,
    .output_flags   = OBS_SOURCE_VIDEO | OBS_SOURCE_CUSTOM_DRAW | OBS_SOURCE_AUDIO,
    .get_name       = mirror_get_name,
    .create         = mirror_create,
    .destroy        = mirror_destroy,
    .update         = mirror_update,
    .get_width      = mirror_get_width,
    .get_height     = mirror_get_height,
    .get_properties = mirror_get_properties,
    .video_tick     = mirror_video_tick,
    .video_render   = mirror_video_render,
};

bool obs_module_load(void)
{
    obs_register_source(&mirror_source_info);
    blog(LOG_INFO, "[Mirror Source] Plugin loaded");
    return true;
}

void obs_module_unload(void)
{
    blog(LOG_INFO, "[Mirror Source] Plugin unloaded");
}

/* ── Helpers ──────────────────────────────────────────────── */

static void close_shmem(struct mirror_source *ctx)
{
    shm_map_close(&ctx->video_shm);

    pthread_mutex_lock(&ctx->audio_lock);
    shm_map_close(&ctx->audio_shm);
    ctx->audio_synced = false;
    pthread_mutex_unlock(&ctx->audio_lock);
}

static bool try_open_shmem(struct mirror_source *ctx)
{
    if (!ctx->video_shm.open && ctx->shm_name && *ctx->shm_name)
        shm_map_open(&ctx->video_shm, ctx->shm_name, TOTAL_SHM_SIZE);

    pthread_mutex_lock(&ctx->audio_lock);
    if (!ctx->audio_shm.open) {
        if (shm_map_open(&ctx->audio_shm, AUDIO_SHM_NAME, AUDIO_SHM_SIZE))
            ctx->audio_synced = false; /* pick up the cursor on first read */
    }
    pthread_mutex_unlock(&ctx->audio_lock);

    return ctx->video_shm.open;
}

static void *audio_thread(void *arg)
{
    struct mirror_source *ctx = arg;

    while (ctx->thread_active) {
        uint32_t sleep_ms = 0;

        pthread_mutex_lock(&ctx->audio_lock);

        if (!ctx->audio_shm.open) {
            sleep_ms = 50;
            goto next;
        }

        struct audio_shm_header *hdr = (struct audio_shm_header *)ctx->audio_shm.ptr;
        if (memcmp(hdr->magic, "MIRA", 4) != 0) {
            sleep_ms = 50;
            goto next;
        }

        uint64_t written = LOAD_ACQ_U64(&hdr->written);
        uint64_t session = LOAD_ACQ_U64(&hdr->session);

        /*
         * On first read, and whenever the backend restarts, start from
         * wherever the writer is now. Starting from zero would replay up to
         * two seconds of whatever the ring already held as a burst of stale
         * audio the moment the source is added.
         */
        if (!ctx->audio_synced || session != ctx->audio_session) {
            ctx->audio_session  = session;
            ctx->audio_consumed = written;
            ctx->audio_synced   = true;
            sleep_ms = 5;
            goto next;
        }

        uint64_t avail = written - ctx->audio_consumed;
        if (avail == 0) {
            sleep_ms = 5;
            goto next;
        }

        if (avail > AUDIO_BUFFER_SAMPLES) {
            /* Lapped: the oldest pending samples are already overwritten.
             * Skip to the newest full buffer rather than emitting garbage. */
            blog(LOG_WARNING,
                 "[Mirror Source] Audio ring overrun, dropped %llu samples",
                 (unsigned long long)(avail - AUDIO_BUFFER_SAMPLES));
            ctx->audio_consumed = written - AUDIO_BUFFER_SAMPLES;
            avail = AUDIO_BUFFER_SAMPLES;
        }

        const float *data = (const float *)(ctx->audio_shm.ptr + AUDIO_HEADER_SIZE);
        uint32_t start = (uint32_t)(ctx->audio_consumed % AUDIO_BUFFER_SAMPLES);
        uint32_t total = (uint32_t)avail;
        uint32_t first = total;
        if (first > AUDIO_BUFFER_SAMPLES - start)
            first = AUDIO_BUFFER_SAMPLES - start;

        /* Timestamp the *start* of the batch. Using os_gettime_ns() directly
         * dated every buffer to the moment it was drained, which pushed the
         * audio clock later than the samples it described. */
        uint64_t now = os_gettime_ns();
        uint64_t base = now - ((uint64_t)total * 1000000000ULL / AUDIO_SAMPLE_RATE);

        struct obs_source_audio audio = {0};
        audio.speakers        = SPEAKERS_MONO;
        audio.samples_per_sec = AUDIO_SAMPLE_RATE;
        audio.format          = AUDIO_FORMAT_FLOAT;

        audio.frames    = first;
        audio.data[0]   = (uint8_t *)(data + start);
        audio.timestamp = base;
        obs_source_output_audio(ctx->source, &audio);

        if (total > first) {
            uint32_t rest = total - first;
            audio.frames    = rest;
            audio.data[0]   = (uint8_t *)data; /* wrapped to the ring start */
            audio.timestamp = base + ((uint64_t)first * 1000000000ULL / AUDIO_SAMPLE_RATE);
            obs_source_output_audio(ctx->source, &audio);
        }

        ctx->audio_consumed += total;

next:
        pthread_mutex_unlock(&ctx->audio_lock);
        if (sleep_ms)
            os_sleep_ms(sleep_ms);
    }
    return NULL;
}

/* ── Source callbacks ─────────────────────────────────────── */

static const char *mirror_get_name(void *unused)
{
    UNUSED_PARAMETER(unused);
    return "Mirror Stream (USB)";
}

static void *mirror_create(obs_data_t *settings, obs_source_t *source)
{
    struct mirror_source *ctx = bzalloc(sizeof(*ctx));
    ctx->source        = source;
    ctx->last_slot_idx = -1;
    ctx->last_slot_seq = 0;

    ctx->pixel_buf_size = MAX_FRAME_SIZE;
    ctx->pixel_buf      = bmalloc(ctx->pixel_buf_size);
    if (!ctx->pixel_buf) {
        blog(LOG_ERROR, "[Mirror Source] Failed to allocate pixel buffer (%zu bytes)",
             ctx->pixel_buf_size);
        bfree(ctx);
        return NULL;
    }

    /* Must exist before mirror_update(), which can close the audio map. */
    pthread_mutex_init(&ctx->audio_lock, NULL);

    mirror_update(ctx, settings);

    ctx->thread_active = true;
    pthread_create(&ctx->audio_thread, NULL, audio_thread, ctx);

    blog(LOG_INFO, "[Mirror Source] Source created (pixel_buf=%zu bytes)",
         ctx->pixel_buf_size);
    return ctx;
}

static void mirror_destroy(void *data)
{
    struct mirror_source *ctx = data;

    ctx->thread_active = false;
    pthread_join(ctx->audio_thread, NULL);

    obs_enter_graphics();
    if (ctx->texture) {
        gs_texture_destroy(ctx->texture);
        ctx->texture = NULL;
    }
    obs_leave_graphics();

    /* The audio thread is already joined, so nothing else holds the lock. */
    close_shmem(ctx);
    pthread_mutex_destroy(&ctx->audio_lock);

    if (ctx->pixel_buf) {
        bfree(ctx->pixel_buf);
        ctx->pixel_buf = NULL;
    }

    if (ctx->shm_name)
        bfree(ctx->shm_name);
    bfree(ctx);

    blog(LOG_INFO, "[Mirror Source] Source destroyed");
}

static void mirror_update(void *data, obs_data_t *settings)
{
    struct mirror_source *ctx = data;

    ctx->advanced = obs_data_get_bool(settings, "advanced");

    const char *new_shm = obs_data_get_string(settings, "shm_name");
    if (!new_shm || !*new_shm || !ctx->advanced) {
        new_shm = SHM_NAME;
    }

    if (!ctx->shm_name || strcmp(ctx->shm_name, new_shm) != 0) {
        close_shmem(ctx);
        if (ctx->shm_name)
            bfree(ctx->shm_name);
        ctx->shm_name = bstrdup(new_shm);
    }

    const char *fmt = obs_data_get_string(settings, "color_fmt");
    ctx->use_unorm = (fmt && strcmp(fmt, "BGRA_UNORM") == 0);

    obs_data_set_default_bool(settings, "srgb_render", false);
    ctx->srgb_render = obs_data_get_bool(settings, "srgb_render");
}

static bool advanced_modified(obs_properties_t *props, obs_property_t *p, obs_data_t *settings)
{
    UNUSED_PARAMETER(p);
    bool advanced = obs_data_get_bool(settings, "advanced");
    obs_property_t *shm_prop = obs_properties_get(props, "shm_name");
    obs_property_set_visible(shm_prop, advanced);
    return true;
}

static obs_properties_t *mirror_get_properties(void *data)
{
    UNUSED_PARAMETER(data);
    obs_properties_t *ppts = obs_properties_create();

    obs_property_t *color_fmt = obs_properties_add_list(
        ppts, "color_fmt", "Color Format", OBS_COMBO_TYPE_LIST, OBS_COMBO_FORMAT_STRING);
    obs_property_list_add_string(color_fmt, "BGRA (Default - sRGB)", "BGRA");
    obs_property_list_add_string(color_fmt, "BGRA_UNORM (Linear - Fixes Grey Screen)",
                                 "BGRA_UNORM");

    obs_properties_add_bool(ppts, "srgb_render", "Enable OBS sRGB Conversion");

    obs_property_t *adv = obs_properties_add_bool(ppts, "advanced", "Advanced Settings");
    obs_property_set_modified_callback(adv, advanced_modified);

    obs_property_t *shm = obs_properties_add_text(ppts, "shm_name", "Shared Memory Path",
                                                  OBS_TEXT_DEFAULT);
    obs_property_set_visible(shm, false);

    return ppts;
}

static uint32_t mirror_get_width(void *data)
{
    struct mirror_source *ctx = data;
    return ctx->last_w > 0 ? ctx->last_w : 1920;
}

static uint32_t mirror_get_height(void *data)
{
    struct mirror_source *ctx = data;
    return ctx->last_h > 0 ? ctx->last_h : 1080;
}

static void mirror_video_tick(void *data, float seconds)
{
    UNUSED_PARAMETER(seconds);
    struct mirror_source *ctx = data;

    uint64_t now_ns = os_gettime_ns();

    if (!ctx->video_shm.open) {
        try_open_shmem(ctx);
        ctx->last_progress_ns = now_ns;
        return;
    }

    /*
     * Nothing new for a while? The backend may have restarted, which unlinks
     * the old segment and creates a new one; our mapping would stay valid but
     * frozen forever. Re-opening by name picks up the current segment. Two
     * syscalls every few seconds while idle is a cheap price for not needing
     * the user to remove and re-add the source.
     */
    if (now_ns - ctx->last_progress_ns > STALE_REMAP_NS) {
        ctx->last_progress_ns = now_ns;
        shm_map_close(&ctx->video_shm);
        if (!try_open_shmem(ctx))
            return;
    }

    const struct shm_control *ctrl = (const struct shm_control *)ctx->video_shm.ptr;
    int32_t latest = LOAD_ACQ_I32(&ctrl->latest_index);

    /* A different session means the slot index and seqlock values we cached
     * describe a segment that no longer exists. */
    if (ctrl->session != ctx->video_session) {
        ctx->video_session  = ctrl->session;
        ctx->last_slot_idx  = -1;
        ctx->last_slot_seq  = 0;
    }

    if (latest < 0 || latest > 2)
        return;

    size_t slot_offset = CONTROL_SIZE + ((size_t)latest * SLOT_SIZE);
    if (ctx->video_shm.size < slot_offset + SLOT_HEADER_SIZE)
        return;

    struct mpro_frame_header *fhdr =
        (struct mpro_frame_header *)(ctx->video_shm.ptr + slot_offset);

    if (memcmp(fhdr->magic, "MIRR", 4) != 0)
        return;

    /* ── Seqlock read protocol ─────────────────────────────────
     * 1. Read seq — odd means a write is in progress: skip this tick.
     * 2. Copy header fields + pixels.
     * 3. Fence, re-read seq — any change means the writer lapped us
     *    mid-copy: discard the (possibly torn) copy.
     */
    uint32_t seq_before = LOAD_ACQ_U32(&fhdr->seq);
    if (seq_before & 1)
        return;

    /* Same slot and same generation as last frame → nothing new. */
    if (latest == ctx->last_slot_idx && seq_before == ctx->last_slot_seq)
        return;

    uint32_t w = fhdr->width;
    uint32_t h = fhdr->height;
    uint64_t ts = fhdr->timestamp;
    uint32_t data_size = fhdr->data_size;

    if (w == 0 || h == 0 || w > MAX_WIDTH || h > MAX_HEIGHT)
        return;
    if (data_size > ctx->pixel_buf_size)
        return;
    /* gs_texture_set_image below reads w*h*4 bytes regardless of data_size, so
     * a short frame would upload uninitialised buffer contents. */
    if (data_size != (size_t)w * h * 4)
        return;
    if (ctx->video_shm.size < slot_offset + SLOT_HEADER_SIZE + data_size)
        return;

    const uint8_t *shm_pixels = ctx->video_shm.ptr + slot_offset + SLOT_HEADER_SIZE;
    memcpy(ctx->pixel_buf, shm_pixels, data_size);

    FULL_FENCE();
    uint32_t seq_after = LOAD_ACQ_U32(&fhdr->seq);
    if (seq_after != seq_before)
        return; /* writer lapped us — try again next tick */

    obs_enter_graphics();
    enum gs_color_format format = ctx->use_unorm ? GS_BGRA_UNORM : GS_BGRA;

    if (!ctx->texture || ctx->tex_width != w || ctx->tex_height != h ||
        ctx->current_fmt != format) {
        if (ctx->texture)
            gs_texture_destroy(ctx->texture);
        ctx->texture     = gs_texture_create(w, h, format, 1, NULL, GS_DYNAMIC);
        ctx->tex_width   = w;
        ctx->tex_height  = h;
        ctx->current_fmt = format;
        blog(LOG_INFO, "[Mirror Source] Resolution: %ux%u fmt=%u", w, h, (uint32_t)format);
    }

    gs_texture_set_image(ctx->texture, ctx->pixel_buf, w * 4, false);
    obs_leave_graphics();

    ctx->last_timestamp   = ts;
    ctx->last_slot_idx    = latest;
    ctx->last_slot_seq    = seq_before;
    ctx->last_w           = w;
    ctx->last_h           = h;
    ctx->last_progress_ns = now_ns; /* real frame — mapping is alive */
}

static void mirror_video_render(void *data, gs_effect_t *effect)
{
    UNUSED_PARAMETER(effect);
    struct mirror_source *ctx = data;

    if (!ctx->texture)
        return;

    const bool linear_srgb = gs_get_linear_srgb();
    const bool previous = gs_framebuffer_srgb_enabled();

    if (ctx->srgb_render || linear_srgb) {
        gs_enable_framebuffer_srgb(true);
    }

    gs_effect_t *eff = obs_get_base_effect(linear_srgb ? OBS_EFFECT_DEFAULT_RECT
                                                       : OBS_EFFECT_DEFAULT);

    gs_blend_state_push();
    gs_blend_function(GS_BLEND_ONE, GS_BLEND_ZERO);

    while (gs_effect_loop(eff, "Draw")) {
        gs_eparam_t *param = gs_effect_get_param_by_name(eff, "image");
        gs_effect_set_texture_srgb(param, ctx->texture);
        gs_draw_sprite(ctx->texture, 0, ctx->tex_width, ctx->tex_height);
    }

    gs_blend_state_pop();

    if (ctx->srgb_render || linear_srgb) {
        gs_enable_framebuffer_srgb(previous);
    }
}
