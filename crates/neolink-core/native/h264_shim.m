// Minimal VideoToolbox H.264 codec for the DL3 transport.
//
// Two reasons this is C and not Rust: VideoToolbox is a plain C API, and the
// session callbacks want a `refcon`, which is less ceremony than threading a
// block through objc2. Everything is intra-only (every frame a keyframe), so a
// damage rectangle can be encoded and decoded on its own with no reference
// frames — which is what the transport needs, since rectangles come and go.
//
// Output is Annex-B: SPS/PPS are prefixed in band on every keyframe. The dock
// never tells us which layout it wants, so Annex-B is chosen because it is
// self-describing.

#import <Foundation/Foundation.h>
#import <VideoToolbox/VideoToolbox.h>
#import <CoreMedia/CoreMedia.h>
#import <CoreVideo/CoreVideo.h>
#include <stdlib.h>
#include <string.h>

typedef struct {
    VTCompressionSessionRef session;
    int32_t width;
    int32_t height;
    int32_t src_width;
    int32_t src_height;
    uint8_t *out;
    size_t len;
    size_t cap;
    int keyframe;
    int failed;
    int callback_ran;
} nl_encoder;

typedef struct {
    VTDecompressionSessionRef session;
    CMVideoFormatDescriptionRef fmt;
    int32_t width;
    int32_t height;
    int decoded;
    int failed;
    int32_t got_w;
    int32_t got_h;
} nl_decoder;

static void nl_append(nl_encoder *e, const uint8_t *bytes, size_t n) {
    if (e->len + n > e->cap) {
        size_t want = (e->len + n) * 2 + 256;
        uint8_t *p = realloc(e->out, want);
        if (!p) { e->failed = 1; return; }
        e->out = p;
        e->cap = want;
    }
    memcpy(e->out + e->len, bytes, n);
    e->len += n;
}

static const uint8_t nl_start_code[4] = {0, 0, 0, 1};

static void nl_emit_parameter_sets(nl_encoder *e, CMFormatDescriptionRef fmt) {
    size_t count = 0;
    if (CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fmt, 0, NULL, NULL, &count, NULL) != noErr) {
        return;
    }
    for (size_t i = 0; i < count; i++) {
        const uint8_t *set = NULL;
        size_t size = 0;
        if (CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fmt, i, &set, &size, NULL, NULL) != noErr) {
            continue;
        }
        nl_append(e, nl_start_code, sizeof nl_start_code);
        nl_append(e, set, size);
    }
}

static void nl_encode_cb(void *refcon, void *source, OSStatus status, VTEncodeInfoFlags flags,
                         CMSampleBufferRef sample) {
    (void)source;
    nl_encoder *e = refcon;
    e->callback_ran = 1;
    if (status != noErr || !sample) { e->failed = 1; return; }
    if (flags & kVTEncodeInfo_FrameDropped) { e->failed = 1; return; }

    CMBlockBufferRef block = CMSampleBufferGetDataBuffer(sample);
    if (!block) { e->failed = 1; return; }
    size_t total = CMBlockBufferGetDataLength(block);
    if (total == 0) { e->failed = 1; return; }

    uint8_t *flat = malloc(total);
    if (!flat) { e->failed = 1; return; }
    if (CMBlockBufferCopyDataBytes(block, 0, total, flat) != kCMBlockBufferNoErr) {
        free(flat);
        e->failed = 1;
        return;
    }

    CFArrayRef attachments = CMSampleBufferGetSampleAttachmentsArray(sample, true);
    if (attachments && CFArrayGetCount(attachments) > 0) {
        CFDictionaryRef d = CFArrayGetValueAtIndex(attachments, 0);
        CFNumberRef notSync = CFDictionaryGetValue(d, kCMSampleAttachmentKey_NotSync);
        int32_t value = 0;
        e->keyframe = !(notSync && CFNumberGetValue(notSync, kCFNumberSInt32Type, &value) && value);
    } else {
        e->keyframe = 1;
    }

    if (e->keyframe) {
        CMFormatDescriptionRef fmt = CMSampleBufferGetFormatDescription(sample);
        if (fmt) nl_emit_parameter_sets(e, fmt);
    }

    // VideoToolbox hands back length-prefixed (AVCC) NAL units; rewrite the
    // 4-byte lengths as Annex-B start codes.
    size_t at = 0;
    while (at + 4 <= total) {
        uint32_t n = ((uint32_t)flat[at] << 24) | ((uint32_t)flat[at + 1] << 16) |
                     ((uint32_t)flat[at + 2] << 8) | (uint32_t)flat[at + 3];
        at += 4;
        if (n == 0 || at + n > total) break;
        nl_append(e, nl_start_code, sizeof nl_start_code);
        nl_append(e, flat + at, n);
        at += n;
    }
    free(flat);
}

// `width`/`height` are the coded size and must be even; `src_*` is the actual
// damage rect, which may be one pixel smaller in either axis.
void *nl_enc_new(int32_t width, int32_t height, int32_t src_width, int32_t src_height) {
    if (width <= 0 || height <= 0 || (width & 1) || (height & 1)) return NULL;
    if (src_width <= 0 || src_height <= 0 || src_width > width || src_height > height) return NULL;
    nl_encoder *e = calloc(1, sizeof *e);
    if (!e) return NULL;
    e->width = width;
    e->height = height;
    e->src_width = src_width;
    e->src_height = src_height;
    const char *hwenv = getenv("NL_SW");
    NSDictionary *spec = hwenv ? @{} : @{
        (__bridge NSString *)kVTVideoEncoderSpecification_EnableHardwareAcceleratedVideoEncoder: @YES,
    };
    OSStatus st = VTCompressionSessionCreate(kCFAllocatorDefault, width, height,
                                             kCMVideoCodecType_H264,
                                             (__bridge CFDictionaryRef)spec, NULL, NULL,
                                             nl_encode_cb, e, &e->session);
    if (st != noErr || !e->session) { free(e); return NULL; }

    VTCompressionSessionPrepareToEncodeFrames(e->session);
    int32_t one = 1;
    CFNumberRef n = CFNumberCreate(NULL, kCFNumberSInt32Type, &one);
    VTSessionSetProperty(e->session, kVTCompressionPropertyKey_MaxKeyFrameInterval, n);
    VTSessionSetProperty(e->session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse);
    VTSessionSetProperty(e->session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue);
    // No inter-frame prediction: a damage rect arrives with no history, so
    // every encoded rect has to stand alone as an IDR.
    VTSessionSetProperty(e->session, kVTCompressionPropertyKey_AllowTemporalCompression,
                         kCFBooleanFalse);
    CFRelease(n);
    return e;
}

// Encode one BGRA rectangle. Returns 1 on success and fills `*out`/`*out_len`
// with a malloc'd Annex-B buffer the caller frees. `*key` says whether the
// frame is a keyframe.
int nl_enc_frame(void *handle, const uint8_t *bgra, size_t stride, uint8_t **out,
                 size_t *out_len, int *key) {
    nl_encoder *e = handle;
    if (!e || !bgra) return 0;
    CVPixelBufferRef px = NULL;
    NSDictionary *attrs = @{
        (__bridge NSString *)kCVPixelBufferIOSurfacePropertiesKey: @{},
    };
    if (CVPixelBufferCreate(kCFAllocatorDefault, e->width, e->height,
                            kCVPixelFormatType_32BGRA,
                            (__bridge CFDictionaryRef)attrs, &px) != kCVReturnSuccess) {
        return 0;
    }
    if (CVPixelBufferLockBaseAddress(px, 0) != kCVReturnSuccess) {
        CFRelease(px);
        return 0;
    }
    uint8_t *dst = CVPixelBufferGetBaseAddress(px);
    size_t dst_stride = CVPixelBufferGetBytesPerRow(px);
    // VideoToolbox needs even dimensions, so an odd damage rect is encoded one
    // pixel wider/taller with the last column and row repeated.
    int32_t copy_w = e->src_width < e->width ? e->src_width : e->width;
    int32_t copy_h = e->src_height < e->height ? e->src_height : e->height;
    size_t row = (size_t)copy_w * 4;
    for (int32_t y = 0; y < e->height; y++) {
        int32_t sy = y < copy_h ? y : copy_h - 1;
        uint8_t *line = dst + (size_t)y * dst_stride;
        memcpy(line, bgra + (size_t)sy * stride, row);
        uint8_t *last = line + row - 4;
        for (int32_t x = copy_w; x < e->width; x++) {
            memcpy(line + (size_t)x * 4, last, 4);
        }
    }
    CVPixelBufferUnlockBaseAddress(px, 0);

    e->len = 0;
    e->keyframe = 0;
    e->failed = 0;
    e->callback_ran = 0;

    VTEncodeInfoFlags flags = 0;
    OSStatus st = VTCompressionSessionEncodeFrame(e->session, px, kCMTimeZero, kCMTimeInvalid,
                                                  NULL, NULL, &flags);
    if (st == noErr) VTCompressionSessionCompleteFrames(e->session, kCMTimeInvalid);
    CFRelease(px);
    if (st != noErr || !e->callback_ran || e->failed || e->len == 0) return 0;

    *out = e->out;
    *out_len = e->len;
    *key = e->keyframe;
    e->out = NULL;
    e->len = 0;
    e->cap = 0;
    return 1;
}

// Release a buffer handed out by `nl_enc_frame`.
void nl_buf_free(void *bytes) { free(bytes); }

void nl_enc_free(void *handle) {
    nl_encoder *e = handle;
    if (!e) return;
    if (e->session) {
        VTCompressionSessionInvalidate(e->session);
        CFRelease(e->session);
    }
    free(e->out);
    free(e);
}

// --- decoder, used only by the round-trip test --------------------------

static void nl_decode_cb(void *refcon, void *source, OSStatus status, VTDecodeInfoFlags flags,
                         CVImageBufferRef image, CMTime pts, CMTime duration) {
    (void)source;
    (void)flags;
    (void)pts;
    (void)duration;
    nl_decoder *d = refcon;
    if (status != noErr || !image) { d->failed = 1; return; }
    d->got_w = (int32_t)CVPixelBufferGetWidth(image);
    d->got_h = (int32_t)CVPixelBufferGetHeight(image);
    if (d->got_w != d->width || d->got_h != d->height) {
        d->failed = 1;
        return;
    }
    d->decoded = 1;
}

void *nl_dec_new(int32_t width, int32_t height) {
    nl_decoder *d = calloc(1, sizeof *d);
    if (!d) return NULL;
    d->width = width;
    d->height = height;
    return d;
}

// Feed one Annex-B frame. Returns 1 when the decoder produced a picture of the
// expected size.
int nl_dec_frame(void *handle, const uint8_t *annexb, size_t len) {
    nl_decoder *d = handle;
    if (!d || !annexb || len < 5) return 0;

    size_t count = 0;
    size_t at = 0;
    while (at + 4 <= len) {
        if (annexb[at] == 0 && annexb[at + 1] == 0 && annexb[at + 2] == 0 && annexb[at + 3] == 1) {
            count++;
            at += 4;
        } else {
            at++;
        }
    }
    if (count == 0) return 0;

    const uint8_t *sets[32];
    size_t sizes[32];
    size_t nsets = 0;
    uint8_t *payload = malloc(len);
    if (!payload) return 0;
    size_t payload_len = 0;

    at = 0;
    while (at + 4 <= len) {
        if (!(annexb[at] == 0 && annexb[at + 1] == 0 && annexb[at + 2] == 0 && annexb[at + 3] == 1)) {
            at++;
            continue;
        }
        size_t begin = at + 4;
        size_t end = begin;
        while (end + 4 <= len &&
               !(annexb[end] == 0 && annexb[end + 1] == 0 && annexb[end + 2] == 0 && annexb[end + 3] == 1)) {
            end++;
        }
        if (end + 4 > len) end = len;
        size_t n = end - begin;
        if (n == 0) { at = end; continue; }
        int type = annexb[begin] & 0x1f;
        if ((type == 7 || type == 8) && nsets < 32) {
            sets[nsets] = annexb + begin;
            sizes[nsets] = n;
            nsets++;
        } else {
            uint32_t be = (uint32_t)n;
            payload[payload_len++] = (uint8_t)(be >> 24);
            payload[payload_len++] = (uint8_t)(be >> 16);
            payload[payload_len++] = (uint8_t)(be >> 8);
            payload[payload_len++] = (uint8_t)be;
            memcpy(payload + payload_len, annexb + begin, n);
            payload_len += n;
        }
        at = end;
    }
    if (payload_len == 0) { free(payload); return 0; }

    if (nsets >= 2) {
        if (d->fmt) { CFRelease(d->fmt); d->fmt = NULL; }
        if (CMVideoFormatDescriptionCreateFromH264ParameterSets(kCFAllocatorDefault, nsets, sets,
                                                               sizes, 4, &d->fmt) != noErr) {
            free(payload);
            return 0;
        }
        if (d->session) {
            VTDecompressionSessionInvalidate(d->session);
            CFRelease(d->session);
            d->session = NULL;
        }
    }
    if (!d->session) {
        if (!d->fmt) { free(payload); return 0; }
        NSDictionary *spec = @{
            (__bridge NSString *)kVTVideoDecoderSpecification_EnableHardwareAcceleratedVideoDecoder: @YES,
        };
        VTDecompressionOutputCallbackRecord record = {
            .decompressionOutputCallback = nl_decode_cb,
            .decompressionOutputRefCon = d,
        };
        if (VTDecompressionSessionCreate(kCFAllocatorDefault, d->fmt,
                                         (__bridge CFDictionaryRef)spec, NULL, &record,
                                         &d->session) != noErr) {
            free(payload);
            return 0;
        }
    }

    CMBlockBufferRef block = NULL;
    if (CMBlockBufferCreateWithMemoryBlock(kCFAllocatorDefault, NULL, payload_len, kCFAllocatorDefault,
                                           NULL, 0, payload_len, 0, &block) != kCMBlockBufferNoErr) {
        free(payload);
        return 0;
    }
    CMBlockBufferReplaceDataBytes(payload, block, 0, payload_len);
    free(payload);

    CMSampleBufferRef sample = NULL;
    size_t sample_size = payload_len;
    if (CMSampleBufferCreateReady(kCFAllocatorDefault, block, d->fmt, 1, 0, NULL, 1, &sample_size,
                                  &sample) != noErr) {
        CFRelease(block);
        return 0;
    }
    CFRelease(block);

    d->decoded = 0;
    d->failed = 0;
    VTDecodeInfoFlags flags = 0;
    OSStatus st = VTDecompressionSessionDecodeFrame(d->session, sample, 0, NULL, &flags);
    if (st == noErr) VTDecompressionSessionWaitForAsynchronousFrames(d->session);
    CFRelease(sample);
    return st == noErr && d->decoded && !d->failed;
}

// Size of the last decoded picture, so a test can see the cropping.
void nl_dec_size(void *handle, int32_t *w, int32_t *h) {
    nl_decoder *d = handle;
    if (!d) return;
    if (w) *w = d->got_w;
    if (h) *h = d->got_h;
}

void nl_dec_free(void *handle) {
    nl_decoder *d = handle;
    if (!d) return;
    if (d->session) {
        VTDecompressionSessionInvalidate(d->session);
        CFRelease(d->session);
    }
    if (d->fmt) CFRelease(d->fmt);
    free(d);
}
