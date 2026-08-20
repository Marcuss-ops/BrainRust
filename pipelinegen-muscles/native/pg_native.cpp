extern "C" {
#include <libavcodec/avcodec.h>
#include <libavformat/avformat.h>
#include <libavutil/error.h>
#include <libavutil/hwcontext.h>
#include <libavutil/pixdesc.h>
}

#include <cstdio>
#include <cstring>

namespace {

void set_error(char *buffer, size_t capacity, const char *message) {
    if (!buffer || capacity == 0) return;
    std::snprintf(buffer, capacity, "%s", message ? message : "native FFmpeg error");
}

void set_ff_error(char *buffer, size_t capacity, const char *prefix, int error) {
    char detail[AV_ERROR_MAX_STRING_SIZE] = {};
    av_strerror(error, detail, sizeof(detail));
    std::snprintf(buffer, capacity, "%s: %s", prefix, detail);
}

AVPixelFormat choose_cuda_format(AVCodecContext *, const AVPixelFormat *formats) {
    for (const AVPixelFormat *format = formats; *format != AV_PIX_FMT_NONE; ++format) {
        if (*format == AV_PIX_FMT_CUDA) return *format;
    }
    return formats[0];
}

bool is_cuda_frame(const AVFrame *frame) {
    return frame && frame->format == AV_PIX_FMT_CUDA;
}

} // namespace

extern "C" int pg_native_render_clip(const char *input_path, const char *output_path,
                                       unsigned width, unsigned height,
                                       char *error_buffer, size_t error_capacity) {
    AVFormatContext *input = nullptr;
    AVFormatContext *output = nullptr;
    AVCodecContext *decoder = nullptr;
    AVCodecContext *encoder = nullptr;
    AVBufferRef *device = nullptr;
    AVStream *in_video = nullptr;
    AVStream *in_audio = nullptr;
    AVStream *out_video = nullptr;
    AVStream *out_audio = nullptr;
    int video_index = -1;
    int audio_index = -1;
    int result = -1;

    if (!input_path || !output_path || width == 0 || height == 0) {
        set_error(error_buffer, error_capacity, "invalid native render arguments");
        return -1;
    }
    if (avformat_open_input(&input, input_path, nullptr, nullptr) < 0 ||
        avformat_find_stream_info(input, nullptr) < 0) {
        set_error(error_buffer, error_capacity, "cannot open or probe input");
        goto cleanup;
    }
    for (unsigned i = 0; i < input->nb_streams; ++i) {
        if (input->streams[i]->codecpar->codec_type == AVMEDIA_TYPE_VIDEO && video_index < 0) {
            video_index = static_cast<int>(i);
            in_video = input->streams[i];
        } else if (input->streams[i]->codecpar->codec_type == AVMEDIA_TYPE_AUDIO && audio_index < 0) {
            audio_index = static_cast<int>(i);
            in_audio = input->streams[i];
        }
    }
    if (video_index < 0 || !in_video || in_video->codecpar->width != static_cast<int>(width) ||
        in_video->codecpar->height != static_cast<int>(height)) {
        set_error(error_buffer, error_capacity, "native path requires matching video geometry");
        goto cleanup;
    }

    {
        const AVCodec *decoder_codec = avcodec_find_decoder(in_video->codecpar->codec_id);
        if (!decoder_codec) { set_error(error_buffer, error_capacity, "decoder unavailable"); goto cleanup; }
        decoder = avcodec_alloc_context3(decoder_codec);
        if (!decoder || avcodec_parameters_to_context(decoder, in_video->codecpar) < 0) {
            set_error(error_buffer, error_capacity, "decoder context setup failed"); goto cleanup;
        }
        if (av_hwdevice_ctx_create(&device, AV_HWDEVICE_TYPE_CUDA, nullptr, nullptr, 0) < 0) {
            set_error(error_buffer, error_capacity, "CUDA device context unavailable"); goto cleanup;
        }
        decoder->hw_device_ctx = av_buffer_ref(device);
        decoder->get_format = choose_cuda_format;
        if (avcodec_open2(decoder, decoder_codec, nullptr) < 0) {
            set_error(error_buffer, error_capacity, "CUDA decoder open failed"); goto cleanup;
        }
    }

    {
        const AVCodec *encoder_codec = avcodec_find_encoder_by_name("h264_nvenc");
        if (!encoder_codec) { set_error(error_buffer, error_capacity, "h264_nvenc unavailable"); goto cleanup; }
        encoder = avcodec_alloc_context3(encoder_codec);
        if (!encoder) { set_error(error_buffer, error_capacity, "encoder allocation failed"); goto cleanup; }
        encoder->width = static_cast<int>(width);
        encoder->height = static_cast<int>(height);
        encoder->time_base = in_video->time_base.num ? in_video->time_base : AVRational{1, 30};
        encoder->framerate = av_guess_frame_rate(input, in_video, nullptr);
        encoder->pix_fmt = AV_PIX_FMT_CUDA;
        encoder->bit_rate = 0;
        encoder->gop_size = 60;
        encoder->max_b_frames = 0;
        encoder->hw_device_ctx = av_buffer_ref(device);
        if (avcodec_open2(encoder, encoder_codec, nullptr) < 0) {
            set_error(error_buffer, error_capacity, "NVENC open failed"); goto cleanup;
        }
    }

    if (avformat_alloc_output_context2(&output, nullptr, nullptr, output_path) < 0 || !output) {
        set_error(error_buffer, error_capacity, "output context allocation failed"); goto cleanup;
    }
    out_video = avformat_new_stream(output, nullptr);
    if (!out_video || avcodec_parameters_from_context(out_video->codecpar, encoder) < 0) {
        set_error(error_buffer, error_capacity, "output video stream setup failed"); goto cleanup;
    }
    out_video->time_base = encoder->time_base;
    if (in_audio) {
        out_audio = avformat_new_stream(output, nullptr);
        if (!out_audio || avcodec_parameters_copy(out_audio->codecpar, in_audio->codecpar) < 0) {
            set_error(error_buffer, error_capacity, "output audio stream setup failed"); goto cleanup;
        }
        out_audio->time_base = in_audio->time_base;
    }
    if (!(output->oformat->flags & AVFMT_NOFILE) && avio_open(&output->pb, output_path, AVIO_FLAG_WRITE) < 0) {
        set_error(error_buffer, error_capacity, "output open failed"); goto cleanup;
    }
    if (avformat_write_header(output, nullptr) < 0) {
        set_error(error_buffer, error_capacity, "output header failed"); goto cleanup;
    }

    {
        AVPacket packet;
        AVFrame *frame = av_frame_alloc();
        AVPacket *encoded = av_packet_alloc();
        if (!frame || !encoded) { set_error(error_buffer, error_capacity, "frame allocation failed"); goto cleanup; }
        while (av_read_frame(input, &packet) >= 0) {
            if (packet.stream_index == video_index) {
                if (avcodec_send_packet(decoder, &packet) >= 0) {
                    while (avcodec_receive_frame(decoder, frame) >= 0) {
                        if (!is_cuda_frame(frame) || avcodec_send_frame(encoder, frame) < 0) {
                            av_packet_unref(&packet); set_error(error_buffer, error_capacity, "CUDA frame encode failed");
                            av_frame_free(&frame); av_packet_free(&encoded); goto cleanup;
                        }
                        while (avcodec_receive_packet(encoder, encoded) >= 0) {
                            av_packet_rescale_ts(encoded, encoder->time_base, out_video->time_base);
                            encoded->stream_index = out_video->index;
                            av_interleaved_write_frame(output, encoded);
                            av_packet_unref(encoded);
                        }
                        av_frame_unref(frame);
                    }
                }
            } else if (packet.stream_index == audio_index && out_audio) {
                av_packet_rescale_ts(&packet, in_audio->time_base, out_audio->time_base);
                packet.stream_index = out_audio->index;
                av_interleaved_write_frame(output, &packet);
            }
            av_packet_unref(&packet);
        }
        avcodec_send_packet(decoder, nullptr);
        while (avcodec_receive_frame(decoder, frame) >= 0) {
            if (avcodec_send_frame(encoder, frame) < 0) break;
            while (avcodec_receive_packet(encoder, encoded) >= 0) {
                av_packet_rescale_ts(encoded, encoder->time_base, out_video->time_base);
                encoded->stream_index = out_video->index;
                av_interleaved_write_frame(output, encoded);
                av_packet_unref(encoded);
            }
        }
        avcodec_send_frame(encoder, nullptr);
        while (avcodec_receive_packet(encoder, encoded) >= 0) {
            av_packet_rescale_ts(encoded, encoder->time_base, out_video->time_base);
            encoded->stream_index = out_video->index;
            av_interleaved_write_frame(output, encoded);
            av_packet_unref(encoded);
        }
        av_frame_free(&frame);
        av_packet_free(&encoded);
    }
    av_write_trailer(output);
    result = 0;

cleanup:
    if (result != 0 && error_buffer && error_capacity && error_buffer[0] == '\0') set_error(error_buffer, error_capacity, "native render failed");
    if (output && !(output->oformat->flags & AVFMT_NOFILE) && output->pb) avio_closep(&output->pb);
    if (output) avformat_free_context(output);
    if (decoder) avcodec_free_context(&decoder);
    if (encoder) avcodec_free_context(&encoder);
    if (device) av_buffer_unref(&device);
    if (input) avformat_close_input(&input);
    return result;
}
