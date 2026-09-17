// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Test-only installed-runtime audit. Never opens an audio output device.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;

public static class NativeVoiceParametersAudit
{
    private const BindingFlags Hidden = BindingFlags.Public | BindingFlags.NonPublic |
        BindingFlags.Instance | BindingFlags.Static;
    [DllImport("kernel32", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandle(string path);
    [DllImport("kernel32", CharSet = CharSet.Ansi, ExactSpelling = true)]
    private static extern IntPtr GetProcAddress(IntPtr module, string name);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int EciGet(IntPtr handle, int voice, int parameter);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int EciSet(IntPtr handle, int voice, int parameter, int value);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private delegate bool EciCopy(IntPtr handle, int source, int destination);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint DtGet(IntPtr handle, uint index, out IntPtr current,
        out IntPtr low, out IntPtr high, out IntPtr defaults);

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
    private static readonly int[] DtOffsets = {
        0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13,
        16, 17, 18, 19, 20, 21, 22, 23, 24, 26, 27, 28, 29, 30
    };

    private static T Bind<T>(IntPtr module, string name) where T : class
    {
        IntPtr address = GetProcAddress(module, name);
        if (address == IntPtr.Zero) throw new Exception("Missing audit export: " + name);
        return (T)(object)Marshal.GetDelegateForFunctionPointer(address, typeof(T));
    }

    private static Dictionary<string, object> Record(params object[] fields)
    {
        var result = new Dictionary<string, object>();
        for (int i = 0; i < fields.Length; i += 2)
            result.Add((string)fields[i], fields[i + 1]);
        return result;
    }

    private static int[][] ReadDt(DtGet get, IntPtr handle)
    {
        IntPtr current, low, high, defaults;
        uint status = get(handle, 0, out current, out low, out high, out defaults);
        if (status != 0) throw new Exception("DECtalk query failed: " + status);
        IntPtr[] pointers = { current, low, high, defaults };
        try
        {
            int[][] result = new int[4][];
            for (int p = 0; p < pointers.Length; p++)
            {
                if (pointers[p] == IntPtr.Zero) throw new Exception("Null DECtalk result");
                result[p] = new int[DtOffsets.Length];
                for (int i = 0; i < DtOffsets.Length; i++)
                    result[p][i] = Marshal.ReadInt16(pointers[p], DtOffsets[i] * 2);
            }
            return result;
        }
        finally { foreach (IntPtr pointer in pointers) Marshal.FreeCoTaskMem(pointer); }
    }

    private static int[] ReadEci(EciGet get, IntPtr handle)
    {
        int[] values = new int[8];
        for (int i = 0; i < values.Length; i++)
        {
            values[i] = get(handle, 0, i);
            if (values[i] < 0) throw new Exception("ECI query failed: " + i);
        }
        return values;
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
        Assembly assembly = Assembly.LoadFrom(helper);
        string prefix = eci ? "OmnivoxEloquence" : "OmnivoxDectalk";
        Type type = assembly.GetType(prefix + "Capture", true);
        Type adapter = assembly.GetType(prefix + "Adapter", true);
        object capture = Activator.CreateInstance(type, Hidden, null,
            new object[] { dll }, null);
        try
        {
            IntPtr handle = (IntPtr)type.GetField("handle", Hidden).GetValue(capture);
            IntPtr module = GetModuleHandle(dll);
            if (module == IntPtr.Zero) throw new Exception("Loaded runtime module not found");
            EciGet eg = eci ? Bind<EciGet>(module, "eciGetVoiceParam") : null;
            EciSet es = eci ? Bind<EciSet>(module, "eciSetVoiceParam") : null;
            EciCopy ec = eci ? Bind<EciCopy>(module, "eciCopyVoice") : null;
            DtGet dg = eci ? null : Bind<DtGet>(module, "TextToSpeechGetSpeakerParams");
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
                    if (!ec(handle, Int32.Parse(voice.Substring(1)), 0))
                        throw new Exception("ECI pristine copy failed");
                    pristine = ReadEci(eg, handle);
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
                int[][] dt = eci ? null : ReadDt(dg, handle);
                int[] baseline = eci ? ReadEci(eg, handle) : dt[0];
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
                        int previous = es(handle, 0, i, target);
                        int[] changed = ReadEci(eg, handle);
                        bool copied = ec(handle, Int32.Parse(voice.Substring(1)), 0);
                        int[] restored = ReadEci(eg, handle);
                        apiPassed = previous == baseline[i] && changed[i] == target &&
                            copied && Differences(pristine, restored, ids).Length == 0;
                        api = Record("previous", previous, "observed", changed[i],
                            "copy_succeeded", copied, "copy_restored_pristine",
                            Differences(pristine, restored, ids).Length == 0);
                    }
                    string command = eci ? " `" + EciCommands[i] + target : " " + ids[i] + " " + target;
                    int bytes = speak(command, pitch, rate, eci && i == 7 ? target : volume);
                    int[] actual = eci ? ReadEci(eg, handle) : ReadDt(dg, handle)[0];
                    speak("", pitch, rate, volume);
                    int[] reset = eci ? ReadEci(eg, handle) : ReadDt(dg, handle)[0];
                    if (actual[i] != target || Differences(baseline, reset, ids).Length != 0 || !apiPassed)
                        failedCases++;
                    parameters.Add(Record("id", ids[i], "status", "probed", "baseline", baseline[i],
                        "default", eci ? pristine[i] : dt[3][i], "low", low, "high", high,
                        "target", target, "observed", actual[i], "setter_matches", actual[i] == target,
                        "changed_ids", Differences(baseline, actual, ids),
                        "selection_reset_matches", Differences(baseline, reset, ids).Length == 0,
                        "reset_changed_ids", Differences(baseline, reset, ids), "pcm_bytes", bytes,
                        "voice_api", api));
                }
                rows.Add(Record("voice_id", voice, "parameters", parameters));
            }
            return Record("engine_id", eci ? "eloquence" : "dectalk", "runtime_version",
                type.GetProperty("Version", Hidden).GetValue(capture, null), "captures", captures,
                "all_candidates_passed", failedCases == 0, "failed_cases", failedCases,
                "range_evidence", eci ? "documented ECI units; boundaries not tested" :
                    "runtime reported limits; boundaries not tested",
                "scope", "single-parameter interior values; no cancellation or boundary qualification",
                "voices", rows);
        }
        finally { ((IDisposable)capture).Dispose(); }
    }
}
