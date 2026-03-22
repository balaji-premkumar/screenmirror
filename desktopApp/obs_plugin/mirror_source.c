/**
 * OBS Studio Source Plugin — Mirror Stream
 *
 * Reads decoded video frames from a POSIX shared memory segment
 * (`/mirror_obs_feed`) written by the Mirror desktop backend,
 * and renders them as an OBS video source.
 *
 * Shared memory layout (must match Rust FrameHeader with #[repr(C)]):
 *   Offset  0: uint8_t  magic[4]    "MIRR"
 *   Offset  4: uint32_t width
 *   Offset  8: uint32_t height
 *   Offset 16: uint64_t timestamp   (padding at 12-15 for alignment)
 *   Offset 24: uint8_t  pixels[width * height * 4]  (BGRA)
 *
 * Build:
 *   gcc -shared -fPIC -o mirror-source.so mirror_source.c \
 *       -I/usr/include/obs -lobs -lrt
 *
 * Install:
 *   mkdir -p ~/.config/obs-studio/plugins/mirror-source/bin/64bit
 *   cp mirror-source.so ~/.config/obs-studio/plugins/mirror-source/bin/64bit/
 */

#include <obs-module.h>
#include <graphics/graphics.h>

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __linux__
#include <fcntl.h>
#include <sys/mman.h>
#include <sys/stat.h>
#include <unistd.h>
#endif

OBS_DECLARE_MODULE()
OBS_MODULE_USE_DEFAULT_LOCALE("mirror-source", "en-US")

#define SHM_NAME    "/mirror_obs_feed"
#define MAGIC_MIRR  0x5252494D  /* "MIRR" as little-endian uint32 */

/* Must match Rust's FrameHeader with #[repr(C)] on x86-64 */
struct frame_header {
    uint8_t  magic[4];
    uint32_t width;
    uint32_t height;
    /* 4 bytes padding here for uint64_t alignment */
    uint64_t timestamp;
};

#define HEADER_SIZE sizeof(struct frame_header)  /* 24 bytes on x86-64 */

struct mirror_source {
    obs_source_t *source;

    /* Shared memory */
    uint8_t      *shmem_ptr;
    size_t        shmem_size;
    int           shmem_fd;
    bool          shmem_open;

    /* Rendering */
    gs_texture_t *texture;
    uint32_t      tex_width;
    uint32_t      tex_height;
    uint64_t      last_timestamp;
};

/* ── Forward declarations ─────────────────────────────────── */
static const char *mirror_get_name(void *unused);
static void       *mirror_create(obs_data_t *settings, obs_source_t *source);
static void        mirror_destroy(void *data);
static uint32_t    mirror_get_width(void *data);
static uint32_t    mirror_get_height(void *data);
static void        mirror_video_tick(void *data, float seconds);
static void        mirror_video_render(void *data, gs_effect_t *effect);

/* ── Source info registration ─────────────────────────────── */
static struct obs_source_info mirror_source_info = {
    .id             = "mirror_stream_source",
    .type           = OBS_SOURCE_TYPE_INPUT,
    .output_flags   = OBS_SOURCE_VIDEO | OBS_SOURCE_CUSTOM_DRAW,
    .get_name       = mirror_get_name,
    .create         = mirror_create,
    .destroy        = mirror_destroy,
    .get_width      = mirror_get_width,
    .get_height     = mirror_get_height,
    .video_tick     = mirror_video_tick,
    .video_render   = mirror_video_render,
};

bool obs_module_load(void)
{
    obs_register_source(&mirror_source_info);
    blog(LOG_INFO, "[Mirror Source] Plugin loaded — looking for /dev/shm%s", SHM_NAME);
    return true;
}

void obs_module_unload(void)
{
    blog(LOG_INFO, "[Mirror Source] Plugin unloaded");
}

/* ── Helpers ──────────────────────────────────────────────── */

static bool try_open_shmem(struct mirror_source *ctx)
{
#ifdef __linux__
    if (ctx->shmem_open)
        return true;

    int fd = shm_open(SHM_NAME, O_RDONLY, 0);
    if (fd < 0)
        return false;

    struct stat st;
    if (fstat(fd, &st) != 0 || st.st_size < (off_t)HEADER_SIZE) {
        close(fd);
        return false;
    }

    void *ptr = mmap(NULL, (size_t)st.st_size, PROT_READ, MAP_SHARED, fd, 0);
    if (ptr == MAP_FAILED) {
        close(fd);
        return false;
    }

    ctx->shmem_ptr  = (uint8_t *)ptr;
    ctx->shmem_size = (size_t)st.st_size;
    ctx->shmem_fd   = fd;
    ctx->shmem_open = true;

    blog(LOG_INFO, "[Mirror Source] Shared memory opened (%zu bytes)", ctx->shmem_size);
    return true;
#else
    (void)ctx;
    return false;
#endif
}

static void close_shmem(struct mirror_source *ctx)
{
#ifdef __linux__
    if (!ctx->shmem_open)
        return;
    munmap(ctx->shmem_ptr, ctx->shmem_size);
    close(ctx->shmem_fd);
    ctx->shmem_ptr  = NULL;
    ctx->shmem_size = 0;
    ctx->shmem_fd   = -1;
    ctx->shmem_open = false;
#else
    (void)ctx;
#endif
}

/* ── Source callbacks ─────────────────────────────────────── */

static const char *mirror_get_name(void *unused)
{
    UNUSED_PARAMETER(unused);
    return "Mirror Stream (USB)";
}

static void *mirror_create(obs_data_t *settings, obs_source_t *source)
{
    UNUSED_PARAMETER(settings);

    struct mirror_source *ctx = bzalloc(sizeof(*ctx));
    ctx->source     = source;
    ctx->shmem_fd   = -1;
    ctx->shmem_open = false;
    ctx->texture    = NULL;
    ctx->tex_width  = 0;
    ctx->tex_height = 0;
    ctx->last_timestamp = 0;

    blog(LOG_INFO, "[Mirror Source] Source created");
    return ctx;
}

static void mirror_destroy(void *data)
{
    struct mirror_source *ctx = data;

    obs_enter_graphics();
    if (ctx->texture) {
        gs_texture_destroy(ctx->texture);
        ctx->texture = NULL;
    }
    obs_leave_graphics();

    close_shmem(ctx);
    bfree(ctx);

    blog(LOG_INFO, "[Mirror Source] Source destroyed");
}

static uint32_t mirror_get_width(void *data)
{
    struct mirror_source *ctx = data;
    return ctx->tex_width > 0 ? ctx->tex_width : 1920;
}

static uint32_t mirror_get_height(void *data)
{
    struct mirror_source *ctx = data;
    return ctx->tex_height > 0 ? ctx->tex_height : 1080;
}

static void mirror_video_tick(void *data, float seconds)
{
    UNUSED_PARAMETER(seconds);
    struct mirror_source *ctx = data;

    /* Try to open shared memory if not already open */
    if (!ctx->shmem_open) {
        try_open_shmem(ctx);
        return;
    }

    /* Validate header */
    if (ctx->shmem_size < HEADER_SIZE)
        return;

    const struct frame_header *hdr = (const struct frame_header *)ctx->shmem_ptr;

    /* Check magic */
    if (memcmp(hdr->magic, "MIRR", 4) != 0) {
        /* Shared memory exists but mirror app hasn't written yet */
        return;
    }

    uint32_t w = hdr->width;
    uint32_t h = hdr->height;
    uint64_t ts = hdr->timestamp;

    if (w == 0 || h == 0 || w > 7680 || h > 4320)
        return;

    /* Skip if this is the same frame we already rendered */
    if (ts == ctx->last_timestamp && ctx->texture)
        return;

    size_t pixel_data_size = (size_t)w * (size_t)h * 4;
    if (ctx->shmem_size < HEADER_SIZE + pixel_data_size)
        return;

    const uint8_t *pixels = ctx->shmem_ptr + HEADER_SIZE;

    /* Create or recreate texture if dimensions changed */
    obs_enter_graphics();
    if (!ctx->texture || ctx->tex_width != w || ctx->tex_height != h) {
        if (ctx->texture)
            gs_texture_destroy(ctx->texture);
        ctx->texture = gs_texture_create(w, h, GS_BGRA, 1, NULL, GS_DYNAMIC);
        ctx->tex_width  = w;
        ctx->tex_height = h;
        blog(LOG_INFO, "[Mirror Source] Texture created/resized: %ux%u", w, h);
    }

    /* Upload pixel data to GPU texture */
    gs_texture_set_image(ctx->texture, pixels, w * 4, false);
    obs_leave_graphics();

    ctx->last_timestamp = ts;
}

static void mirror_video_render(void *data, gs_effect_t *effect)
{
    struct mirror_source *ctx = data;

    if (!ctx->texture)
        return;

    const bool linear_srgb = gs_get_linear_srgb();

    const bool previous = gs_framebuffer_srgb_enabled();
    gs_enable_framebuffer_srgb(linear_srgb);

    gs_effect_t *eff = obs_get_base_effect(linear_srgb
                            ? OBS_EFFECT_DEFAULT_RECT
                            : OBS_EFFECT_DEFAULT);

    gs_eparam_t *param = gs_effect_get_param_by_name(eff, "image");
    gs_effect_set_texture_srgb(param, ctx->texture);

    gs_draw_sprite(ctx->texture, 0, ctx->tex_width, ctx->tex_height);

    gs_enable_framebuffer_srgb(previous);
}
