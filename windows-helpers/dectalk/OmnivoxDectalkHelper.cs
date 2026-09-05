// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.IO;

internal sealed class OmnivoxDectalkAdapter : IOmnivoxCaptureEngine
{
    private static readonly OmnivoxHelperVoice[] EngineVoices =
        new OmnivoxHelperVoice[]
        {
            new OmnivoxHelperVoice("paul", "Perfect Paul", "en-US", "male"),
            new OmnivoxHelperVoice("betty", "Beautiful Betty", "en-US", "female"),
            new OmnivoxHelperVoice("harry", "Huge Harry", "en-US", "male"),
            new OmnivoxHelperVoice("frank", "Frail Frank", "en-US", "male"),
            new OmnivoxHelperVoice("kit", "Kit the Kid", "en-US", null),
            new OmnivoxHelperVoice("rita", "Rough Rita", "en-US", "female"),
            new OmnivoxHelperVoice("ursula", "Uppity Ursula", "en-US", "female"),
            new OmnivoxHelperVoice("dennis", "Doctor Dennis", "en-US", "male"),
            new OmnivoxHelperVoice("wendy", "Whispering Wendy", "en-US", "female")
        };

    private static readonly Dictionary<string, string> VoiceCodes =
        new Dictionary<string, string>(StringComparer.Ordinal)
        {
            { "paul", ":np" },
            { "betty", ":nb" },
            { "harry", ":nh" },
            { "frank", ":nf" },
            { "kit", ":nk" },
            { "rita", ":nr" },
            { "ursula", ":nu" },
            { "dennis", ":nd" },
            { "wendy", ":nw" }
        };

    private static readonly Dictionary<string, int> VoiceAveragePitch =
        new Dictionary<string, int>(StringComparer.Ordinal)
        {
            { "paul", 122 },
            { "betty", 208 },
            { "harry", 89 },
            { "frank", 155 },
            { "kit", 306 },
            { "rita", 106 },
            { "ursula", 240 },
            { "dennis", 110 },
            { "wendy", 200 }
        };

    private static readonly int[] PitchRange =
        { 0, 20, 40, 60, 80, 100, 137, 174, 211, 250 };
    private static readonly int[] Assertiveness =
        { 0, 10, 20, 30, 40, 50, 60, 70, 80, 100 };
    private static readonly int[] HatRise =
        { 0, 3, 6, 9, 12, 18, 34, 48, 63, 80 };
    private static readonly int[] StressRise =
        { 0, 6, 12, 18, 24, 32, 50, 65, 82, 90 };
    private static readonly int[] Quickness =
        { 0, 20, 40, 60, 80, 100, 100, 100, 100, 100 };
    private static readonly int[] BaselineFall =
        { 0, 3, 6, 9, 14, 18, 20, 35, 60, 40 };
    private static readonly int[] Richness =
        { 0, 14, 28, 42, 56, 70, 60, 70, 80, 100 };
    private static readonly int[] Smoothness =
        { 100, 80, 60, 40, 20, 3, 24, 16, 8, 0 };
    private static readonly double[] RatePoints =
        { 0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0, 1.2 };
    private static readonly double[] NativeRatePoints =
        {
            75.0000, 75.0000, 114.0177, 161.4411,
            222.5271, 288.5925, 368.4105, 426.14775,
            477.4305, 509.9715, 544.3800, 600.0000
        };

    private static readonly OmnivoxHelperCapabilities EngineCapabilities =
        new OmnivoxHelperCapabilities
        {
            Rate = true,
            AveragePitch = true,
            PitchRange = true,
            Stress = true,
            Richness = true,
            Volume = true,
            WordMarkers = true,
            SentenceMarkers = true,
            PhonemeMarkers = true,
            NativeIndexMarkers = true,
            RequestedAnchors = "word_boundary",
            TextRepertoire = "iso_8859_1"
        };

    private readonly OmnivoxDectalkCapture capture;

    internal OmnivoxDectalkAdapter(string dllPath)
    {
        capture = new OmnivoxDectalkCapture(dllPath);
    }

    public string EngineId { get { return "dectalk"; } }
    public string DisplayName { get { return "DECtalk Software"; } }
    public string Version { get { return capture.Version; } }
    public string HelperName { get { return "Omnivox DECtalk x86 helper"; } }
    public string DefaultVoiceId { get { return "paul"; } }
    public int SampleRate
    {
        get { return OmnivoxDectalkCapture.SpeechSampleRate; }
    }
    public int Channels { get { return 1; } }
    public bool SupportsProgressiveSynthesis { get { return true; } }
    public OmnivoxHelperVoice[] Voices { get { return EngineVoices; } }
    public OmnivoxHelperCapabilities Capabilities
    {
        get { return EngineCapabilities; }
    }

    public OmnivoxCaptureResult Synthesize(string text, string voiceId,
        double rate, double pitch, double? pitchRange, double? stress,
        double? richness, double volume,
        OmnivoxHelperAnchor[] anchors, Func<bool> cancellationRequested,
        IOmnivoxCaptureSink sink)
    {
        string voiceCode;
        if (!VoiceCodes.TryGetValue(voiceId, out voiceCode))
        {
            throw new ArgumentException("Unknown DECtalk voice", "voiceId");
        }

        int nativeRate = MapRate(rate);
        int nativePitch = (int)Math.Round(
            VoiceAveragePitch[voiceId] * pitch,
            MidpointRounding.AwayFromZero);
        nativePitch = Math.Max(50, Math.Min(500, nativePitch));
        string voiceParameters = MapExtendedAcss(pitchRange, stress, richness);
        return capture.Synthesize(text, voiceCode, nativeRate, nativePitch,
            voiceParameters, volume, anchors, cancellationRequested, sink);
    }

    internal static int MapRate(double rate)
    {
        // Measured reference and saturation policy: docs/RATE-CALIBRATION.md.
        if (Double.IsNaN(rate) || Double.IsInfinity(rate))
        {
            rate = 0.5;
        }
        double mapped = NativeRatePoints[NativeRatePoints.Length - 1];
        if (rate <= RatePoints[0])
        {
            mapped = NativeRatePoints[0];
        }
        else
        {
            for (int index = 1; index < RatePoints.Length; index++)
            {
                if (rate <= RatePoints[index])
                {
                    double position = (rate - RatePoints[index - 1]) /
                        (RatePoints[index] - RatePoints[index - 1]);
                    mapped = NativeRatePoints[index - 1] +
                        (NativeRatePoints[index] -
                            NativeRatePoints[index - 1]) * position;
                    break;
                }
            }
        }
        return (int)Math.Round(mapped, MidpointRounding.AwayFromZero);
    }

    internal static string MapExtendedAcss(double? pitchRange,
        double? stress, double? richness)
    {
        string parameters = "";
        if (pitchRange.HasValue)
        {
            parameters += " pr " + MapNormalized(pitchRange.Value, PitchRange) +
                " as " + MapNormalized(pitchRange.Value, Assertiveness);
        }
        if (stress.HasValue)
        {
            parameters += " hr " + MapNormalized(stress.Value, HatRise) +
                " sr " + MapNormalized(stress.Value, StressRise) +
                " qu " + MapNormalized(stress.Value, Quickness) +
                " bf " + MapNormalized(stress.Value, BaselineFall);
        }
        if (richness.HasValue)
        {
            parameters += " ri " + MapNormalized(richness.Value, Richness) +
                " sm " + MapNormalized(richness.Value, Smoothness);
        }
        return parameters;
    }

    private static int MapNormalized(double value, int[] levels)
    {
        double position = Math.Max(0.0, Math.Min(1.0, value)) *
            (levels.Length - 1);
        int lower = (int)Math.Floor(position);
        int upper = Math.Min(lower + 1, levels.Length - 1);
        double mapped = levels[lower] +
            (levels[upper] - levels[lower]) * (position - lower);
        return (int)Math.Round(mapped, MidpointRounding.AwayFromZero);
    }

    public void Stop()
    {
        capture.Stop();
    }

    public void Dispose()
    {
        capture.Dispose();
    }
}

internal static class OmnivoxDectalkHelper
{
    private static string ResolveDllPath(string[] args)
    {
        string dllPath = args.Length > 0 ? args[0] :
            Environment.GetEnvironmentVariable("OMNIVOX_DECTALK_DLL");
        if (String.IsNullOrEmpty(dllPath))
        {
            dllPath = Environment.GetEnvironmentVariable(
                "EMACSVOX_DECTALK_DLL");
        }
        if (!String.IsNullOrEmpty(dllPath))
        {
            return dllPath;
        }

        string adjacent = Path.Combine(
            AppDomain.CurrentDomain.BaseDirectory, "DECtalk.dll");
        if (File.Exists(adjacent))
        {
            return adjacent;
        }
        string sibling = Path.GetFullPath(Path.Combine(
            AppDomain.CurrentDomain.BaseDirectory, "..", "runtime",
            "DECtalk.dll"));
        if (File.Exists(sibling))
        {
            return sibling;
        }

        string localData = Environment.GetFolderPath(
            Environment.SpecialFolder.LocalApplicationData);
        if (String.IsNullOrEmpty(localData) || !Path.IsPathRooted(localData))
        {
            throw new OmnivoxRuntimeUnavailableException(
                "The Windows local application data directory is unavailable; " +
                "set OMNIVOX_DECTALK_DLL to an absolute DECtalk.dll path " +
                "with its matching dtalk_us.dic in the same directory.");
        }
        string standardDirectory = Path.Combine(
            localData, "Omnivox", "runtimes", "dectalk", "x86");
        dllPath = Path.Combine(standardDirectory, "DECtalk.dll");
        if (!File.Exists(dllPath))
        {
            throw new OmnivoxRuntimeUnavailableException(
                "DECtalk.dll was not found beside the helper, in its sibling " +
                "runtime directory, or at \"" + dllPath + "\". Install " +
                "matching IA32 DECtalk.dll and dtalk_us.dic files in \"" +
                standardDirectory + "\", or set OMNIVOX_DECTALK_DLL.");
        }
        return dllPath;
    }

    internal static int Main(string[] args)
    {
        return OmnivoxHelperRuntime.Run("dectalk", "DECtalk Software",
            "Omnivox DECtalk x86 helper", delegate()
            {
                return new OmnivoxDectalkAdapter(ResolveDllPath(args));
            });
    }
}
