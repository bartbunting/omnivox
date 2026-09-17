// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Probe actual helper bytes in an isolated x86 STA process; never play audio.
using System;
using System.Collections;
using System.Collections.Generic;
using System.IO;
using System.Reflection;
using System.Threading;
using System.Runtime.Remoting.Messaging;
using System.Runtime.Remoting.Proxies;
using System.Web.Script.Serialization;

public static class EloquenceExecutionAudit
{
    private const BindingFlags Hidden = BindingFlags.Public | BindingFlags.NonPublic |
        BindingFlags.Instance | BindingFlags.Static;
    private static readonly string[] Ids = { "gender", "head_size", "pitch_baseline",
        "pitch_fluctuation", "roughness", "breathiness", "speed", "volume" };
    private static readonly int[] Targets = { 1, 67, 91, 47, 22, 29, 90, 83 };
    private const string Text = "The quick brown fox jumps over the lazy dog.";

    // A local interface proxy exercises the actual helper assembly's internal
    // sink without recompiling it or introducing a native test-only entry point.
    private sealed class Sink : RealProxy
    {
        internal bool Applied;
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
        return (int[])Call(native, "ReadActiveVoiceParameters", handle);
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
            if ((string)item["engine_id"] == "eloquence") result.Add(item);
        Check(result.Count == 2, "Expected both independent Eloquence composition fixtures");
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

    public static object Planning(string helper, string fixtures)
    {
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type edits = assembly.GetType("OmnivoxEloquenceParameters", true);
        int[] pristine = { 0, 50, 65, 30, 0, 0, 50, 92 };
        var rows = new List<object>();
        foreach (var fixture in FixtureCases(fixtures))
        {
            int?[] mapped = new int?[8];
            foreach (var entry in (Dictionary<string, object>)fixture["mapped_common"])
                mapped[Array.IndexOf(Ids, entry.Key)] = Convert.ToInt32(entry.Value);
            var context = (Dictionary<string, object>)fixture["context"];
            object plan = New(edits, FixtureOperations(fixture), new List<string>(context.Keys).ToArray());
            int[] actual = (int[])Call(plan, "Compose", pristine, mapped);
            CheckSubset(fixture, actual);
            string[] masked = (string[])edits.GetProperty("MaskedParameters", Hidden).GetValue(plan, null);
            var expected = new List<string>();
            foreach (object id in (IEnumerable)fixture["expected_masked"]) expected.Add((string)id);
            Check(String.Join(",", masked) == String.Join(",", expected.ToArray()), "masked fixture differs");
            rows.Add(Record("case", fixture["name"], "passed", true));
        }
        for (int i = 0; i < Ids.Length; i++)
        {
            string id = Ids[i];
            int maximum = i == 0 ? 1 : i == 6 ? 250 : 100;
            foreach (int invalid in new[] { -1, maximum + 1 })
                Rejects(() => New(edits, new Dictionary<string, int?> { { id, invalid } },
                    new[] { "rate", "average_pitch", "pitch_range", "stress", "richness", "volume" }),
                    typeof(ArgumentOutOfRangeException), "masked invalid " + id);
        }
        Rejects(() => New(edits, new Dictionary<string, int?> { { "unknown", 1 } }, new string[0]),
            typeof(ArgumentException), "unknown parameter");
        Rejects(() => New(edits, new Dictionary<string, int?>(), new[] { "unknown" }),
            typeof(ArgumentException), "unknown context");
        Rejects(() => New(edits, new Dictionary<string, int?>(), new[] { "richness", "richness" }),
            typeof(ArgumentException), "duplicate context");
        var operations = new Dictionary<string, int?> { { "volume", null }, { "head_size", 0 }, { "speed", 200 } };
        string[] dimensions = { "rate_offset" };
        object frozen = New(edits, operations, dimensions);
        operations["head_size"] = 99;
        dimensions[0] = "volume";
        int?[] common = { null, null, 65, null, null, null, 75, 100 };
        int[] composed = (int[])Call(frozen, "Compose", pristine, common);
        Check(composed[1] == 0 && composed[6] == 75 && composed[7] == 92,
            "zero/default/equal context or immutable snapshot differs");
        rows.Add(Record("case", "atomic_validation_and_frozen_edits", "passed", true));
        return Record("status", "passed", "fixtures", rows, "invalid_values_rejected", 16,
            "profile_id", edits.GetField("ProfileId", Hidden).GetRawConstantValue());
    }

    public static object Runtime(string helper, string dll, string fixtures)
    {
        Check(Thread.CurrentThread.GetApartmentState() == ApartmentState.STA, "audit requires STA");
        Assembly assembly = Assembly.Load(File.ReadAllBytes(helper));
        Type captureType = assembly.GetType("OmnivoxEloquenceCapture", true);
        Type editsType = assembly.GetType("OmnivoxEloquenceParameters", true);
        Type adapterType = assembly.GetType("OmnivoxEloquenceAdapter", true);
        Array anchors = Array.CreateInstance(assembly.GetType("OmnivoxHelperAnchor", true), 0);
        object capture = New(captureType, dll);
        var rows = new List<object>();
        int syntheses = 0, resets = 0, cancellationCases = 0, failureCases = 0;
        int progressiveCases = 0, progressiveFailures = 0;
        try
        {
            object native = Field(capture, "native");
            IntPtr handle = (IntPtr)Field(capture, "handle");
            for (int voice = 1; voice <= 8; voice++)
            {
                string voiceId = "v" + voice;
                Call(native, "CopyPresetToActive", handle, voice);
                int[] pristine = Read(native, handle);
                int?[] common = { null, null, pristine[2], null, null, null, 75, 100 };
                int[] ordinary = (int[])pristine.Clone(); ordinary[6] = 75; ordinary[7] = 100;
                Action recover = delegate() {
                    Equal(pristine, Read(native, handle), "pristine after native request " + voiceId);
                    syntheses++;
                    Bytes(Call(capture, "Synthesize", Text, voiceId, 75, pristine[2], "", 100,
                        anchors, new Func<bool>(() => false), null));
                    Equal(ordinary, Read(native, handle), "ordinary reset " + voiceId);
                    resets++;
                };
                Action<Dictionary<string, int?>, string[], string, Func<bool>, Action<int[]>> speak =
                    delegate(Dictionary<string, int?> operations, string[] context, string text,
                        Func<bool> cancelled, Action<int[]> applied) {
                        object edits = New(editsType, operations, context);
                        object result = Call(capture, "SynthesizeNative", text, voiceId, common, edits,
                            anchors, cancelled, null, applied);
                        Bytes(result); syntheses++;
                    };
                for (int index = 0; index < Ids.Length; index++)
                {
                    int expectedIndex = index;
                    foreach (int value in new[] { 0, index == 0 ? 1 : index == 6 ? 250 : 100 })
                    {
                        int callbacks = 0;
                        speak(new Dictionary<string, int?> { { Ids[index], value } }, new string[0], Text,
                            () => false, observed => {
                                callbacks++;
                                int[] expected = (int[])ordinary.Clone(); expected[expectedIndex] = value;
                                Equal(expected, observed, "endpoint " + voiceId);
                                Check((ulong)Field(capture, "capturedFrames") == 0, "readback followed PCM");
                            });
                        Check(callbacks == 1, "expected one pre-PCM readback");
                        recover();
                    }
                }
                speak(Operations(Targets), new string[0], Text, () => false,
                    observed => Equal(Targets, observed, "combined edit " + voiceId));
                recover();
                var defaults = new Dictionary<string, int?>();
                foreach (string id in Ids) defaults.Add(id, null);
                speak(defaults, new string[0], Text, () => false,
                    observed => Equal(pristine, observed, "all defaults " + voiceId));
                recover();
                foreach (string phase in new[] { "partial_application", "after_application", "after_pcm" })
                {
                    int checks = 0;
                    bool applied = false, partialObserved = false;
                    Func<bool> cancel = () => {
                        if (phase == "partial_application")
                        {
                            if (++checks == 5)
                            {
                                int[] current = Read(native, handle);
                                bool changed = false, unchanged = false;
                                for (int i = 0; i < current.Length; i++)
                                {
                                    if (Targets[i] == pristine[i]) continue;
                                    changed |= current[i] == Targets[i];
                                    unchanged |= current[i] == pristine[i];
                                }
                                partialObserved = changed && unchanged;
                            }
                            return checks >= 5;
                        }
                        if (phase == "after_application") return applied;
                        return applied && (ulong)Field(capture, "capturedFrames") > 0;
                    };
                    Rejects(() => speak(Operations(Targets), new string[0], Text + " " + Text,
                        cancel, observed => { applied = true; }), typeof(OperationCanceledException), phase);
                    if (phase == "partial_application")
                        Check(partialObserved && !applied, "cancellation did not interrupt a partial native plan");
                    if (phase == "after_pcm")
                        Check((ulong)Field(capture, "capturedFrames") > 0, "cancellation did not reach PCM");
                    else Check((ulong)Field(capture, "capturedFrames") == 0, "early cancellation emitted PCM");
                    recover(); cancellationCases++;
                }
                int[] beforeCancelledDispatch = Read(native, handle);
                Rejects(() => speak(Operations(Targets), new string[0], Text, () => true,
                    observed => { throw new Exception("cancelled dispatch applied parameters"); }),
                    typeof(OperationCanceledException), "cancelled before dispatch");
                Equal(beforeCancelledDispatch, Read(native, handle), "pre-dispatch cancellation mutated state");
                cancellationCases++;
                Rejects(() => speak(Operations(Targets), new string[0], Text, () => false,
                    observed => { throw new IOException("injected receipt failure"); }),
                    typeof(IOException), "failure after native application");
                Check((ulong)Field(capture, "capturedFrames") == 0, "receipt failure emitted PCM");
                recover(); failureCases++;
                foreach (string mode in new[] { "complete", "cancel", "failure" })
                {
                    var sink = new Sink(assembly.GetType("IOmnivoxCaptureSink", true));
                    sink.CancelAfterAudio = mode == "cancel";
                    sink.FailAudio = mode == "failure";
                    object edits = New(editsType, Operations(Targets), new string[0]);
                    Action progressive = () => {
                        object result = Call(capture, "SynthesizeNative", Text + " " + Text,
                            voiceId, common, edits, anchors, new Func<bool>(() => sink.Cancel),
                            sink.GetTransparentProxy(), new Action<int[]>(observed => {
                                Equal(Targets, observed, "progressive applied values");
                                sink.Applied = true;
                            }));
                        Check(((byte[])Field(result, "Audio")).Length == 0, "progressive path buffered PCM");
                        Check(sink.Frames > 0 && sink.Markers > 0, "missing progressive PCM or markers");
                    };
                    if (mode == "complete") { progressive(); syntheses++; }
                    else if (mode == "cancel")
                    {
                        Rejects(progressive, typeof(OperationCanceledException), "progressive cancellation");
                        cancellationCases++;
                    }
                    else
                    {
                        Rejects(progressive, typeof(InvalidOperationException), "progressive PCM failure");
                        progressiveFailures++;
                    }
                    Check(sink.Frames > 0, "progressive interruption did not reach PCM");
                    recover(); progressiveCases++;
                }
                rows.Add(Record("voice_id", voiceId, "endpoint_cases", 16,
                    "combined_edits", true, "explicit_defaults", true,
                    "cancellation_phases", 4, "failure_reset", true, "progressive_cases", 3));
            }
            // Optional exports remain unnecessary for ordinary speech.
            foreach (string name in new[] { "getParam", "getVoiceParam", "setVoiceParam", "copyVoice" })
            {
                FieldInfo field = native.GetType().GetField(name, Hidden);
                object saved = field.GetValue(native);
                try
                {
                    field.SetValue(native, null);
                    object edits = New(editsType, Operations(Targets), new string[0]);
                    Rejects(() => Call(capture, "SynthesizeNative", Text, "v1", new int?[8], edits,
                        anchors, new Func<bool>(() => false), null, new Action<int[]>(v => { })),
                        typeof(NotSupportedException), "missing " + name);
                    Bytes(Call(capture, "Synthesize", Text, "v1", 75, 65, "", 100,
                        anchors, new Func<bool>(() => false), null)); syntheses++;
                }
                finally { field.SetValue(native, saved); }
            }
            Rejects(() => Call(editsType, "RequireQualifiedRuntime", "other", dll),
                typeof(NotSupportedException), "unqualified version");
            string scratch = Path.GetTempFileName();
            try
            {
                File.WriteAllText(scratch, "not the qualified runtime");
                Rejects(() => Call(editsType, "RequireQualifiedRuntime", "6.1.0.0", scratch),
                    typeof(NotSupportedException), "unqualified DLL bytes");
            }
            finally { File.Delete(scratch); }
        }
        finally { ((IDisposable)capture).Dispose(); }

        object adapter = New(adapterType, dll);
        try
        {
            int callerThread = Thread.CurrentThread.ManagedThreadId;
            foreach (var fixture in FixtureCases(fixtures))
            {
                var context = (Dictionary<string, object>)fixture["context"];
                double richness = Convert.ToDouble(((Dictionary<string, object>)fixture["shared_common"])["richness"]);
                if (context.ContainsKey("richness"))
                    richness = Convert.ToDouble(((Dictionary<string, object>)context["richness"])["value"]);
                int callbacks = 0;
                Bytes(Call(adapter, "SynthesizeWithParameters", Text, "v1", 0.5, 1.0,
                    null, null, richness, 1.0, anchors, new Func<bool>(() => false), null,
                    FixtureOperations(fixture), new List<string>(context.Keys).ToArray(),
                    new Action<int[]>(observed => {
                        callbacks++;
                        Check(Thread.CurrentThread.ManagedThreadId != callerThread &&
                            Thread.CurrentThread.GetApartmentState() == ApartmentState.STA,
                            "native application left the adapter's STA owner thread");
                        CheckSubset(fixture, observed);
                    })));
                Check(callbacks == 1, "adapter readback callback count"); syntheses++;
                Bytes(Call(adapter, "Synthesize", Text, "v1", 0.5, 1.0, null, null, null, 1.0,
                    anchors, new Func<bool>(() => false), null)); syntheses++;
            }
        }
        finally { ((IDisposable)adapter).Dispose(); }
        return Record("status", "passed", "voices", rows, "successful_captures", syntheses,
            "ordinary_reset_checks", resets, "cancellation_cases", cancellationCases,
            "receipt_failure_cases", failureCases, "progressive_pcm_failures", progressiveFailures,
            "progressive_cases", progressiveCases, "owner_thread_fixtures", 2,
            "scope", "native execution, pre-PCM readback and reset; no helper-6 wire or UI");
    }
}
