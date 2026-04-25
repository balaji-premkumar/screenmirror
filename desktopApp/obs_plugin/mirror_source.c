#include <obs-module.h>
#include <util/bmem.h>
#include <util/platform.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <fcntl.h>
#include <unistd.h>
#include <string.h>
#include <pthread.h>
#include <stddef.h>   /* offsetof */
#include <stdint.h>

OBS_DECLARE_MODULE()
OBS_MODULE_AUTHOR("Mirror Team")
OBS_MODULE_USE_DEFAULT_LOCALE("mirror-source", "en-US")

/* ── Shared memory header — MUST match obs_feed.rs exactly ──────────────── */
#define SHM_NAME         "obs_mirror_buffer"
#define AUDIO_SHM_NAME   "/mirror_obs_audio"
#define CONTROL_SIZE     64
#define SLOT_HEADER_SIZE 64
#define MAX_WIDTH        3840
#define MAX_HEIGHT       2160
#define MAX_FRAME_SIZE   (MAX_WIDTH * MAX_HEIGHT * 4)
#define SLOT_SIZE        (SLOT_HEADER_SIZE + MAX_FRAME_SIZE)
#define AUDIO_BUFFER_SAMPLES 96000

struct shm_control {
    char     magic[4];       /* "MPRO" */
    int32_t  latest_index;   /* -1, 0, 1, or 2 */
    uint8_t  _pad[56];
};

struct mpro_frame_header {
    char     magic[4];       /* "MIRR" */
    uint32_t width;
    uint32_t height;
    uint64_t timestamp;
    uint32_t data_size;
    uint8_t  _pad[8];        /* Pad to 32 bytes */
};

/* Compile-time layout verification — catches mismatches between Rust and C */
_Static_assert(sizeof(struct shm_control) == 64, "shm_control must be 64 bytes");
_Static_assert(sizeof(struct mpro_frame_header) == 32, "mpro_frame_header must be 32 bytes");

struct audio_shm_header {
    char     magic[4];
    uint32_t head;
};

#define AUDIO_HEADER_SIZE sizeof(struct audio_shm_header)
#define AUDIO_SHM_SIZE (AUDIO_HEADER_SIZE + (AUDIO_BUFFER_SAMPLES * sizeof(float)))

struct mirror_source {
    obs_source_t *source;
    char         *shm_name;
    bool          advanced;

    /* Video shared memory */
    uint8_t      *shmem_ptr;
    size_t        shmem_size;
    int           shmem_fd;
    bool          shmem_open;

    /* Local pixel buffer — allocated once in mirror_create at max 4K size. */
    uint8_t      *pixel_buf;
    size_t        pixel_buf_size;

    /* Audio shared memory */
    uint8_t      *audio_shm_ptr;
    int           audio_shm_fd;
    bool          audio_shm_open;
    uint32_t      audio_tail;

    gs_texture_t *texture;
    uint32_t      tex_width;
    uint32_t      tex_height;
    uint64_t      last_timestamp;
    uint64_t      last_frame_count;
    uint32_t      last_w;
    uint32_t      last_h;
    enum gs_color_format current_fmt;

    bool          use_unorm;
    bool          srgb_render;

    pthread_t     audio_thread;
    int32_t       last_slot_idx;
    bool          thread_active;
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
    if (ctx->shmem_open) {
        munmap(ctx->shmem_ptr, ctx->shmem_size);
        close(ctx->shmem_fd);
        ctx->shmem_ptr  = NULL;
        ctx->shmem_size = 0;
        ctx->shmem_fd   = -1;
        ctx->shmem_open = false;
    }
    if (ctx->audio_shm_open) {
        munmap(ctx->audio_shm_ptr, AUDIO_SHM_SIZE);
        close(ctx->audio_shm_fd);
        ctx->audio_shm_ptr = NULL;
        ctx->audio_shm_fd = -1;
        ctx->audio_shm_open = false;
    }
}

static bool try_open_shmem(struct mirror_source *ctx)
{
    if (!ctx->shmem_open && ctx->shm_name && *ctx->shm_name) {
        int fd = shm_open(ctx->shm_name, O_RDONLY, 0);
        if (fd >= 0) {
            struct stat st;
            if (fstat(fd, &st) == 0 && st.st_size >= (off_t)CONTROL_SIZE) {
                void *ptr = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_SHARED, fd, 0);
                if (ptr != MAP_FAILED) {
                    ctx->shmem_ptr  = (uint8_t *)ptr;
                    ctx->shmem_size = (size_t)st.st_size;
                    ctx->shmem_fd   = fd;
                    ctx->shmem_open = true;
                } else close(fd);
            } else close(fd);
        }
    }

    if (!ctx->audio_shm_open) {
        int fd = shm_open(AUDIO_SHM_NAME, O_RDONLY, 0);
        if (fd >= 0) {
            void *ptr = mmap(NULL, AUDIO_SHM_SIZE, PROT_READ, MAP_SHARED, fd, 0);
            if (ptr != MAP_FAILED) {
                ctx->audio_shm_ptr = (uint8_t *)ptr;
                ctx->audio_shm_fd = fd;
                ctx->audio_shm_open = true;
            } else close(fd);
        }
    }

    return ctx->shmem_open;
}

static void *audio_thread(void *arg)
{
    struct mirror_source *ctx = arg;

    while (ctx->thread_active) {
        if (!ctx->audio_shm_open) {
            usleep(50000);
            continue;
        }

        struct audio_shm_header *hdr = (struct audio_shm_header *)ctx->audio_shm_ptr;
        if (memcmp(hdr->magic, "MIRA", 4) != 0) {
            usleep(50000);
            continue;
        }

        uint32_t head = hdr->head;
        if (head >= AUDIO_BUFFER_SAMPLES) head = 0;

        if (ctx->audio_tail == head) {
            usleep(5000); // 5ms sleep
            continue;
        }

        float *data = (float *)(ctx->audio_shm_ptr + AUDIO_HEADER_SIZE);
        uint32_t count = 0;
        
        // Handle wrap-around
        if (head < ctx->audio_tail) {
            count = AUDIO_BUFFER_SAMPLES - ctx->audio_tail;
            struct obs_source_audio audio = {0};
            audio.speakers = SPEAKERS_MONO;
            audio.samples_per_sec = 48000;
            audio.format = AUDIO_FORMAT_FLOAT;
            audio.frames = count;
            audio.data[0] = (uint8_t *)(data + ctx->audio_tail);
            audio.timestamp = os_gettime_ns();
            obs_source_output_audio(ctx->source, &audio);
            ctx->audio_tail = 0;
        }

        count = head - ctx->audio_tail;
        if (count > 0) {
            struct obs_source_audio audio = {0};
            audio.speakers = SPEAKERS_MONO;
            audio.samples_per_sec = 48000;
            audio.format = AUDIO_FORMAT_FLOAT;
            audio.frames = count;
            audio.data[0] = (uint8_t *)(data + ctx->audio_tail);
            audio.timestamp = os_gettime_ns();
            obs_source_output_audio(ctx->source, &audio);
            ctx->audio_tail = head;
        }
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
    ctx->source          = source;
    ctx->shmem_fd        = -1;
    ctx->shmem_open      = false;
    ctx->audio_shm_fd    = -1;
    ctx->audio_shm_open  = false;
    ctx->audio_tail      = 0;
    ctx->texture         = NULL;
    ctx->last_slot_idx   = -1;

    ctx->pixel_buf_size = MAX_FRAME_SIZE;
    ctx->pixel_buf      = bmalloc(ctx->pixel_buf_size);
    if (!ctx->pixel_buf) {
        blog(LOG_ERROR, "[Mirror Source] Failed to allocate pixel buffer (%zu bytes)", ctx->pixel_buf_size);
        bfree(ctx);
        return NULL;
    }

    mirror_update(ctx, settings);

    ctx->thread_active = true;
    pthread_create(&ctx->audio_thread, NULL, audio_thread, ctx);

    blog(LOG_INFO, "[Mirror Source] Source created (pixel_buf=%zu bytes)", ctx->pixel_buf_size);
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

    close_shmem(ctx);

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

    obs_property_t *color_fmt = obs_properties_add_list(ppts, "color_fmt", "Color Format", OBS_COMBO_TYPE_LIST, OBS_COMBO_FORMAT_STRING);
    obs_property_list_add_string(color_fmt, "BGRA (Default - sRGB)", "BGRA");
    obs_property_list_add_string(color_fmt, "BGRA_UNORM (Linear - Fixes Grey Screen)", "BGRA_UNORM");

    obs_properties_add_bool(ppts, "srgb_render", "Enable OBS sRGB Conversion");

    obs_property_t *adv = obs_properties_add_bool(ppts, "advanced", "Advanced Settings");
    obs_property_set_modified_callback(adv, advanced_modified);

    obs_property_t *shm = obs_properties_add_text(ppts, "shm_name", "Shared Memory Path", OBS_TEXT_DEFAULT);
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

    if (!ctx->shmem_open) {
        try_open_shmem(ctx);
        return;
    }

    const struct shm_control *ctrl = (const struct shm_control *)ctx->shmem_ptr;
    int32_t latest = __atomic_load_n(&ctrl->latest_index, __ATOMIC_ACQUIRE);

    if (latest < 0 || latest > 2)
        return;

    if (latest == ctx->last_slot_idx)
        return;

    size_t slot_offset = CONTROL_SIZE + (latest * SLOT_SIZE);
    if (ctx->shmem_size < slot_offset + SLOT_HEADER_SIZE)
        return;

    const struct mpro_frame_header *fhdr = (const struct mpro_frame_header *)(ctx->shmem_ptr + slot_offset);
    
    char magic[4];
    memcpy(magic, fhdr->magic, 4);
    if (memcmp(magic, "MIRR", 4) != 0)
        return;

    uint32_t w = fhdr->width;
    uint32_t h = fhdr->height;
    uint64_t ts = fhdr->timestamp;
    uint32_t data_size = fhdr->data_size;

    if (w == 0 || h == 0 || w > MAX_WIDTH || h > MAX_HEIGHT)
        return;

    if (data_size > ctx->pixel_buf_size)
        return;

    const uint8_t *shm_pixels = ctx->shmem_ptr + slot_offset + SLOT_HEADER_SIZE;
    memcpy(ctx->pixel_buf, shm_pixels, data_size);

    int32_t latest_after = __atomic_load_n(&ctrl->latest_index, __ATOMIC_ACQUIRE);
    if (latest_after != latest) {
        return;
    }

    obs_enter_graphics();
    enum gs_color_format format = ctx->use_unorm ? GS_BGRA_UNORM : GS_BGRA;

    if (!ctx->texture || ctx->tex_width != w || ctx->tex_height != h || ctx->current_fmt != format) {
        if (ctx->texture)
            gs_texture_destroy(ctx->texture);
        ctx->texture     = gs_texture_create(w, h, format, 1, NULL, GS_DYNAMIC);
        ctx->tex_width   = w;
        ctx->tex_height  = h;
        ctx->current_fmt = format;
        ctx->last_w      = w;
        ctx->last_h      = h;
        blog(LOG_INFO, "[Mirror Source] Resolution: %ux%u fmt=%u", w, h, (uint32_t)format);
    }

    gs_texture_set_image(ctx->texture, ctx->pixel_buf, w * 4, false);
    obs_leave_graphics();

    ctx->last_timestamp   = ts;
    ctx->last_slot_idx    = latest;
    ctx->last_w           = w;
    ctx->last_h           = h;
}

static void mirror_video_render(void *data, gs_effect_t *effect)
{
    struct mirror_source *ctx = data;

    if (!ctx->texture)
        return;

    const bool linear_srgb = gs_get_linear_srgb();
    const bool previous = gs_framebuffer_srgb_enabled();
    
    if (ctx->srgb_render || linear_srgb) {
        gs_enable_framebuffer_srgb(true);
    }

    gs_effect_t *eff = obs_get_base_effect(linear_srgb
                            ? OBS_EFFECT_DEFAULT_RECT
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
