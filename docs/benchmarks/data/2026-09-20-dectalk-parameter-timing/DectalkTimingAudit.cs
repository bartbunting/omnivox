// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Diagnostic only: wrap actual native delegates in the supplied helper assembly.
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;

public static class DectalkTimingAudit
{
    private const BindingFlags Hidden = BindingFlags.Public | BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static;
    private static Delegate SpeakOriginal, SyncOriginal, ResetOriginal, ReadOriginal;
    private static List<object> Calls;
    private static string Phase;
    private static bool SpeechSubmitted;
    private static long Entry;
    private static Dictionary<string, object> Row(params object[] args)
    {
        var row = new Dictionary<string, object>();
        for (int i = 0; i < args.Length; i += 2) row.Add((string)args[i], args[i + 1]);
        return row;
    }
    private static double Milliseconds(long ticks) { return ticks * 1000.0 / Stopwatch.Frequency; }
    private static void Record(string api, long start)
    {
        long end = Stopwatch.GetTimestamp();
        Calls.Add(Row("phase", Phase, "api", api, "start_ms", Milliseconds(start - Entry), "duration_ms", Milliseconds(end - start)));
    }
    private static uint Speak(IntPtr handle, IntPtr text, uint flags)
    {
        string value = Marshal.PtrToStringAnsi(text);
        if (value.IndexOf("Timing", StringComparison.Ordinal) >= 0) { Phase = "speech"; SpeechSubmitted = true; }
        else if (value == "[:np]") Phase = SpeechSubmitted ? "restore" : "preset";
        else if (value.StartsWith("[:np ", StringComparison.Ordinal)) Phase = "common";
        else Phase = "edits";
        long start = Stopwatch.GetTimestamp();
        uint result = (uint)SpeakOriginal.DynamicInvoke(handle, text, flags);
        Record("speak", start); return result;
    }
    private static uint Sync(IntPtr handle)
    {
        long start = Stopwatch.GetTimestamp();
        uint result = (uint)SyncOriginal.DynamicInvoke(handle);
        Record("sync", start); return result;
    }
    private static uint Reset(IntPtr handle, bool modes)
    {
        Phase = "cleanup";
        long start = Stopwatch.GetTimestamp();
        uint result = (uint)ResetOriginal.DynamicInvoke(handle, modes);
        Record("reset", start); return result;
    }
    private static uint Read(IntPtr handle, uint index, out IntPtr current, out IntPtr low, out IntPtr high, out IntPtr defaults)
    {
        object[] args = { handle, index, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero };
        long start = Stopwatch.GetTimestamp();
        uint result = (uint)ReadOriginal.DynamicInvoke(args);
        Record("readback", start);
        current = (IntPtr)args[2]; low = (IntPtr)args[3]; high = (IntPtr)args[4]; defaults = (IntPtr)args[5];
        return result;
    }
    private static object Field(object obj, string name) { return obj.GetType().GetField(name, Hidden).GetValue(obj); }
    private static Delegate Hook(object native, string field, string method)
    {
        FieldInfo info = native.GetType().GetField(field, Hidden);
        Delegate original = (Delegate)info.GetValue(native);
        info.SetValue(native, Delegate.CreateDelegate(info.FieldType, typeof(DectalkTimingAudit).GetMethod(method, Hidden)));
        return original;
    }
    private static object Invoke(object target, string method, params object[] args)
    {
        try { return target.GetType().GetMethod(method, Hidden).Invoke(target, args); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }
    public static object Run(string helper, string dll, int blocks, int iterations)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type adapterType = assembly.GetType("OmnivoxDectalkAdapter", true);
        Type editsType = assembly.GetType("OmnivoxDectalkParameters", true);
        Array anchors = Array.CreateInstance(assembly.GetType("OmnivoxHelperAnchor", true), 0);
        int[] minimum = (int[])editsType.GetField("Minimum", Hidden).GetValue(null);
        int[] maximum = (int[])editsType.GetField("Maximum", Hidden).GetValue(null);
        string[] ids = (string[])editsType.GetField("Ids", Hidden).GetValue(null);
        var samples = new List<object>();
        var order = new List<object>();
        for (int block = 0; block < blocks; block++)
        {
            string[] cases = { "ordinary", "native_empty", "native_one", "native_all" };
            Random random = new Random(71423 + block);
            for (int i = cases.Length - 1; i > 0; i--) { int j = random.Next(i + 1); string s = cases[i]; cases[i] = cases[j]; cases[j] = s; }
            foreach (string scenario in cases)
            {
                order.Add(Row("block", block, "case", scenario));
                object adapter = Activator.CreateInstance(adapterType, Hidden, null, new object[] { dll }, null);
                object native = Field(Field(adapter, "capture"), "native");
                SpeakOriginal = Hook(native, "speak", "Speak"); SyncOriginal = Hook(native, "sync", "Sync");
                ResetOriginal = Hook(native, "reset", "Reset"); ReadOriginal = Hook(native, "getSpeakerParams", "Read");
                try
                {
                    var edits = new Dictionary<string, int?>();
                    if (scenario == "native_one") edits.Add("sm", 61);
                    if (scenario == "native_all")
                        for (int i = 0; i < ids.Length; i++) edits.Add(ids[i], (minimum[i] + maximum[i]) / 2);
                    for (int iteration = -3; iteration < iterations; iteration++)
                    {
                        Calls = new List<object>(); Phase = "entry"; SpeechSubmitted = false;
                        double appliedAt = -1; int receipts = 0;
                        Entry = Stopwatch.GetTimestamp();
                        object result;
                        if (scenario == "ordinary")
                            result = Invoke(adapter, "Synthesize", "Timing a short line.", "paul", 0.4, 1.0, null, null, null, 1.0,
                                anchors, new Func<bool>(() => false), null);
                        else
                            result = Invoke(adapter, "SynthesizeWithParameters", "Timing a short line.", "paul", 0.4, 1.0, null, null, null, 1.0,
                                anchors, new Func<bool>(() => false), null, edits, new string[0], new Action<int[]>(values => {
                                    appliedAt = Milliseconds(Stopwatch.GetTimestamp() - Entry); receipts++;
                                    if (values.Length != ids.Length) throw new Exception("Incomplete native readback");
                                    for (int i = 0; i < ids.Length; i++)
                                        if (edits.ContainsKey(ids[i]) && values[i] != edits[ids[i]].Value) throw new Exception("Readback mismatch: " + ids[i]);
                                }));
                        double total = Milliseconds(Stopwatch.GetTimestamp() - Entry);
                        byte[] pcm = (byte[])Field(result, "Audio");
                        if (pcm.Length == 0 || pcm.Length % 2 != 0 || receipts != (scenario == "ordinary" ? 0 : 1))
                            throw new Exception("Missing PCM or invalid application receipt");
                        if (iteration >= 0) samples.Add(Row("block", block, "case", scenario, "iteration", iteration,
                            "total_ms", total, "applied_ms", appliedAt, "frames", pcm.Length / 2, "calls", Calls));
                    }
                }
                finally
                {
                    native.GetType().GetField("speak", Hidden).SetValue(native, SpeakOriginal);
                    native.GetType().GetField("sync", Hidden).SetValue(native, SyncOriginal);
                    native.GetType().GetField("reset", Hidden).SetValue(native, ResetOriginal);
                    native.GetType().GetField("getSpeakerParams", Hidden).SetValue(native, ReadOriginal);
                    ((IDisposable)adapter).Dispose();
                }
            }
        }
        return Row("measurement", "native-call timings through reflection delegate hooks; buffered silent capture; not acoustic or server startup",
            "clock", "Stopwatch.GetTimestamp", "warmups_per_case", 3, "settings", Row("voice", "paul", "rate", 0.4, "pitch", 1.0),
            "order", order, "samples", samples);
    }
}
