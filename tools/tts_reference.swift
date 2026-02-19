#!/usr/bin/env swift
//
// tts_reference.swift - Generate reference WAV files from AVSpeechSynthesizer
//
// Uses the EXACT same API (writeUtterance:toBufferCallback:) and parameters
// that omnivox uses, to produce a clean reference for comparison.
//
// Usage: swift tts_reference.swift <voice_name> <output.wav> [text]
//
// Examples:
//   swift tts_reference.swift "Alex" alex_reference.wav "Hello world"
//   swift tts_reference.swift "Samantha (Enhanced)" samantha_reference.wav

import AVFoundation
import Foundation

// WAV header writer
func writeWAV(samples: [Float], sampleRate: Int, channels: Int, to url: URL) throws {
    let bytesPerSample = 4 // Float32
    let dataSize = samples.count * bytesPerSample
    let headerSize = 44

    var data = Data(capacity: headerSize + dataSize)

    // RIFF header
    data.append(contentsOf: "RIFF".utf8)
    var chunkSize = UInt32(headerSize - 8 + dataSize).littleEndian
    data.append(Data(bytes: &chunkSize, count: 4))
    data.append(contentsOf: "WAVE".utf8)

    // fmt subchunk
    data.append(contentsOf: "fmt ".utf8)
    var subchunk1Size: UInt32 = 16
    subchunk1Size = subchunk1Size.littleEndian
    data.append(Data(bytes: &subchunk1Size, count: 4))
    var audioFormat: UInt16 = 3 // IEEE Float
    audioFormat = audioFormat.littleEndian
    data.append(Data(bytes: &audioFormat, count: 2))
    var numChannels = UInt16(channels).littleEndian
    data.append(Data(bytes: &numChannels, count: 2))
    var sr = UInt32(sampleRate).littleEndian
    data.append(Data(bytes: &sr, count: 4))
    var byteRate = UInt32(sampleRate * channels * bytesPerSample).littleEndian
    data.append(Data(bytes: &byteRate, count: 4))
    var blockAlign = UInt16(channels * bytesPerSample).littleEndian
    data.append(Data(bytes: &blockAlign, count: 2))
    var bitsPerSample: UInt16 = 32
    bitsPerSample = bitsPerSample.littleEndian
    data.append(Data(bytes: &bitsPerSample, count: 2))

    // data subchunk
    data.append(contentsOf: "data".utf8)
    var subchunk2Size = UInt32(dataSize).littleEndian
    data.append(Data(bytes: &subchunk2Size, count: 4))

    // PCM samples
    for sample in samples {
        var s = sample
        data.append(Data(bytes: &s, count: 4))
    }

    try data.write(to: url)
}

// Resample using linear interpolation
func resampleToRate(_ targetRate: Int, from samples: [Float], sourceRate: Int, channels: Int) -> [Float] {
    if sourceRate == targetRate { return samples }

    let frameCount = samples.count / channels
    let ratio = Double(targetRate) / Double(sourceRate)
    let outputFrames = Int(Double(frameCount) * ratio)

    // Deinterleave
    var channelData = [[Float]](repeating: [Float](repeating: 0, count: frameCount), count: channels)
    for f in 0..<frameCount {
        for ch in 0..<channels {
            channelData[ch][f] = samples[f * channels + ch]
        }
    }

    // Resample each channel using linear interpolation
    var outputChannels = [[Float]](repeating: [Float](repeating: 0, count: outputFrames), count: channels)
    for ch in 0..<channels {
        let input = channelData[ch]
        for i in 0..<outputFrames {
            let srcPos = Double(i) / ratio
            let srcIdx = Int(srcPos)
            let frac = Float(srcPos - Double(srcIdx))
            if srcIdx + 1 < frameCount {
                outputChannels[ch][i] = input[srcIdx] * (1.0 - frac) + input[srcIdx + 1] * frac
            } else if srcIdx < frameCount {
                outputChannels[ch][i] = input[srcIdx]
            }
        }
    }

    // Reinterleave
    var result = [Float](repeating: 0, count: outputFrames * channels)
    for f in 0..<outputFrames {
        for ch in 0..<channels {
            result[f * channels + ch] = outputChannels[ch][f]
        }
    }
    return result
}

// Main
guard CommandLine.arguments.count >= 3 else {
    print("Usage: swift tts_reference.swift <voice_name> <output.wav> [text]")
    print("  voice_name: e.g. 'Alex', 'Samantha (Enhanced)'")
    print("  output.wav: path for output file")
    print("  text: text to speak (default: 'The quick brown fox jumps over the lazy dog')")
    exit(1)
}

let voiceName = CommandLine.arguments[1]
let outputPath = CommandLine.arguments[2]
let text = CommandLine.arguments.count > 3
    ? CommandLine.arguments[3...].joined(separator: " ")
    : "The quick brown fox jumps over the lazy dog"

print("Voice: \(voiceName)")
print("Text: \(text)")
print("Output: \(outputPath)")

// Find the voice
let allVoices = AVSpeechSynthesisVoice.speechVoices()
guard let voice = allVoices.first(where: { $0.name == voiceName && $0.language.hasPrefix("en") }) else {
    print("ERROR: Voice '\(voiceName)' not found. Available en voices:")
    for v in allVoices where v.language.hasPrefix("en") {
        print("  \(v.name) [\(v.identifier)] lang=\(v.language)")
    }
    exit(1)
}

print("Found voice: \(voice.name) [\(voice.identifier)] lang=\(voice.language)")

// Create utterance with SAME parameters as omnivox defaults
let utterance = AVSpeechUtterance(string: text)
utterance.voice = voice
utterance.rate = 0.5          // Same as TtsSettings::default().rate
utterance.pitchMultiplier = 1.0  // Same as TtsSettings::default().pitch
utterance.volume = 1.0        // Same as TtsSettings::default().volume

// Synthesize to buffer (same API as omnivox ObjC bridge)
let synth = AVSpeechSynthesizer()
var allSamples = [Float]()
var capturedSampleRate: Int = 0
var capturedChannels: Int = 0
var synthesisComplete = false
var chunksReceived = 0

synth.write(utterance) { buffer in
    guard let pcm = buffer as? AVAudioPCMBuffer else { return }

    if pcm.frameLength == 0 {
        synthesisComplete = true
        return
    }

    capturedSampleRate = Int(pcm.format.sampleRate)
    capturedChannels = Int(pcm.format.channelCount)

    guard let floatData = pcm.floatChannelData else { return }

    for frame in 0..<Int(pcm.frameLength) {
        for ch in 0..<capturedChannels {
            allSamples.append(floatData[ch][frame])
        }
    }
    chunksReceived += 1
}

// Wait for completion (same approach as omnivox ObjC bridge)
let deadline = Date().addingTimeInterval(30.0)
var lastChunkCount = 0
var lastChunkTime = Date()

while Date() < deadline {
    RunLoop.current.run(mode: .default, before: Date(timeIntervalSinceNow: 0.01))

    if synthesisComplete { break }

    if chunksReceived > 0 {
        if chunksReceived != lastChunkCount {
            lastChunkCount = chunksReceived
            lastChunkTime = Date()
        } else if Date().timeIntervalSince(lastChunkTime) > 0.2 {
            break
        }
    }
}

guard !allSamples.isEmpty else {
    print("ERROR: No audio data captured")
    exit(1)
}

let frameCount = allSamples.count / max(capturedChannels, 1)
let duration = Double(frameCount) / Double(capturedSampleRate)
print("Captured: \(allSamples.count) samples, \(capturedSampleRate)Hz, \(capturedChannels)ch, \(String(format: "%.2f", duration))s")
print("Chunks received: \(chunksReceived)")

// Save raw (native sample rate) WAV
let rawURL = URL(fileURLWithPath: outputPath.replacingOccurrences(of: ".wav", with: "_raw.wav"))
try writeWAV(samples: allSamples, sampleRate: capturedSampleRate, channels: capturedChannels, to: rawURL)
print("Saved raw: \(rawURL.path) (\(capturedSampleRate)Hz)")

// Convert to stereo if mono (same as omnivox to_stereo)
var stereoSamples: [Float]
var stereoChannels: Int
if capturedChannels == 1 {
    stereoSamples = [Float]()
    stereoSamples.reserveCapacity(allSamples.count * 2)
    for s in allSamples {
        stereoSamples.append(s)
        stereoSamples.append(s)
    }
    stereoChannels = 2
    print("Converted mono to stereo")
} else {
    stereoSamples = allSamples
    stereoChannels = capturedChannels
}

// Resample to 44100Hz (for comparison with omnivox output)
let resampled = resampleToRate(44100, from: stereoSamples, sourceRate: capturedSampleRate, channels: stereoChannels)
let resampledURL = URL(fileURLWithPath: outputPath)
try writeWAV(samples: resampled, sampleRate: 44100, channels: stereoChannels, to: resampledURL)
print("Saved resampled: \(resampledURL.path) (44100Hz, \(resampled.count) samples)")

print("\nDone! Compare these files with omnivox output.")
print("The _raw file is the unprocessed AVSpeechSynthesizer output.")
print("The main file is resampled to 44100Hz (linear interpolation).")
