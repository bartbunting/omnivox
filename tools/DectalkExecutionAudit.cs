// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Probe actual helper bytes in an isolated x86 STA process; never play audio.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading;
using System.Runtime.Remoting.Messaging;
using System.Runtime.Remoting.Proxies;
using System.Web.Script.Serialization;

public static class DectalkExecutionAudit
{
    private const BindingFlags Hidden = BindingFlags.Public | BindingFlags.NonPublic |
        BindingFlags.Instance | BindingFlags.Static;
    private static readonly string[] Ids = { "sx", "sm", "as", "ap", "pr", "br", "ri", "nf", "la", "hs", "f4", "b4", "f5", "b5", "gf", "gh", "gv", "gn", "g1", "g2", "g3", "g4", "g5", "bf", "lx", "qu", "hr", "sr" };
    private const string Text = "The quick brown fox jumps over the lazy dog.";

    // A local interface proxy exercises the actual helper assembly's internal
    // sink without recompiling it or introducing a native test-only entry point.
    private sealed class Sink : RealProxy
    {
        internal bool Applied;
        internal Action OnAudio;
        internal bool FailAudio;
        internal bool Cancel;
        internal bool CancelAfterAudio;
        internal ulong Frames;
        internal int Markers;
        internal Sink(Type sinkType) : base(sinkType) { }
        public override IMessage Invoke(IMessage message)
        {
            IMethodCallMessage call = (IMethodCallMessage)message;
            try
            {
                Check(Applied, "progressive output preceded native readback");
                if (call.MethodName == "Audio")
                {
                    byte[] audio = (byte[])call.Args[0];
                    int offset = (int)call.Args[1], count = (int)call.Args[2];
                    Check(count > 0 && count % 2 == 0 && offset >= 0 && offset + count <= audio.Length,
                        "invalid progressive PCM");
                    Frames += (ulong)(count / 2);
                    if (OnAudio != null) OnAudio();
                    if (CancelAfterAudio) Cancel = true;
                    if (FailAudio) throw new IOException("injected PCM delivery failure");
                }
                else if (call.MethodName == "Markers")
                {
                    foreach (object marker in (Array)call.Args[0])
                    {
                        Check((ulong)Field(marker, "FrameOffset") >= Frames, "late progressive marker");
                        Markers++;
                    }
                }
                else throw new Exception("unexpected sink call " + call.MethodName);
                return new ReturnMessage(null, null, 0, call.LogicalCallContext, call);
            }
            catch (Exception error) { return new ReturnMessage(error, call); }
        }
    }

    private static object Call(object target, string name, params object[] arguments)
    {
        Type type = target as Type ?? target.GetType();
        try { return type.GetMethod(name, Hidden).Invoke(target is Type ? null : target, arguments); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }

    private static object New(Type type, params object[] arguments)
    {
        try { return Activator.CreateInstance(type, Hidden, null, arguments, null); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }

    private static object Field(object target, string name)
    {
        return target.GetType().GetField(name, Hidden).GetValue(target);
    }

    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    private static void Equal(int[] expected, int[] actual, string message)
    {
        Check(expected.Length == actual.Length, message + " length");
        for (int i = 0; i < expected.Length; i++)
            Check(expected[i] == actual[i], message + " " + Ids[i] +
                ": expected " + expected[i] + ", got " + actual[i]);
    }

    private static void Rejects(Action action, Type expected, string message)
    {
        try { action(); }
        catch (Exception error)
        {
            Check(expected.IsInstanceOfType(error), message + ": " + error);
            return;
        }
        throw new Exception(message + ": accepted invalid request");
    }

    private static Dictionary<string, object> Record(params object[] fields)
    {
        var result = new Dictionary<string, object>();
        for (int i = 0; i < fields.Length; i += 2) result.Add((string)fields[i], fields[i + 1]);
        return result;
    }

    private static Dictionary<string, int?> Operations(int[] values)
    {
        var result = new Dictionary<string, int?>();
        for (int i = 0; i < values.Length; i++) result.Add(Ids[i], values[i]);
        return result;
    }

    private static int[] Read(object native, IntPtr handle)
    {
        return ((int[][])Call(native, "ReadSpeakerParameterFields", handle))[0];
    }

    private static int Bytes(object result)
    {
        int bytes = ((byte[])Field(result, "Audio")).Length;
        Check(bytes > 0 && bytes % 2 == 0, "native synthesis returned invalid PCM");
        return bytes;
    }

    private static List<Dictionary<string, object>> FixtureCases(string path)
    {
        var root = new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(File.ReadAllText(path));
        var result = new List<Dictionary<string, object>>();
        foreach (Dictionary<string, object> item in (IEnumerable)root["composition_cases"])
            if ((string)item["engine_id"] == "dectalk") result.Add(item);
        Check(result.Count == 7, "Expected all seven independent DECtalk composition fixtures");
        return result;
    }

    private static Dictionary<string, int?> FixtureOperations(Dictionary<string, object> fixture)
    {
        var result = new Dictionary<string, int?>();
        foreach (var entry in (Dictionary<string, object>)fixture["native_parameters"])
        {
            var operation = (Dictionary<string, object>)entry.Value;
            result.Add(entry.Key, (string)operation["op"] == "default" ? (int?)null : Convert.ToInt32(operation["value"]));
        }
        return result;
    }

    private static void CheckSubset(Dictionary<string, object> fixture, int[] values)
    {
        foreach (var entry in (Dictionary<string, object>)fixture["expected_native_subset"])
            Check(values[Array.IndexOf(Ids, entry.Key)] == Convert.ToInt32(entry.Value),
                (string)fixture["name"] + ": " + entry.Key);
    }

    private static readonly int[] Paul = { 1,3,100,122,100,0,70,0,0,100,3300,260,3650,330,70,70,65,74,68,60,48,64,86,18,0,40,18,32 };

    public static object Planning(string helper, string fixtures)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type edits = assembly.GetType("OmnivoxDectalkParameters", true);
        var rows = new List<object>();
        foreach (var fixture in FixtureCases(fixtures))
        {
            int[] mapped = (int[])Paul.Clone();
            foreach (var entry in (Dictionary<string, object>)fixture["mapped_common"])
                mapped[Array.IndexOf(Ids, entry.Key)] = Convert.ToInt32(entry.Value);
            var context = (Dictionary<string, object>)fixture["context"];
            object plan = New(edits, FixtureOperations(fixture), new List<string>(context.Keys).ToArray());
            int[] actual = (int[])Call(plan, "Compose", Paul, mapped);
            CheckSubset(fixture, actual);
            string[] masked = (string[])edits.GetProperty("MaskedParameters", Hidden).GetValue(plan, null);
            var expected = new List<string>();
            foreach (object id in (IEnumerable)fixture["expected_masked"]) expected.Add((string)id);
            Check(String.Join(",", masked) == String.Join(",", expected.ToArray()), "masked fixture differs");
            rows.Add(Record("case", fixture["name"], "passed", true));
        }
        int[] low = (int[])edits.GetField("Minimum", Hidden).GetValue(null);
        int[] high = (int[])edits.GetField("Maximum", Hidden).GetValue(null);
        for (int i = 0; i < Ids.Length; i++)
        {
            string id = Ids[i];
            foreach (int invalid in new[] { low[i] - 1, high[i] + 1 })
                Rejects(() => New(edits, new Dictionary<string, int?> { { id, invalid } },
                    new[] { "average_pitch", "pitch_range", "stress", "richness" }),
                    typeof(ArgumentOutOfRangeException), "masked invalid " + id);
        }
        Rejects(() => New(edits, new Dictionary<string, int?> { { "unknown", 1 } }, new string[0]),
            typeof(ArgumentException), "unknown parameter");
        Rejects(() => New(edits, new Dictionary<string, int?>(), new[] { "unknown" }),
            typeof(ArgumentException), "unknown context");
        Rejects(() => New(edits, new Dictionary<string, int?>(), new[] { "stress", "stress" }),
            typeof(ArgumentException), "duplicate context");
        var ops = new Dictionary<string, int?> { { "sm", null }, { "br", 0 }, { "hr", 50 } };
        string[] dimensions = { "stress" };
        object frozen = New(edits, ops, dimensions);
        ops["br"] = 22; dimensions[0] = "richness";
        int[] common = (int[])Paul.Clone(); common[1] = 0;
        int[] composed = (int[])Call(frozen, "Compose", Paul, common);
        Check(composed[1] == 3 && composed[5] == 0 && composed[26] == 18, "frozen/default/zero/context");
        return Record("status", "passed", "fixtures", rows, "invalid_values_rejected", 56);
    }

    private static Delegate OriginalReset;
    private static readonly ManualResetEvent ResetEntered = new ManualResetEvent(false);
    private static readonly ManualResetEvent ReleaseReset = new ManualResetEvent(false);
    private static int BlockNextReset;
    private static uint BlockingReset(IntPtr handle, bool modes)
    {
        if (Interlocked.Exchange(ref BlockNextReset, 0) == 1)
        {
            ResetEntered.Set();
            Check(ReleaseReset.WaitOne(10000), "reset test release timeout");
        }
        try { return (uint)OriginalReset.DynamicInvoke(handle, modes); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }

    private static Action BeforeReset;
    private static bool FailReset, FailRestoration;
    private static int ResetCalls, SpeakCalls;
    private static Delegate OriginalSpeak;
    private static Sink LifecycleSink;

    private static uint ObservedReset(IntPtr handle, bool modes)
    {
        Interlocked.Increment(ref ResetCalls);
        if (BeforeReset != null) BeforeReset();
        if (FailReset) return 1;
        try { return (uint)OriginalReset.DynamicInvoke(handle, modes); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }

    private static uint ObservedSpeak(IntPtr handle, IntPtr text, uint flags)
    {
        Interlocked.Increment(ref SpeakCalls);
        if (FailRestoration && LifecycleSink.Frames > 0 &&
            Marshal.PtrToStringAnsi(text) == "[:np]") return 1;
        try { return (uint)OriginalSpeak.DynamicInvoke(handle, text, flags); }
        catch (TargetInvocationException error) { throw error.InnerException; }
    }

    private static uint FailingSetRate(IntPtr handle, uint rate) { return 1; }

    private static Delegate BatchSpeak, BatchSync, BatchRead;
    private static bool TextQueued, DamageReadback;
    private static int PreparationSyncs;
    private static uint BatchObservedSpeak(IntPtr handle, IntPtr text, uint flags)
    {
        if (Marshal.PtrToStringAnsi(text).Contains("Batch")) TextQueued = true;
        return (uint)BatchSpeak.DynamicInvoke(handle, text, flags);
    }
    private static uint BatchObservedSync(IntPtr handle)
    {
        if (!TextQueued) PreparationSyncs++;
        return (uint)BatchSync.DynamicInvoke(handle);
    }
    private static uint BatchObservedRead(IntPtr handle, uint index,
        out IntPtr current, out IntPtr low, out IntPtr high, out IntPtr defaults)
    {
        object[] args = { handle, index, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero, IntPtr.Zero };
        uint status = (uint)BatchRead.DynamicInvoke(args);
        current = (IntPtr)args[2]; low = (IntPtr)args[3]; high = (IntPtr)args[4]; defaults = (IntPtr)args[5];
        if (DamageReadback)
        {
            DamageReadback = false;
            Marshal.WriteInt16(current, 2, 62); // Wrong smoothness; native state stays intact.
        }
        return status;
    }

    public static object BatchedParameters(string helper, string dll)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        object adapter = New(assembly.GetType("OmnivoxDectalkAdapter", true), dll);
        object capture = Field(adapter, "capture"), native = Field(capture, "native");
        Type sinkType = assembly.GetType("IOmnivoxCaptureSink", true);
        Type anchorType = assembly.GetType("OmnivoxHelperAnchor", true);
        Array anchors = Array.CreateInstance(anchorType, 1);
        anchors.SetValue(New(anchorType, "leading", (uint)0, "before"), 0);
        var cases = new List<string>();
        FieldInfo sf = native.GetType().GetField("speak", Hidden);
        FieldInfo yf = native.GetType().GetField("sync", Hidden);
        FieldInfo rf = native.GetType().GetField("getSpeakerParams", Hidden);
        BatchSpeak = (Delegate)sf.GetValue(native); BatchSync = (Delegate)yf.GetValue(native);
        BatchRead = (Delegate)rf.GetValue(native);
        sf.SetValue(native, Delegate.CreateDelegate(sf.FieldType, typeof(DectalkExecutionAudit).GetMethod("BatchObservedSpeak", Hidden)));
        yf.SetValue(native, Delegate.CreateDelegate(yf.FieldType, typeof(DectalkExecutionAudit).GetMethod("BatchObservedSync", Hidden)));
        rf.SetValue(native, Delegate.CreateDelegate(rf.FieldType, typeof(DectalkExecutionAudit).GetMethod("BatchObservedRead", Hidden)));
        Sink sink = null;
        int receipts = 0;
        bool failReceipt = false;
        Action beforeReceipt = null;
        var cancelled = new ManualResetEvent(false);
        Func<string, string, object> synthesize = (text, voice) => {
            sink = new Sink(sinkType); receipts = PreparationSyncs = 0; TextQueued = false;
            return Call(adapter, "SynthesizeWithParameters", text, voice, 0.5, 1.0,
                null, null, null, 1.0, anchors, new Func<bool>(() => cancelled.WaitOne(0)), sink.GetTransparentProxy(),
                new Dictionary<string, int?> { { "sm", 61 } }, new string[0], new Action<int[]>(values => {
                    Check(values[1] == 61 && sink.Frames == 0 && sink.Markers == 0,
                        "incorrect or late application receipt");
                    receipts++; sink.Applied = true;
                    if (beforeReceipt != null) beforeReceipt();
                    if (failReceipt) throw new IOException("injected batch receipt failure");
                }));
        };
        try
        {
            synthesize("Batch cold.", "paul");
            Check(PreparationSyncs == 1 && receipts == 1 && sink.Frames > 0, "cold preset not verified once");
            for (int i = 0; i < 3; i++)
            {
                synthesize("Batch warm.", "paul");
                Check(PreparationSyncs == 0 && receipts == 1 && sink.Frames > 0 && sink.Markers > 0,
                    "warm parameters waited before text or lost output");
            }
            cases.Add("cold_and_warm_receipt_before_leading_anchor_and_pcm");
            synthesize("Batch other voice.", "betty");
            Check(PreparationSyncs == 1, "new voice reused another preset");
            synthesize("Batch original voice.", "paul");
            Check(PreparationSyncs == 0, "return to verified preset waited again");
            cases.Add("preset_cache_is_per_voice");
            synthesize("[:dv sm 62] Batch embedded command.", "paul");
            Check(PreparationSyncs == 3 && receipts == 1 && sink.Frames > 0,
                "embedded command did not retain pre-text verification");
            synthesize("Batch after embedded command.", "paul");
            Check(PreparationSyncs == 1, "embedded command did not invalidate presets");
            synthesize("Batch warm again.", "paul");
            Check(PreparationSyncs == 0, "plain recovery was not cached");
            cases.Add("embedded_command_fallback_and_cache_invalidation");
            // Ordinary commands must invalidate native defaults too.
            Call(adapter, "Synthesize", "[:dv sm 62] Batch ordinary command.", "paul", 0.5, 1.0,
                null, null, null, 1.0, anchors, new Func<bool>(() => false), null);
            synthesize("Batch after ordinary command.", "paul");
            Check(PreparationSyncs == 1, "ordinary vendor command retained cached presets");
            cases.Add("ordinary_command_invalidates_cache");
            DamageReadback = true;
            Rejects(() => synthesize("Batch bad readback.", "paul"), typeof(InvalidOperationException), "readback mismatch");
            Check(receipts == 0 && sink.Frames == 0 && sink.Markers == 0, "failed readback leaked output");
            synthesize("Batch after bad readback.", "paul");
            Check(PreparationSyncs == 1 && receipts == 1 && sink.Frames > 0, "readback failure did not recover");
            cases.Add("readback_failure_releases_no_output_and_recovers");
            failReceipt = true;
            Rejects(() => synthesize("Batch failed receipt.", "paul"), typeof(IOException), "receipt failure");
            Check(sink.Frames == 0 && sink.Markers == 0, "failed receipt leaked output");
            failReceipt = false;
            synthesize("Batch after failed receipt.", "paul");
            cases.Add("receipt_failure_releases_no_output_and_recovers");
            var entered = new ManualResetEvent(false);
            var release = new ManualResetEvent(false);
            var stopStarted = new ManualResetEvent(false);
            Exception workerError = null, stopError = null;
            beforeReceipt = () => {
                entered.Set();
                Check(release.WaitOne(10000), "receipt release timeout");
            };
            Thread worker = new Thread(() => {
                try { synthesize("Batch cancellation during receipt.", "paul"); }
                catch (Exception error) { workerError = error; }
            });
            Thread stopper = new Thread(() => {
                stopStarted.Set();
                try { Call(adapter, "Stop"); } catch (Exception error) { stopError = error; }
            });
            worker.IsBackground = stopper.IsBackground = true;
            bool stopperStarted = false;
            try
            {
                worker.Start();
                Check(entered.WaitOne(10000), "receipt callback was not reached");
                cancelled.Set(); stopper.Start(); stopperStarted = true;
                Check(stopStarted.WaitOne(10000), "Stop did not start");
            }
            finally
            {
                release.Set();
                Check(worker.Join(10000) && (!stopperStarted || stopper.Join(10000)), "receipt/Stop deadlock");
                beforeReceipt = null;
                entered.Dispose(); release.Dispose(); stopStarted.Dispose();
            }
            Check(workerError is OperationCanceledException && stopError == null,
                "receipt cancellation failed: " + workerError + stopError);
            Check(sink.Frames == 0 && sink.Markers == 0, "cancelled receipt leaked output");
            cancelled.Reset();
            synthesize("Batch after receipt cancellation.", "paul");
            cases.Add("stop_during_receipt_releases_no_output_and_recovers");
            foreach (string text in new[] { "", " ", "." })
            {
                synthesize(text, "paul");
                Check(receipts == 1, "silent text omitted its receipt");
            }
            cases.Add("silent_text_has_one_receipt");
        }
        finally
        {
            DamageReadback = false;
            sf.SetValue(native, BatchSpeak); yf.SetValue(native, BatchSync); rf.SetValue(native, BatchRead);
            ((IDisposable)adapter).Dispose();
            cancelled.Dispose();
        }
        return Record("status", "passed", "cases", cases);
    }

    // Exercise actual compiled helper state with bounded native-call faults.
    // No production hooks or timing thresholds are needed for these checks.
    public static object ResetLifecycle(string helper, string dll)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type adapterType = assembly.GetType("OmnivoxDectalkAdapter", true);
        Type sinkType = assembly.GetType("IOmnivoxCaptureSink", true);
        Array anchors = Array.CreateInstance(assembly.GetType("OmnivoxHelperAnchor", true), 0);
        var cases = new List<string>();
        foreach (string scenario in new[] { "first_and_warm_audio_before_reset", "setup_failure_recovery",
            "cleanup_reset_failure_rejects_reuse", "restoration_failure_rejects_reuse", "stop_during_cleanup" })
        {
            object adapter = New(adapterType, dll);
            object native = Field(Field(adapter, "capture"), "native");
            FieldInfo resetField = native.GetType().GetField("reset", Hidden);
            FieldInfo speakField = native.GetType().GetField("speak", Hidden);
            FieldInfo rateField = native.GetType().GetField("setRate", Hidden);
            OriginalReset = (Delegate)resetField.GetValue(native);
            OriginalSpeak = (Delegate)speakField.GetValue(native);
            object originalRate = rateField.GetValue(native);
            ResetCalls = SpeakCalls = 0;
            BeforeReset = null; FailReset = FailRestoration = false;
            resetField.SetValue(native, Delegate.CreateDelegate(resetField.FieldType,
                typeof(DectalkExecutionAudit).GetMethod("ObservedReset", Hidden)));
            speakField.SetValue(native, Delegate.CreateDelegate(speakField.FieldType,
                typeof(DectalkExecutionAudit).GetMethod("ObservedSpeak", Hidden)));
            Action ordinary = () => {
                LifecycleSink = new Sink(sinkType); LifecycleSink.Applied = true;
                Call(adapter, "Synthesize", Text, "paul", 0.5, 1.0, null, null, null, 1.0,
                    anchors, new Func<bool>(() => false), LifecycleSink.GetTransparentProxy());
                Check(LifecycleSink.Frames > 0 && LifecycleSink.Markers > 0, "missing follow-up output");
            };
            try
            {
                if (scenario == "first_and_warm_audio_before_reset")
                {
                    BeforeReset = () => Check(LifecycleSink.Frames > 0, "reset delayed first audio");
                    for (int i = 1; i <= 3; i++)
                    {
                        ordinary();
                        Check(ResetCalls == i, "expected one cleanup per ordinary utterance");
                        Call(adapter, "Stop");
                        Check(ResetCalls == i, "idle Stop reset a ready instance");
                    }
                }
                else if (scenario == "setup_failure_recovery")
                {
                    rateField.SetValue(native, Delegate.CreateDelegate(rateField.FieldType,
                        typeof(DectalkExecutionAudit).GetMethod("FailingSetRate", Hidden)));
                    Rejects(ordinary, typeof(InvalidOperationException), "set-rate failure");
                    Check(SpeakCalls == 0 && ResetCalls == 1, "failed setup was not drained");
                    rateField.SetValue(native, originalRate);
                    ordinary();
                }
                else if (scenario == "cleanup_reset_failure_rejects_reuse" ||
                    scenario == "restoration_failure_rejects_reuse")
                {
                    if (scenario == "cleanup_reset_failure_rejects_reuse")
                    {
                        FailReset = true;
                        Rejects(ordinary, typeof(InvalidOperationException), "cleanup reset failure");
                        FailReset = false;
                    }
                    else
                    {
                        FailRestoration = true;
                        LifecycleSink = new Sink(sinkType);
                        Rejects(() => Call(adapter, "SynthesizeWithParameters", Text, "paul", 0.5, 1.0,
                            null, null, null, 1.0, anchors, new Func<bool>(() => false),
                            LifecycleSink.GetTransparentProxy(), new Dictionary<string, int?> { { "sm", 61 } },
                            new string[0], new Action<int[]>(values => LifecycleSink.Applied = true)),
                            typeof(InvalidOperationException), "voice restoration failure");
                        FailRestoration = false;
                    }
                    Check(LifecycleSink.Frames > 0, "cleanup failure occurred before speech");
                    int beforeSpeak = SpeakCalls, beforeReset = ResetCalls;
                    Rejects(ordinary, typeof(InvalidOperationException), "reuse after failed cleanup");
                    Check(SpeakCalls == beforeSpeak && ResetCalls == beforeReset,
                        "failed cleanup allowed more native work");
                }
                else
                {
                    var entered = new ManualResetEvent(false);
                    var release = new ManualResetEvent(false);
                    var stopStarted = new ManualResetEvent(false);
                    var stopDone = new ManualResetEvent(false);
                    Exception workerError = null, stopError = null;
                    BeforeReset = () => {
                        Check(LifecycleSink.Frames > 0, "cleanup began before audio");
                        entered.Set();
                        Check(release.WaitOne(10000), "cleanup release timeout");
                    };
                    Thread worker = new Thread(() => {
                        try { ordinary(); } catch (Exception error) { workerError = error; }
                    });
                    Thread stopper = new Thread(() => {
                        stopStarted.Set();
                        try { Call(adapter, "Stop"); } catch (Exception error) { stopError = error; }
                        finally { stopDone.Set(); }
                    });
                    worker.IsBackground = stopper.IsBackground = true;
                    bool stopperStarted = false;
                    try
                    {
                        worker.Start();
                        Check(entered.WaitOne(10000), "cleanup was not reached");
                        stopper.Start(); stopperStarted = true;
                        Check(stopStarted.WaitOne(10000), "stopper was not started");
                        Check(!stopDone.WaitOne(100), "Stop bypassed the cleanup lock");
                    }
                    finally
                    {
                        release.Set();
                        Check(worker.Join(10000) && (!stopperStarted || stopper.Join(10000)), "cleanup/Stop deadlock");
                        BeforeReset = null;
                        entered.Dispose(); release.Dispose(); stopStarted.Dispose(); stopDone.Dispose();
                    }
                    Check(workerError == null && stopError == null, "cleanup overlap failed: " + workerError + stopError);
                    Check(ResetCalls == 1, "Stop reset again after completed cleanup");
                    ordinary();
                }
                cases.Add(scenario);
            }
            finally
            {
                BeforeReset = null; FailReset = FailRestoration = false;
                resetField.SetValue(native, OriginalReset);
                speakField.SetValue(native, OriginalSpeak);
                rateField.SetValue(native, originalRate);
                ((IDisposable)adapter).Dispose();
            }
        }
        return Record("status", "passed", "cases", cases);
    }

    public static object Runtime(string helper, string dll, string fixtures)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type adapterType = assembly.GetType("OmnivoxDectalkAdapter", true);
        Type editsType = assembly.GetType("OmnivoxDectalkParameters", true);
        object adapter = New(adapterType, dll);
        object capture = Field(adapter, "capture"), native = Field(capture, "native");
        IntPtr handle = (IntPtr)Field(capture, "handle");
        Array anchors = Array.CreateInstance(assembly.GetType("OmnivoxHelperAnchor", true), 0);
        var rows = new List<object>();
        int syntheses = 0, resets = 0, cancellations = 0, failures = 0, progressive = 0, races = 0;
        try
        {
            IDictionary codes = (IDictionary)adapterType.GetField("VoiceCodes", Hidden).GetValue(null);
            IDictionary pitches = (IDictionary)adapterType.GetField("VoiceAveragePitch", Hidden).GetValue(null);
            foreach (string voice in codes.Keys)
            {
                string code = (string)codes[voice];
                // Independently select and read a pristine preset without speech.
                Call(capture, "BeginCapture", null, 1.0);
                Call(capture, "Speak", "[" + code + "]");
                Check((uint)Call(native, "TextToSpeechSync", handle) == 0, "preset sync");
                int[][] limits = (int[][])Call(native, "ReadSpeakerParameterFields", handle);
                int[] pristine = limits[0];
                Func<object> ordinary = () => Call(adapter, "Synthesize", Text, voice, 0.5, 1.0,
                    null, null, null, 1.0, anchors, new Func<bool>(() => false), null);
                Bytes(ordinary()); syntheses++;
                int[] baseline = Read(native, handle);
                Action recover = () => {
                    Equal(pristine, Read(native, handle), "pristine restore " + voice);
                    Bytes(ordinary()); syntheses++; resets++;
                    Equal(baseline, Read(native, handle), "ordinary after native " + voice);
                };
                Func<Dictionary<string, int?>, string[], Func<bool>, Sink, Action<int[]>, object> speak =
                    (ops, context, cancelled, sink, applied) => Call(adapter, "SynthesizeWithParameters",
                        Text + " " + Text, voice, 0.5, 1.0, null, null, null, 1.0,
                        anchors, cancelled, sink == null ? null : sink.GetTransparentProxy(), ops, context,
                        new Action<int[]>(v => {
                            Check((ulong)Field(capture, "capturedFrames") == 0, "receipt followed PCM");
                            if (sink != null) sink.Applied = true;
                            if (applied != null) applied(v);
                        }));
                for (int index = 0; index < Ids.Length; index++)
                {
                    int selected = index;
                    foreach (int value in new[] { limits[1][index], limits[2][index] })
                    {
                        int receipts = 0;
                        Bytes(speak(new Dictionary<string, int?> { { Ids[index], value } }, new string[0],
                            () => false, null, observed => {
                                int[] expected = (int[])baseline.Clone(); expected[selected] = value;
                                Equal(expected, observed, "endpoint " + voice); receipts++;
                            }));
                        Check(receipts == 1, "endpoint receipt count"); syntheses++; recover();
                    }
                }
                int[] targets = new int[28];
                for (int i = 0; i < targets.Length; i++) targets[i] = (limits[1][i] + limits[2][i]) / 2;
                Bytes(speak(Operations(targets), new string[0], () => false, null,
                    values => Equal(targets, values, "combined edit"))); syntheses++; recover();
                var defaults = new Dictionary<string, int?>();
                foreach (string id in Ids) defaults.Add(id, null);
                Bytes(speak(defaults, new string[0], () => false, null,
                    values => Equal(pristine, values, "defaults"))); syntheses++; recover();

                int[] before = Read(native, handle);
                Rejects(() => speak(Operations(targets), new string[0], () => true, null, null),
                    typeof(OperationCanceledException), "before dispatch");
                Equal(before, Read(native, handle), "pre-dispatch state"); cancellations++;
                // Cancel after the common preamble, before native edits; then after readback.
                foreach (bool afterApplication in new[] { false, true })
                {
                    bool applied = false, observedPreamble = false;
                    int checks = 0;
                    Rejects(() => speak(Operations(targets), new string[0], () => {
                        if (afterApplication) return applied;
                        if (++checks == 4)
                        {
                            Equal(baseline, Read(native, handle), "partial common application");
                            observedPreamble = true;
                        }
                        return checks >= 4;
                    }, null, values => { applied = true; }), typeof(OperationCanceledException), "application cancel");
                    Check(afterApplication ? applied : observedPreamble && !applied, "application phase not exercised");
                    cancellations++; recover();
                }
                Rejects(() => speak(Operations(targets), new string[0], () => false, null,
                    values => { throw new IOException("injected receipt failure"); }), typeof(IOException), "receipt failure");
                failures++; recover();
                foreach (string mode in new[] { "complete", "cancel", "failure" })
                {
                    var sink = new Sink(assembly.GetType("IOmnivoxCaptureSink", true));
                    sink.CancelAfterAudio = mode == "cancel"; sink.FailAudio = mode == "failure";
                    Action action = () => {
                        object result = speak(Operations(targets), new string[0], () => sink.Cancel, sink, null);
                        Check(((byte[])Field(result, "Audio")).Length == 0, "buffered progressive PCM");
                        Check(sink.Markers > 0, "missing progressive markers");
                    };
                    if (mode == "complete") { action(); syntheses++; }
                    else if (mode == "cancel") { Rejects(action, typeof(OperationCanceledException), "PCM cancel"); cancellations++; }
                    else { Rejects(action, typeof(InvalidOperationException), "PCM failure"); failures++; }
                    Check(sink.Frames > 0, "progressive case did not reach PCM"); progressive++; recover();
                }

                // Deterministic overlap: hold the actual cancelling thread at
                // native Reset while synthesis exits and tries to restore state.
                FieldInfo resetField = native.GetType().GetField("reset", Hidden);
                OriginalReset = (Delegate)resetField.GetValue(native);
                Delegate hook = Delegate.CreateDelegate(resetField.FieldType,
                    typeof(DectalkExecutionAudit).GetMethod("BlockingReset", Hidden));
                var stopRequested = new ManualResetEvent(false);
                var done = new ManualResetEvent(false);
                var raceSink = new Sink(assembly.GetType("IOmnivoxCaptureSink", true));
                raceSink.OnAudio = () => {
                    stopRequested.Set();
                    Check(ResetEntered.WaitOne(10000), "cancelling thread did not enter reset");
                };
                Exception workerError = null, stopError = null;
                bool raceCancelled = false;
                Thread worker = new Thread(() => {
                    try { speak(Operations(targets), new string[0], () => Volatile.Read(ref raceCancelled), raceSink, null); }
                    catch (Exception error) { workerError = error; }
                    finally { done.Set(); }
                });
                Thread stopper = new Thread(() => {
                    try {
                        Check(stopRequested.WaitOne(10000), "PCM race timeout");
                        Volatile.Write(ref raceCancelled, true);
                        Interlocked.Exchange(ref BlockNextReset, 1);
                        Call(adapter, "Stop");
                    } catch (Exception error) { stopError = error; }
                });
                worker.IsBackground = stopper.IsBackground = true;
                ResetEntered.Reset(); ReleaseReset.Reset();
                resetField.SetValue(native, hook);
                try
                {
                    worker.Start(); stopper.Start();
                    Check(ResetEntered.WaitOne(10000), "reset was not entered");
                    Check(!done.WaitOne(100), "request completed while native reset was held");
                }
                finally
                {
                    ReleaseReset.Set();
                    Check(stopper.Join(10000) && worker.Join(10000), "reset/restore deadlock");
                    resetField.SetValue(native, OriginalReset);
                    stopRequested.Dispose(); done.Dispose();
                }
                Check(stopError == null, "Stop failed: " + stopError);
                Check(workerError is OperationCanceledException, "expected cancelled worker: " + workerError);
                recover(); races++; cancellations++;
                rows.Add(Record("voice_id", voice, "endpoints", 56, "reset_overlap", true));
            }
            foreach (var fixture in FixtureCases(fixtures))
            {
                var context = (Dictionary<string, object>)fixture["context"];
                double richness = Convert.ToDouble(((Dictionary<string, object>)fixture["shared_common"])["richness"]);
                if (context.ContainsKey("richness")) richness = Convert.ToDouble(((Dictionary<string, object>)context["richness"])["value"]);
                Bytes(Call(adapter, "SynthesizeWithParameters", Text, "paul", 0.5, 1.0, null, null, richness, 1.0,
                    anchors, new Func<bool>(() => false), null, FixtureOperations(fixture), new List<string>(context.Keys).ToArray(),
                    new Action<int[]>(values => CheckSubset(fixture, values)))); syntheses++;
            }
            // Legacy common pitch/stress clamp inside DECtalk, not in native edit validation.
            Bytes(Call(adapter, "Synthesize", Text, "kit", 0.5, 2.0, null, 0.0, null, 1.0,
                anchors, new Func<bool>(() => false), null)); syntheses++;
            int[] clamped = Read(native, handle);
            Bytes(Call(adapter, "SynthesizeWithParameters", Text, "kit", 0.5, 2.0, null, 0.0, null, 1.0,
                anchors, new Func<bool>(() => false), null, new Dictionary<string,int?>(), new string[0],
                new Action<int[]>(values => Equal(clamped, values, "legacy common clamps")))); syntheses++;
            FieldInfo binding = native.GetType().GetField("getSpeakerParams", Hidden);
            object saved = binding.GetValue(native);
            try
            {
                binding.SetValue(native, null);
                Rejects(() => Call(adapter, "SynthesizeWithParameters", Text, "paul", 0.5, 1.0, null, null, null, 1.0,
                    anchors, new Func<bool>(() => false), null, new Dictionary<string,int?>(), new string[0], null),
                    typeof(NotSupportedException), "missing optional binding");
                Bytes(Call(adapter, "Synthesize", Text, "paul", 0.5, 1.0, null, null, null, 1.0,
                    anchors, new Func<bool>(() => false), null)); syntheses++;
            }
            finally { binding.SetValue(native, saved); }
            Rejects(() => Call(editsType, "RequireQualifiedRuntime", "other", dll), typeof(NotSupportedException), "wrong version");
            string temp = Path.GetTempFileName();
            try { Rejects(() => Call(editsType, "RequireQualifiedRuntime", "v4.99 Github NORMAL ACCESS32 ", temp), typeof(NotSupportedException), "wrong bytes"); }
            finally { File.Delete(temp); }
        }
        finally { ((IDisposable)adapter).Dispose(); }
        return Record("status", "passed", "voices", rows, "successful_captures", syntheses,
            "ordinary_reset_checks", resets, "cancellations", cancellations, "delivery_failures", failures,
            "progressive_cases", progressive, "reset_overlap_cases", races, "composition_fixtures", 7);
    }
}
