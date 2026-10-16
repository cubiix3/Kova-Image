/* FFmpeg owns its ABI-sensitive structures; Rust sees only this opaque handle. */
#include <errno.h>
#include <math.h>
#include <stdint.h>
#include <string.h>
#include <libavformat/avformat.h>
#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/imgutils.h>
#include <libavutil/display.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>

#define PIXELS 16777216
#define AUDIO_FRAMES 96000

typedef struct KovaVideo {
    AVFormatContext *format;
    AVIOContext *io;
    AVCodecContext *video, *audio;
    AVFrame *frame, *pending, *displayed;
    AVPacket *packet;
    struct SwsContext *scale;
    SwrContext *resample;
    AVChannelLayout audio_layout;
    int audio_rate, audio_format;
    float *samples;
    int video_index, audio_index, receiving, draining;
    double start, last_video_pts, audio_end_pts;
    int rotation, flip;
    uint8_t *rotated;
    size_t rotated_size;
} KovaVideo;

typedef struct KovaMediaInfo {
    int width, height, audio;
    double duration;
} KovaMediaInfo;

typedef struct KovaFrameInfo {
    int kind, samples, width, height;
    double pts;
    const float *audio;
} KovaFrameInfo;

static int deny_open(AVFormatContext *s, AVIOContext **p, const char *url,
                     int flags, AVDictionary **options) {
    (void)s; (void)p; (void)url; (void)flags; (void)options;
    return AVERROR(EPERM);
}

void kova_video_init(void) {
    /* Called once before any FFmpeg decoder or probe runs. */
    av_max_alloc(64 * 1024 * 1024);
    av_log_set_level(AV_LOG_ERROR);
}

void kova_video_close(KovaVideo *v) {
    if (!v) return;
    avformat_close_input(&v->format);
    if (v->io) av_freep(&v->io->buffer);
    avio_context_free(&v->io);
    avcodec_free_context(&v->video);
    avcodec_free_context(&v->audio);
    av_frame_free(&v->frame);
    av_frame_free(&v->pending);
    av_frame_free(&v->displayed);
    av_packet_free(&v->packet);
    sws_freeContext(v->scale);
    swr_free(&v->resample);
    av_channel_layout_uninit(&v->audio_layout);
    av_free(v->samples);
    av_free(v->rotated);
    av_free(v);
}

static int open_decoder(KovaVideo *v, int index, AVCodecContext **out) {
    AVCodecParameters *parameters = v->format->streams[index]->codecpar;
    const AVCodec *codec = avcodec_find_decoder(parameters->codec_id);
    if (!codec) return AVERROR_DECODER_NOT_FOUND;
    *out = avcodec_alloc_context3(codec);
    if (!*out) return AVERROR(ENOMEM);
    int result = avcodec_parameters_to_context(*out, parameters);
    if (result < 0) return result;
    (*out)->max_pixels = PIXELS;
    (*out)->max_samples = AUDIO_FRAMES * 32;
    (*out)->thread_count = 2;
    (*out)->pkt_timebase = v->format->streams[index]->time_base;
    return avcodec_open2(*out, codec, NULL);
}

int kova_video_open(KovaVideo **out, void *opaque,
                    int (*read)(void *, uint8_t *, int),
                    int64_t (*seek)(void *, int64_t, int),
                    int (*interrupt)(void *), KovaMediaInfo *info) {
    KovaVideo *v = av_mallocz(sizeof(*v));
    if (!v) return AVERROR(ENOMEM);
    int result = AVERROR(ENOMEM);
    v->video_index = v->audio_index = v->receiving = -1;
    v->audio_format = -1;
    v->format = avformat_alloc_context();
    uint8_t *buffer = av_malloc(65536);
    if (!v->format || !buffer) { av_free(buffer); goto fail; }
    v->io = avio_alloc_context(buffer, 65536, 0, opaque, read, NULL, seek);
    if (!v->io) { av_free(buffer); goto fail; }
    v->format->pb = v->io;
    v->format->flags |= AVFMT_FLAG_CUSTOM_IO;
    v->format->io_open = deny_open;
    v->format->interrupt_callback.callback = interrupt;
    v->format->interrupt_callback.opaque = opaque;
    v->format->probesize = 1024 * 1024;
    v->format->max_analyze_duration = 2 * AV_TIME_BASE;
    v->format->max_streams = 32;
    v->format->max_index_size = 4 * 1024 * 1024;
    AVDictionary *options = NULL;
    av_dict_set(&options, "protocol_whitelist", "", 0);
    av_dict_set(&options, "format_whitelist", "mov,matroska,webm,avi,asf,mpeg,mpegts,flv", 0);
    result = avformat_open_input(&v->format, NULL, NULL, &options);
    av_dict_free(&options);
    if (result < 0) goto fail;
    unsigned streams = v->format->nb_streams;
    if (!streams || streams > 32) { result = AVERROR_INVALIDDATA; goto fail; }
    AVDictionary *probe_options[32] = {0};
    for (unsigned i = 0; i < streams; i++) {
        AVCodecParameters *parameters = v->format->streams[i]->codecpar;
        if (parameters->width > 8192 || parameters->height > 8192 ||
            (int64_t)parameters->width * parameters->height > PIXELS) {
            result = AVERROR(EFBIG); goto fail;
        }
    }
    for (unsigned i = 0; i < streams; i++) {
        av_dict_set(&probe_options[i], "max_pixels", "16777216", 0);
        av_dict_set(&probe_options[i], "max_samples", "3072000", 0);
        av_dict_set(&probe_options[i], "threads", "2", 0);
    }
    result = avformat_find_stream_info(v->format, probe_options);
    for (unsigned i = 0; i < streams; i++) av_dict_free(&probe_options[i]);
    if (result < 0) goto fail;
    v->video_index = av_find_best_stream(v->format, AVMEDIA_TYPE_VIDEO, -1, -1, NULL, 0);
    if (v->video_index < 0) { result = v->video_index; goto fail; }
    AVCodecParameters *p = v->format->streams[v->video_index]->codecpar;
    if (p->width <= 0 || p->height <= 0 || p->width > 8192 || p->height > 8192 ||
        (int64_t)p->width * p->height > PIXELS) { result = AVERROR(EFBIG); goto fail; }
    AVRational sar = p->sample_aspect_ratio;
    double aspect = sar.num > 0 && sar.den > 0 ? av_q2d(sar) : 1.0;
    if (aspect < 0.0625 || aspect > 16) { result = AVERROR_INVALIDDATA; goto fail; }
    const AVPacketSideData *matrix = av_packet_side_data_get(p->coded_side_data,
        p->nb_coded_side_data, AV_PKT_DATA_DISPLAYMATRIX);
    if (matrix && matrix->size >= 9 * sizeof(int32_t)) {
        const int32_t *m = (const int32_t *)matrix->data;
        int32_t normalized[9];
        memcpy(normalized, m, sizeof(normalized));
        v->flip = (double)m[0] * m[4] - (double)m[1] * m[3] < 0;
        if (v->flip) {
            if (m[0] == INT32_MIN || m[1] == INT32_MIN) { result = AVERROR_INVALIDDATA; goto fail; }
            normalized[0] = -normalized[0];
            normalized[1] = -normalized[1];
        }
        double rotation = -av_display_rotation_get(normalized);
        if (!isfinite(rotation)) { result = AVERROR_INVALIDDATA; goto fail; }
        v->rotation = ((int)round(rotation / 90.0) % 4 + 4) % 4;
    }
    info->width = (int)round(p->width * aspect);
    info->height = p->height;
    if (v->rotation & 1) { int tmp = info->width; info->width = info->height; info->height = tmp; }
    info->duration = (double)v->format->duration / AV_TIME_BASE;
    if (!isfinite(info->duration) || info->duration <= 0 || info->duration > 604800) {
        result = AVERROR_INVALIDDATA; goto fail;
    }
    v->start = v->format->start_time == AV_NOPTS_VALUE ? 0 :
        (double)v->format->start_time / AV_TIME_BASE;
    result = open_decoder(v, v->video_index, &v->video);
    if (result < 0) goto fail;
    v->audio_index = av_find_best_stream(v->format, AVMEDIA_TYPE_AUDIO, -1, v->video_index, NULL, 0);
    if (v->audio_index >= 0) {
        result = open_decoder(v, v->audio_index, &v->audio);
        if (result < 0) goto fail;
    }
    info->audio = v->audio != NULL;
    v->frame = av_frame_alloc();
    v->pending = av_frame_alloc();
    v->displayed = av_frame_alloc();
    v->packet = av_packet_alloc();
    v->samples = av_malloc(AUDIO_FRAMES * 2 * sizeof(float));
    if (!v->frame || !v->pending || !v->displayed || !v->packet || !v->samples) {
        result = AVERROR(ENOMEM); goto fail;
    }
    *out = v;
    return 0;
fail:
    kova_video_close(v);
    return result;
}

static int audio_frame(KovaVideo *v, KovaFrameInfo *info) {
    AVFrame *f = v->frame;
    if (f->sample_rate <= 0 || f->sample_rate > 384000 || f->nb_samples < 0 ||
        f->nb_samples > AUDIO_FRAMES || f->ch_layout.nb_channels < 1 ||
        f->ch_layout.nb_channels > 32) return AVERROR_INVALIDDATA;
    if (!v->resample || v->audio_rate != f->sample_rate || v->audio_format != f->format ||
        av_channel_layout_compare(&v->audio_layout, &f->ch_layout)) {
        swr_free(&v->resample);
        av_channel_layout_uninit(&v->audio_layout);
        AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
        int result = swr_alloc_set_opts2(&v->resample, &stereo, AV_SAMPLE_FMT_FLT, 48000,
            &f->ch_layout, (enum AVSampleFormat)f->format, f->sample_rate, 0, NULL);
        if (result < 0) return result;
        result = swr_init(v->resample);
        if (result < 0) return result;
        result = av_channel_layout_copy(&v->audio_layout, &f->ch_layout);
        if (result < 0) return result;
        v->audio_rate = f->sample_rate;
        v->audio_format = f->format;
    }
    if (swr_get_out_samples(v->resample, f->nb_samples) > AUDIO_FRAMES) return AVERROR(EFBIG);
    /* Converted samples include the delay retained from the preceding input frame. */
    info->pts -= (double)swr_get_delay(v->resample, f->sample_rate) / f->sample_rate;
    uint8_t *output = (uint8_t *)v->samples;
    int samples = swr_convert(v->resample, &output, AUDIO_FRAMES,
        (const uint8_t **)f->extended_data, f->nb_samples);
    if (samples < 0) return samples;
    info->kind = 2;
    info->audio = v->samples;
    info->samples = samples;
    v->audio_end_pts = info->pts + (double)samples / 48000;
    return 2;
}

/* Returns one decoded video or audio frame; never accumulates a frame sequence. */
int kova_video_next(KovaVideo *v, KovaFrameInfo *info) {
    for (;;) {
        if (v->format->interrupt_callback.callback(v->format->interrupt_callback.opaque))
            return AVERROR_EXIT;
        if (v->receiving >= 0) {
            AVCodecContext *decoder = v->receiving == v->video_index ? v->video : v->audio;
            av_frame_unref(v->frame);
            int result = avcodec_receive_frame(decoder, v->frame);
            if (result >= 0) {
                AVStream *stream = v->format->streams[v->receiving];
                int64_t pts = v->frame->best_effort_timestamp;
                info->pts = pts == AV_NOPTS_VALUE ? v->last_video_pts :
                    pts * av_q2d(stream->time_base) - v->start;
                if (!isfinite(info->pts) || info->pts < -604800 ||
                    info->pts > (double)v->format->duration / AV_TIME_BASE + 10) return AVERROR_INVALIDDATA;
                if (v->receiving == v->audio_index) return audio_frame(v, info);
                if (v->frame->width <= 0 || v->frame->height <= 0 ||
                    v->frame->width > 8192 || v->frame->height > 8192 ||
                    (int64_t)v->frame->width * v->frame->height > PIXELS) return AVERROR(EFBIG);
                v->last_video_pts = info->pts;
                av_frame_unref(v->pending);
                result = av_frame_ref(v->pending, v->frame);
                if (result < 0) return result;
                info->kind = 1;
                AVRational sar = v->frame->sample_aspect_ratio;
                double aspect = sar.num > 0 && sar.den > 0 ? av_q2d(sar) : 1.0;
                if (aspect < 0.0625 || aspect > 16) return AVERROR_INVALIDDATA;
                info->width = (int)round(v->frame->width * aspect);
                info->height = v->frame->height;
                if (v->rotation & 1) { int tmp = info->width; info->width = info->height; info->height = tmp; }
                return 1;
            }
            if (result != AVERROR(EAGAIN) && result != AVERROR_EOF) return result;
            v->receiving = -1;
        }
        if (v->draining) {
            if (v->draining == 1 && v->audio) {
                v->draining = 2;
                int result = avcodec_send_packet(v->audio, NULL);
                if (result < 0 && result != AVERROR_EOF) return result;
                v->receiving = v->audio_index;
                continue;
            }
            if (v->resample) {
                uint8_t *output = (uint8_t *)v->samples;
                int samples = swr_convert(v->resample, &output, AUDIO_FRAMES, NULL, 0);
                if (samples < 0) return samples;
                if (samples) {
                    info->kind = 2;
                    info->samples = samples;
                    info->audio = v->samples;
                    info->pts = v->audio_end_pts;
                    v->audio_end_pts += (double)samples / 48000;
                    return 2;
                }
            }
            return 0;
        }
        av_packet_unref(v->packet);
        int result = av_read_frame(v->format, v->packet);
        if (result == AVERROR_EOF) {
            v->draining = 1;
            result = avcodec_send_packet(v->video, NULL);
            if (result < 0 && result != AVERROR_EOF) return result;
            v->receiving = v->video_index;
            continue;
        }
        if (result < 0) return result;
        if (v->packet->size > 16 * 1024 * 1024) return AVERROR(EFBIG);
        AVCodecContext *decoder = NULL;
        if (v->packet->stream_index == v->video_index) decoder = v->video;
        else if (v->packet->stream_index == v->audio_index) decoder = v->audio;
        if (!decoder) continue;
        result = avcodec_send_packet(decoder, v->packet);
        if (result < 0) return result;
        v->receiving = v->packet->stream_index;
    }
}

int kova_video_seek(KovaVideo *v, double seconds) {
    double timestamp = (seconds + v->start) * AV_TIME_BASE;
    if (!isfinite(timestamp) || timestamp <= (double)INT64_MIN || timestamp >= (double)INT64_MAX)
        return AVERROR_INVALIDDATA;
    int64_t time = (int64_t)timestamp;
    int result = avformat_seek_file(v->format, -1, INT64_MIN, time, time, AVSEEK_FLAG_BACKWARD);
    if (result < 0) return result;
    avcodec_flush_buffers(v->video);
    if (v->audio) avcodec_flush_buffers(v->audio);
    swr_free(&v->resample);
    av_frame_unref(v->pending);
    av_frame_unref(v->displayed);
    av_packet_unref(v->packet);
    v->receiving = -1;
    v->draining = 0;
    v->last_video_pts = seconds;
    v->audio_end_pts = seconds;
    return 0;
}

void kova_video_present(KovaVideo *v) {
    av_frame_unref(v->displayed);
    av_frame_move_ref(v->displayed, v->pending);
}

int kova_video_render(KovaVideo *v, uint8_t *pixels, int width, int height) {
    if (!v->displayed->data[0] || width < 1 || height < 1 || width > 3840 || height > 2160)
        return AVERROR(EINVAL);
    int scaled_width = v->rotation & 1 ? height : width;
    int scaled_height = v->rotation & 1 ? width : height;
    v->scale = sws_getCachedContext(v->scale, v->displayed->width, v->displayed->height,
        (enum AVPixelFormat)v->displayed->format, scaled_width, scaled_height, AV_PIX_FMT_RGBA,
        SWS_BILINEAR, NULL, NULL, NULL);
    if (!v->scale) return AVERROR(ENOMEM);
    int space = SWS_CS_DEFAULT;
    if (v->displayed->colorspace == AVCOL_SPC_BT709) space = SWS_CS_ITU709;
    else if (v->displayed->colorspace == AVCOL_SPC_SMPTE240M) space = SWS_CS_SMPTE240M;
    else if (v->displayed->colorspace == AVCOL_SPC_BT2020_NCL ||
             v->displayed->colorspace == AVCOL_SPC_BT2020_CL) space = SWS_CS_BT2020;
    const int *coefficients = sws_getCoefficients(space);
    int result = sws_setColorspaceDetails(v->scale, coefficients,
        v->displayed->color_range == AVCOL_RANGE_JPEG, coefficients, 1, 0, 1 << 16, 1 << 16);
    if (result < 0) return result;
    uint8_t *target = pixels;
    if (v->rotation || v->flip) {
        size_t bytes = (size_t)width * height * 4;
        if (v->rotated_size < bytes) {
            uint8_t *buffer = av_realloc(v->rotated, bytes);
            if (!buffer) return AVERROR(ENOMEM);
            v->rotated = buffer;
            v->rotated_size = bytes;
        }
        target = v->rotated;
    }
    uint8_t *planes[4] = {target, NULL, NULL, NULL};
    int strides[4] = {scaled_width * 4, 0, 0, 0};
    result = sws_scale(v->scale, (const uint8_t *const *)v->displayed->data,
        v->displayed->linesize, 0, v->displayed->height, planes, strides);
    if (result != scaled_height) return AVERROR_INVALIDDATA;
    if (target != pixels) {
        for (int y = 0; y < height; y++) for (int x = 0; x < width; x++) {
            int sx = x, sy = y;
            if (v->rotation == 1) { sx = y; sy = scaled_height - 1 - x; }
            else if (v->rotation == 2) { sx = scaled_width - 1 - x; sy = scaled_height - 1 - y; }
            else if (v->rotation == 3) { sx = scaled_width - 1 - y; sy = x; }
            if (v->flip) sx = scaled_width - 1 - sx;
            memcpy(pixels + ((size_t)y * width + x) * 4,
                target + ((size_t)sy * scaled_width + sx) * 4, 4);
        }
    }
    return 0;
}

void kova_video_error(int code, char *buffer, int size) {
    av_strerror(code, buffer, (size_t)size);
}
