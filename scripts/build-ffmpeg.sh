#!/usr/bin/env bash
set -euo pipefail
root=$(cygpath -u "$KOVA_NATIVE_ROOT")
prefix=$(cygpath -m "$root/sdk")
mkdir -p "$root/ffmpeg-build"
cd "$root/ffmpeg-build"
export PKG_CONFIG_PATH="$root/sdk/lib/pkgconfig"
"$root/ffmpeg-8.1.3/configure" \
  --toolchain=msvc --arch=x86_64 --target-os=win32 --prefix="$prefix" \
  --enable-shared --disable-static --disable-debug --disable-doc \
  --disable-programs --disable-network --disable-autodetect --disable-everything \
  --disable-avdevice --disable-avfilter --enable-libdav1d \
  --enable-decoder=h264,hevc,vp8,vp9,libdav1d,mpeg1video,mpeg2video,mpeg4,msmpeg4v1,msmpeg4v2,msmpeg4v3,wmv1,wmv2,wmv3,vc1,mjpeg,prores,dnxhd,theora,ffv1,huffyuv,cfhd,aac,aac_fixed,ac3,eac3,dca,truehd,mlp,mp3,mp3float,mp2,opus,vorbis,flac,alac,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_f32le,pcm_f64le,wmav1,wmav2,wmapro,wmalossless \
  --enable-parser=h264,hevc,vp8,vp9,av1,mpegvideo,mpeg4video,vc1,mjpeg,aac,ac3,dca,mpegaudio,opus,vorbis,flac \
  --enable-demuxer=mov,matroska,avi,asf,mpegps,mpegts,flv,mpegvideo
make -j4
make install
