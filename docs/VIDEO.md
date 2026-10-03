# Local video and audio

Kova Image uses Windows Media Foundation Media Engine through the existing
windows-rs dependency. No FFmpeg/libVLC/mpv engine, browser, external playback
process, plugin search or codec download is part of the application.

## Playback contract

- Containers: MP4/M4V, MOV, WebM and MKV, identified by headers. Windows must
  have a compatible decoder for the actual video and audio tracks.
- Play/pause, finite duration/current time, seek, mute/volume, optional loop,
  fullscreen and mixed-media folder navigation are implemented.
- Left/Right navigate files; Ctrl+Left/Right seek five seconds. Space controls
  playback and M controls mute. Focused sliders use arrow keys instead of navigation.
- Image transforms and clipboard frame extraction do not apply to video.
  Ctrl+C and Ctrl+Shift+C copy the video path. Fullscreen fits video to the
  entire viewport, including upscaling smaller sources.
- Autoplay and looping are separate from animated-image settings. Minimized or
  occluded windows pause video/audio while retaining the user's pause preference.
  Moving the window keeps playback running: the frameless title bar moves the
  window itself instead of entering the native modal move loop, which would
  stall frame delivery.
- Each launch owns its window. There is no background service or IPC protocol.

The native worker initializes only for video. Media Engine owns synchronized
audio/video timing. D3D11 acceleration is requested; WARP is the device-creation
fallback. Native decoding may still be software depending on the codec/device.
The app polls for new native frames at approximately 60 Hz while playing, sleeps
between checks and slows polling while paused/hidden. Static images never use
this worker. Video frames and commands use bounded coalescing mailboxes.

![Native video with synthetic test content](images/video.png)

## Limits and tradeoffs

Only regular local drive files up to 32 GiB are admitted. UNC and mapped network
video paths, leaf reparse points other than cloud-sync placeholders, playlists
and URL sources are rejected. A
read-only retained handle denies concurrent writes/deletion. Seeking reads this
same handle through a COM stream, rather than reopening the filename.

MP4/MOV metadata traversal rejects external data references, reference movies,
compressed movie headers, malformed sizes and excessive nesting/box counts.
Container payloads are skipped with seeks. Media Engine receives a byte stream
plus a synthetic type hint; locally registered MF plugins are disabled.

Native dimensions are checked when available: maximum 8,192 per side and
16,777,216 pixels; duration must be finite, positive and at most seven days.
The presentation surface fits the current window and is capped at 3840 x 2160,
preserving aspect ratio and never enlarging a smaller source. A larger monitor
still receives at most a 4K presentation buffer. D3D staging readback and Slint upload are CPU copies, not zero-copy.
Each maximum RGBA presentation buffer is about 33 MiB at 3840 × 2160; native decoded surfaces,
staging, the pending frame and renderer textures are additional memory costs.

Native codecs may allocate before reporting dimensions. These are application
admission limits, not a total-process RAM cap or decoder sandbox. Rust cannot
catch native access violations. OS codec/device loss, hostile-file corpora,
rotation metadata, HDR/color management, unusual aspect ratios and Windows N
installations need broader validation. No subtitles, streaming, playlists, DRM
or codec settings are provided; audio files are played as described below.

## Audio

Audio files go through the same admission, player and controls as video; the
player runs Media Engine in its audio-only mode, so no Direct3D device is created
and nothing is drawn per frame. The state is checked ten times a second.

- Formats, identified by content (an extension decides only for a bare MPEG or ADTS
  stream): MP3 (with or without ID3 tag), M4A/M4B (MP4 audio, by brand or extension),
  AAC (ADTS), WAV, FLAC, Ogg Vorbis, Opus and WMA (ASF with the `.wma` extension).
  Windows must have a decoder for the actual codec; Windows decodes MP3, AAC, WAV,
  FLAC and WMA itself, but not Ogg Vorbis or Opus, which need Microsoft's free
  "Web Media Extensions" from the Microsoft Store. Without them a clear message
  says so (`MF_E_UNSUPPORTED_BYTESTREAM_TYPE`).
- M4A files are checked for external data references like MP4. WMA (ASF) files are
  handed to Windows' own ASF source without inspection by Kova Image.
- Windows reports no duration for a FLAC shorter than about one second, and such a
  file is refused with "Invalid or unsupported media duration". WAV variants that
  Media Foundation does not read (big-endian RIFX, RF64) are not admitted.
- `src/audio.rs` reads the title, artist, album and cover before playback:
  ID3v2.2 to 2.4 and ID3v1 (MP3, and AAC or FLAC with a tag in front), the iTunes
  `ilst` atoms of M4A, and the Vorbis comment and picture blocks of FLAC. Tags are
  best effort and bounded (16 MiB for a tag or block, 32 MiB for `moov`, 16 MiB for
  a picture, 200 characters for a text, at most 10,000 comments); a damaged tag is
  ignored. The cover is decoded by the viewer's own image decoders at the size of
  the window and is not enlarged. WAV, Ogg, Opus and WMA show no tags.
- A song with a cover shows it like a picture; one without shows a panel with the
  title (or the file name), artist and album. The file information lists the tags.
- Navigation, autoplay, looping, volume, mute, minimizing, the slideshow and
  Ctrl+Left/Right seeking work as for video. Image transforms do not apply.

## Reproducible checks

```powershell
python scripts/video-fixtures.py
python scripts/audio-fixtures.py
cargo run --locked --example audio_probe -- tests/fixtures/tone.mp3
cargo run --locked --example audio_scan -- list-of-audio-paths.txt
python scripts/ui-smoke.py --state=audio
cargo run --locked --example video_probe -- artifacts/video-fixtures/clip1.mp4
cargo run --locked --example video_probe -- artifacts/video-fixtures/clip2.mov
cargo run --locked --example video_probe -- artifacts/video-fixtures/clip4.mkv
cargo build --locked
python scripts/ui-smoke.py --state=video
python scripts/ui-smoke.py --state=video --software
```

The fixture script needs a developer-installed FFmpeg or `imageio-ffmpeg`.
It creates synthetic six-second test patterns with an audio tone. The native
probe mutes audio and checks real RGBA frames, pause/seek, end-of-file, looping
and file-handle release after acknowledged Stop. GUI tests mute their own test
instance and verify timeline, controls, fullscreen and mixed/stale navigation.
Codec-dependent playback checks are opt-in; deterministic admission tests run
in ordinary Cargo CI without requiring sound/GPU devices.

Local Windows 11 checks passed for H.264/AAC in MP4, MOV and MKV. VP9/Opus and
VP8/Vorbis WebM fixtures returned `0xC00D5212` (missing matching decoder) on this
machine. WebM playback is therefore **conditional and not locally validated as
successful**. Its error state remains navigable; no codec is silently installed.

Primary API references: [Media Engine byte streams](https://learn.microsoft.com/en-us/windows/win32/api/mfmediaengine/nf-mfmediaengine-imfmediaengineex-setsourcefrombytestream),
[D3D acceleration](https://learn.microsoft.com/en-us/windows/win32/medfound/mf-media-engine-dxgi-manager),
[frame availability](https://learn.microsoft.com/en-us/windows/win32/api/mfmediaengine/nf-mfmediaengine-imfmediaengine-onvideostreamtick),
[Media Foundation formats](https://learn.microsoft.com/en-us/windows/win32/medfound/supported-media-formats-in-media-foundation),
[MPEG-4 source](https://learn.microsoft.com/en-us/windows/win32/medfound/mpeg-4-file-source),
[MKV source](https://learn.microsoft.com/en-us/windows/win32/medfound/mkv-support).
