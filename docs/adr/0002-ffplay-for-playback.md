# 0002 — Delegate playback to a child ffplay process

## Context

The desktop app needs to show the stream and play its sound. It previously did
this itself: an SDL2 window for video, and an audio device opened directly the
first time an audio packet arrived.

That had three problems.

The audio device was opened without anyone asking. A user who plugged in a
phone to feed OBS got sound out of their speakers as a side effect, with no
control to stop it.

SDL2 must create its window on the main thread on macOS. The decode thread is
not the main thread, so the preview could not work there at all.

And it added `libsdl2-dev` and `libasound2-dev` to the build requirements of a
project that otherwise only needs FFmpeg.

## Decision

Do not render or play anything in-process. Remux the incoming HEVC and PCM into
a Matroska stream and pipe it into a child `ffplay`, which owns the window and
the audio device.

Playback starts only when the user presses the button.

## Consequences

The app opens no audio device, ever. Sound reaches the machine through `ffplay`
or through the OBS feed, and both are opt-in. That is the property worth
protecting: if a third sink is added, it belongs in `sinks/` next to the
comment explaining why there are only two.

SDL2 and ALSA leave the dependency list, and the macOS main-thread problem
disappears with them.

Video reaches `ffplay` as an HEVC passthrough rather than as decoded frames, so
a playback-only session does not pay for decode in this process at all. The
decoder still runs briefly, because the Matroska header needs the frame size
and that is where it comes from.

The costs:

- `ffplay` has to be present. It is detected at startup and the playback button
  is hidden when it is missing.
- Both streams go into one pipe, because `ffplay` accepts a single input. That
  is why there is a muxer here rather than two pipes.
- Matroska needs an `hvcC` CodecPrivate block built from the stream's parameter
  sets. Getting that wrong fails with `AVERROR_INVALIDDATA` at
  `avformat_write_header` and nothing plays — which is exactly what happened
  the first time this was built.
- `max_interleave_delta` is set so video is not held back waiting for audio
  that may never arrive, since audio is absent whenever the user denied the
  microphone permission.
