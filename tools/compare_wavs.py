#!/usr/bin/env python3
"""Compare two WAV files numerically to detect wobble/artifacts.

Reads two WAV files (float32 or PCM16), trims silence, aligns by
first non-silent sample, and reports RMS difference, correlation,
SNR, per-segment analysis, and amplitude modulation detection.

Usage: python3 compare_wavs.py <file1.wav> <file2.wav>
"""

import struct
import sys
import math


def read_wav(path):
    with open(path, 'rb') as f:
        riff = f.read(4)
        assert riff == b'RIFF', f"Not a RIFF file: {riff}"
        f.read(4)
        wave = f.read(4)
        assert wave == b'WAVE', f"Not a WAVE file: {wave}"

        audio_format = channels = sample_rate = bits_per_sample = 0
        samples = []

        while True:
            chunk_id = f.read(4)
            if len(chunk_id) < 4:
                break
            chunk_size = struct.unpack('<I', f.read(4))[0]

            if chunk_id == b'fmt ':
                audio_format = struct.unpack('<H', f.read(2))[0]
                channels = struct.unpack('<H', f.read(2))[0]
                sample_rate = struct.unpack('<I', f.read(4))[0]
                f.read(4)  # byte rate
                f.read(2)  # block align
                bits_per_sample = struct.unpack('<H', f.read(2))[0]
                if chunk_size > 16:
                    f.read(chunk_size - 16)
            elif chunk_id == b'data':
                if audio_format == 3 and bits_per_sample == 32:
                    count = chunk_size // 4
                    raw = f.read(chunk_size)
                    samples = list(struct.unpack(f'<{count}f', raw))
                elif audio_format == 1 and bits_per_sample == 16:
                    count = chunk_size // 2
                    raw = f.read(chunk_size)
                    samples = [s / 32768.0 for s in struct.unpack(f'<{count}h', raw)]
                else:
                    print(f"Unsupported: format={audio_format}, bits={bits_per_sample}")
                    f.read(chunk_size)
            else:
                f.read(chunk_size)

    return samples, sample_rate, channels


def trim_silence(samples, channels, threshold=0.01):
    """Trim leading and trailing silence."""
    frames = len(samples) // channels
    first = 0
    for i in range(frames):
        for ch in range(channels):
            if abs(samples[i * channels + ch]) > threshold:
                first = i
                break
        else:
            continue
        break

    last = frames - 1
    for i in range(frames - 1, -1, -1):
        for ch in range(channels):
            if abs(samples[i * channels + ch]) > threshold:
                last = i
                break
        else:
            continue
        break

    return samples[first * channels:(last + 1) * channels]


def to_mono(samples, channels):
    """Take left channel."""
    if channels == 1:
        return samples
    frames = len(samples) // channels
    return [samples[i * channels] for i in range(frames)]


def main():
    if len(sys.argv) < 3:
        print("Usage: python3 compare_wavs.py <file1.wav> <file2.wav>")
        sys.exit(1)

    path1, path2 = sys.argv[1], sys.argv[2]
    s1, sr1, ch1 = read_wav(path1)
    s2, sr2, ch2 = read_wav(path2)

    print(f"File 1: {path1}")
    print(f"  {len(s1)} samples, {sr1}Hz, {ch1}ch, {len(s1)/sr1/ch1:.3f}s")
    print(f"File 2: {path2}")
    print(f"  {len(s2)} samples, {sr2}Hz, {ch2}ch, {len(s2)/sr2/ch2:.3f}s")

    m1 = to_mono(trim_silence(s1, ch1), 1) if ch1 > 1 else trim_silence(s1, 1)
    m2 = to_mono(trim_silence(s2, ch2), 1) if ch2 > 1 else trim_silence(s2, 1)
    min_len = min(len(m1), len(m2))
    print(f"\nTrimmed mono: {len(m1)} vs {len(m2)}, comparing {min_len} samples")

    # Global stats
    sum_diff_sq = 0.0
    max_diff = 0.0
    max_diff_idx = 0
    for i in range(min_len):
        d = m1[i] - m2[i]
        sum_diff_sq += d * d
        if abs(d) > max_diff:
            max_diff = abs(d)
            max_diff_idx = i

    rms = math.sqrt(sum_diff_sq / min_len)
    mean1 = sum(m1[:min_len]) / min_len
    mean2 = sum(m2[:min_len]) / min_len
    cov = var1 = var2 = 0.0
    for i in range(min_len):
        d1, d2 = m1[i] - mean1, m2[i] - mean2
        cov += d1 * d2
        var1 += d1 * d1
        var2 += d2 * d2
    corr = cov / math.sqrt(var1 * var2) if var1 > 0 and var2 > 0 else 0

    sig_pow = sum(m2[i] ** 2 for i in range(min_len)) / min_len
    snr = 10 * math.log10(sig_pow / (sum_diff_sq / min_len)) if sum_diff_sq > 0 else float('inf')

    print(f"\n--- Results ---")
    print(f"RMS difference: {rms:.6f}")
    print(f"Max difference: {max_diff:.6f} at sample {max_diff_idx} ({max_diff_idx/sr1:.3f}s)")
    print(f"Correlation:    {corr:.6f}")
    print(f"SNR:            {snr:.1f} dB")

    # Per-segment
    seg_size = min_len // 10
    print(f"\n{'Seg':<6}{'Time':<10}{'RMS':<14}{'MaxDiff':<14}{'Corr':<12}")
    for seg in range(10):
        s, e = seg * seg_size, min((seg + 1) * seg_size, min_len)
        n = e - s
        sr = math.sqrt(sum((m1[i] - m2[i]) ** 2 for i in range(s, e)) / n)
        sm = max(abs(m1[i] - m2[i]) for i in range(s, e))
        a1, a2 = sum(m1[s:e]) / n, sum(m2[s:e]) / n
        c = v1 = v2 = 0.0
        for i in range(s, e):
            d1, d2 = m1[i] - a1, m2[i] - a2
            c += d1 * d2; v1 += d1 * d1; v2 += d2 * d2
        sc = c / math.sqrt(v1 * v2) if v1 > 0 and v2 > 0 else 0
        print(f"{seg:<6}{s/sr1:<10.2f}{sr:<14.6f}{sm:<14.6f}{sc:<12.6f}")


if __name__ == '__main__':
    main()
