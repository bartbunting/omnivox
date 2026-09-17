// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Test-only installed-runtime audit. Never opens an audio output device.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Reflection;

public static class NativeVoiceParametersAudit
{
    private const BindingFlags Hidden = BindingFlags.Public | BindingFlags.NonPublic |
        BindingFlags.Instance | BindingFlags.Static;
    private static readonly string[] EciIds = {
        "gender", "head_size", "pitch_baseline", "pitch_fluctuation",
        "roughness", "breathiness", "speed", "volume"
    };
    private static readonly string[] EciCommands = {
        "vg", "vh", "vb", "vf", "vr", "vy", "vs", "vv"
    };
    // Only initialized, documented SPDEFS fields: no padding or reserved fields.
    private static readonly string[] DtIds = {
        "sx", "sm", "as", "ap", "pr", "br", "ri", "nf", "la", "hs",
        "f4", "b4", "f5", "b5", "gf", "gh", "gv", "gn", "g1", "g2",
        "g3", "g4", "g5", "bf", "lx", "qu", "hr", "sr"
    };
    private static Dictionary<string, object> Record(params object[] fields)
    {
        var result = new Dictionary<string, object>();
        for (int i = 0; i < fields.Length; i += 2)
            result.Add((string)fields[i], fields[i + 1]);
        return result;
    }

    private static object Invoke(object native, string method, params object[] args)
    {
        try { return native.GetType().GetMethod(method, Hidden).Invoke(native, args); }
        catch (TargetInvocationException e) { throw e.InnerException; }
    }

    private static int[][] ReadDt(object native, IntPtr handle)
    {
        return (int[][])Invoke(native, "ReadSpeakerParameterFields", handle);
    }

    private static int[] ReadEci(object native, IntPtr handle)
    {
        return (int[])Invoke(native, "ReadActiveVoiceParameters", handle);
    }

    private static bool Rejects(Action action, Type expected)
    {
        try { action(); return false; }
        catch (Exception e) { return e.GetType() == expected; }
    }

    private static string[] Differences(int[] before, int[] after, string[] ids)
    {
        var changed = new List<string>();
        for (int i = 0; i < ids.Length; i++)
            if (before[i] != after[i]) changed.Add(ids[i]);
        return changed.ToArray();
    }

    public static object Run(string helper, string dll, bool eci)
    {
        helper = Path.GetFullPath(helper);
        dll = Path.GetFullPath(dll);
        // Load the exact selected helper bytes for reflection-only capture tests.
        // LoadFrom rejects a WSL UNC assembly as remote; no dependencies or
        // entry point are selected from that path by this audit.
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        string prefix = eci ? "OmnivoxEloquence" : "OmnivoxDectalk";
        Type type = assembly.GetType(prefix + "Capture", true);
        Type adapter = assembly.GetType(prefix + "Adapter", true);
        object capture = Activator.CreateInstance(type, Hidden, null,
            new object[] { dll }, null);
        try
        {
            IntPtr handle = (IntPtr)type.GetField("handle", Hidden).GetValue(capture);
            object native = type.GetField("native", Hidden).GetValue(capture);
            bool bindings = (bool)native.GetType().GetProperty(
                eci ? "HasVoiceParameterApi" : "HasSpeakerParameterApi", Hidden).GetValue(native, null);
            if (!bindings) throw new Exception("Optional parameter bindings are unavailable");
            MethodInfo synth = type.GetMethod("Synthesize", Hidden);
            Array anchors = Array.CreateInstance(assembly.GetType("OmnivoxHelperAnchor", true), 0);
            IDictionary voices = (IDictionary)adapter.GetField(
                eci ? "VoicePitchBaselines" : "VoiceAveragePitch", Hidden).GetValue(null);
            IDictionary codes = eci ? null : (IDictionary)adapter.GetField("VoiceCodes", Hidden).GetValue(null);
            var names = new List<string>();
            foreach (string voice in voices.Keys) names.Add(voice);
            names.Sort(StringComparer.Ordinal);
            var rows = new List<object>();
            int captures = 0;
            int failedCases = 0;
            foreach (string voice in names)
            {
                int pitch = (int)voices[voice], rate = eci ? 75 : 180, volume = 100;
                int[] pristine = null;
                if (eci)
                {
                    Invoke(native, "CopyPresetToActive", handle, Int32.Parse(voice.Substring(1)));
                    pristine = ReadEci(native, handle);
                    pitch = pristine[2]; rate = pristine[6]; volume = pristine[7];
                }
                Func<string, int, int, int, int> speak = (commands, p, r, v) => {
                    object result = synth.Invoke(capture, new object[] {
                        "The quick brown fox jumps over the lazy dog.",
                        eci ? voice : (string)codes[voice], r, p, commands,
                        eci ? (object)v : (object)1.0, anchors, new Func<bool>(() => false), null
                    });
                    byte[] pcm = (byte[])result.GetType().GetField("Audio", Hidden).GetValue(result);
                    if (pcm.Length == 0) throw new Exception("Empty native PCM");
                    captures++;
                    return pcm.Length;
                };
                speak("", pitch, rate, volume);
                int[][] dt = eci ? null : ReadDt(native, handle);
                int[] baseline = eci ? ReadEci(native, handle) : dt[0];
                string[] ids = eci ? EciIds : DtIds;
                var parameters = new List<object>();
                for (int i = 0; i < ids.Length; i++)
                {
                    int low = eci ? 0 : dt[1][i];
                    int high = eci ? (i == 0 ? 1 : i == 6 ? 250 : 100) : dt[2][i];
                    if (low > high || baseline[i] < low || baseline[i] > high)
                    {
                        failedCases++;
                        parameters.Add(Record("id", ids[i], "status", "invalid_reported_limits",
                            "baseline", baseline[i], "low", low, "high", high));
                        continue;
                    }
                    if (low == high)
                    {
                        failedCases++;
                        parameters.Add(Record("id", ids[i], "status", "fixed_range",
                            "baseline", baseline[i], "low", low, "high", high));
                        continue;
                    }
                    int delta = Math.Max(1, (high - low) / 10);
                    int target = baseline[i] + delta <= high ? baseline[i] + delta : baseline[i] - delta;
                    target = Math.Max(low, target);
                    object api = null;
                    bool apiPassed = true;
                    if (eci)
                    {
                        int previous = (int)Invoke(native, "SetActiveVoiceParameter", handle, i, target);
                        int[] changed = ReadEci(native, handle);
                        Invoke(native, "CopyPresetToActive", handle, Int32.Parse(voice.Substring(1)));
                        bool copied = true;
                        int[] restored = ReadEci(native, handle);
                        apiPassed = previous == baseline[i] && changed[i] == target &&
                            copied && Differences(pristine, restored, ids).Length == 0;
                        api = Record("previous", previous, "observed", changed[i],
                            "copy_succeeded", copied, "copy_restored_pristine",
                            Differences(pristine, restored, ids).Length == 0);
                    }
                    string command = eci ? " `" + EciCommands[i] + target : " " + ids[i] + " " + target;
                    int bytes = speak(command, pitch, rate, eci && i == 7 ? target : volume);
                    int[] actual = eci ? ReadEci(native, handle) : ReadDt(native, handle)[0];
                    speak("", pitch, rate, volume);
                    int[] reset = eci ? ReadEci(native, handle) : ReadDt(native, handle)[0];
                    if (actual[i] != target || Differences(baseline, reset, ids).Length != 0 || !apiPassed)
                        failedCases++;
                    var boundaries = new List<object>();
                    foreach (int boundary in new int[] { low, high })
                    {
                        bool boundaryApi = true;
                        if (eci)
                        {
                            Invoke(native, "SetActiveVoiceParameter", handle, i, boundary);
                            boundaryApi = ReadEci(native, handle)[i] == boundary;
                            Invoke(native, "CopyPresetToActive", handle, Int32.Parse(voice.Substring(1)));
                            boundaryApi &= Differences(pristine, ReadEci(native, handle), ids).Length == 0;
                        }
                        string boundaryCommand = eci ? " `" + EciCommands[i] + boundary :
                            " " + ids[i] + " " + boundary;
                        int boundaryBytes = speak(boundaryCommand, pitch, rate,
                            eci && i == 7 ? boundary : volume);
                        int[] observed = eci ? ReadEci(native, handle) : ReadDt(native, handle)[0];
                        speak("", pitch, rate, volume);
                        int[] restored = eci ? ReadEci(native, handle) : ReadDt(native, handle)[0];
                        bool passed = boundaryApi && observed[i] == boundary &&
                            Differences(baseline, restored, ids).Length == 0;
                        if (!passed) failedCases++;
                        boundaries.Add(Record("target", boundary, "observed", observed[i],
                            "passed", passed, "voice_api_passed", eci ? (object)boundaryApi : null,
                            "changed_ids", Differences(baseline, observed, ids),
                            "reset_changed_ids", Differences(baseline, restored, ids),
                            "pcm_bytes", boundaryBytes));
                    }
                    bool invalidRejected = true;
                    if (eci)
                    {
                        foreach (int invalid in new int[] { low - 1, high + 1 })
                            invalidRejected &= Rejects(() => Invoke(native,
                                "SetActiveVoiceParameter", handle, i, invalid),
                                typeof(ArgumentOutOfRangeException));
                        invalidRejected &= Differences(baseline, ReadEci(native, handle), ids).Length == 0;
                        if (!invalidRejected) failedCases++;
                    }
                    parameters.Add(Record("id", ids[i], "status", "probed", "baseline", baseline[i],
                        "default", eci ? pristine[i] : dt[3][i], "low", low, "high", high,
                        "target", target, "observed", actual[i], "setter_matches", actual[i] == target,
                        "changed_ids", Differences(baseline, actual, ids),
                        "selection_reset_matches", Differences(baseline, reset, ids).Length == 0,
                        "reset_changed_ids", Differences(baseline, reset, ids), "pcm_bytes", bytes,
                        "voice_api", api, "boundaries", boundaries,
                        "invalid_api_values_rejected", eci ? (object)invalidRejected : null));
                }
                var absentChecks = new List<object>();
                foreach (string fieldName in eci ?
                    new string[] { "getParam", "getVoiceParam", "setVoiceParam", "copyVoice" } :
                    new string[] { "getSpeakerParams" })
                {
                    FieldInfo field = native.GetType().GetField(fieldName, Hidden);
                    object bound = field.GetValue(native);
                    object library = native.GetType().GetField("library", Hidden).GetValue(native);
                    object absent = library.GetType().GetMethod("ResolveOptional", Hidden)
                        .MakeGenericMethod(bound.GetType()).Invoke(library,
                            new object[] { "OmnivoxAuditMissingOptionalExport" });
                    bool lookupMissing = absent == null;
                    bool unavailable, rejected;
                    int ordinaryBytes;
                    try
                    {
                        field.SetValue(native, null);
                        unavailable = !(bool)native.GetType().GetProperty(
                            eci ? "HasVoiceParameterApi" : "HasSpeakerParameterApi", Hidden).GetValue(native, null);
                        rejected = Rejects(() => { if (eci) ReadEci(native, handle); else ReadDt(native, handle); },
                            typeof(NotSupportedException));
                        if (eci)
                            rejected &= Rejects(() => Invoke(native, "SetActiveVoiceParameter", handle, 2, 50),
                                typeof(NotSupportedException)) &&
                                Rejects(() => Invoke(native, "CopyPresetToActive", handle, 1), typeof(NotSupportedException));
                        ordinaryBytes = speak("", pitch, rate, volume);
                    }
                    finally { field.SetValue(native, bound); }
                    if (!lookupMissing || !unavailable || !rejected) failedCases++;
                    absentChecks.Add(Record("binding", fieldName, "missing_lookup_returns_null", lookupMissing,
                        "reported_unavailable", unavailable,
                        "native_operations_rejected", rejected, "ordinary_pcm_bytes", ordinaryBytes));
                }
                bool? invalidIndexesRejected = null;
                if (eci)
                {
                    invalidIndexesRejected =
                        Rejects(() => Invoke(native, "SetActiveVoiceParameter", handle, -1, 0), typeof(ArgumentOutOfRangeException)) &&
                        Rejects(() => Invoke(native, "SetActiveVoiceParameter", handle, 8, 0), typeof(ArgumentOutOfRangeException)) &&
                        Rejects(() => Invoke(native, "CopyPresetToActive", handle, 0), typeof(ArgumentOutOfRangeException)) &&
                        Rejects(() => Invoke(native, "CopyPresetToActive", handle, 9), typeof(ArgumentOutOfRangeException));
                    if (invalidIndexesRejected != true) failedCases++;
                }
                bool? unitsRejected = null;
                if (eci)
                {
                    try
                    {
                        Invoke(native, "SetParam", handle, 8, 1);
                        unitsRejected = Rejects(() => ReadEci(native, handle), typeof(NotSupportedException)) &&
                            Rejects(() => Invoke(native, "SetActiveVoiceParameter", handle, 2, 50), typeof(NotSupportedException));
                    }
                    finally { Invoke(native, "SetParam", handle, 8, 0); }
                    if (unitsRejected != true) failedCases++;
                    Invoke(native, "CopyPresetToActive", handle, Int32.Parse(voice.Substring(1)));
                }
                rows.Add(Record("voice_id", voice, "parameters", parameters,
                    "missing_binding_checks", absentChecks, "non_eci_units_rejected", unitsRejected,
                    "invalid_api_indexes_rejected", invalidIndexesRejected));
            }
            return Record("engine_id", eci ? "eloquence" : "dectalk", "runtime_version",
                type.GetProperty("Version", Hidden).GetValue(capture, null), "captures", captures,
                "all_candidates_passed", failedCases == 0, "failed_cases", failedCases,
                "range_evidence", eci ? "documented ECI units; both boundaries tested" :
                    "runtime reported limits; both boundaries tested",
                "scope", "helper bindings; interior and boundary values; ordinary reset; missing-binding and ECI-unit guards; no cancellation qualification",
                "voices", rows);
        }
        finally { ((IDisposable)capture).Dispose(); }
    }
}
