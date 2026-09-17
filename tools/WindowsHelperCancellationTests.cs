// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
// Test the real helper protocol loop with controlled native calls and wire output.

using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

public static class WindowsHelperCancellationTests
{
    private const int Timeout = 5000;

    private static void Check(bool condition, string message)
    {
        if (!condition) throw new Exception(message);
    }

    private static void Wait(WaitHandle signal, string name)
    {
        Check(signal.WaitOne(Timeout), "Timed out: " + name);
    }

    private sealed class Input : TextReader
    {
        private readonly Queue<char> pending = new Queue<char>();
        private bool closed;
        internal readonly AutoResetEvent LineRead = new AutoResetEvent(false);

        internal void Send(string line)
        {
            lock (pending)
            {
                foreach (char value in line + "\n") pending.Enqueue(value);
                Monitor.PulseAll(pending);
            }
        }

        public override int Read()
        {
            lock (pending)
            {
                while (pending.Count == 0 && !closed) Monitor.Wait(pending);
                if (pending.Count == 0) return -1;
                char value = pending.Dequeue();
                if (value == '\n') LineRead.Set();
                return value;
            }
        }

        public override void Close()
        {
            lock (pending)
            {
                closed = true;
                Monitor.PulseAll(pending);
            }
        }
    }

    private sealed class Output : TextWriter
    {
        private readonly List<Dictionary<string, object>> frames =
            new List<Dictionary<string, object>>();
        internal string BlockType;
        internal readonly ManualResetEvent Blocked = new ManualResetEvent(false);
        internal readonly ManualResetEvent Release = new ManualResetEvent(false);
        public override Encoding Encoding { get { return Encoding.UTF8; } }

        public override void WriteLine(string line)
        {
            Dictionary<string, object> frame =
                new JavaScriptSerializer().Deserialize<Dictionary<string, object>>(line);
            if ((string)frame["type"] == BlockType)
            {
                Blocked.Set();
                Wait(Release, "release blocked " + BlockType);
            }
            lock (frames)
            {
                frames.Add(frame);
                Monitor.PulseAll(frames);
            }
        }

        internal Dictionary<string, object>[] Snapshot()
        {
            lock (frames) return frames.ToArray();
        }

        internal bool Has(int id, string type)
        {
            foreach (Dictionary<string, object> frame in Snapshot())
                if (Convert.ToInt32(frame["request_id"]) == id &&
                    (string)frame["type"] == type) return true;
            return false;
        }

        internal void Await(int id, string type)
        {
            DateTime deadline = DateTime.UtcNow.AddMilliseconds(Timeout);
            lock (frames)
            {
                while (!Has(id, type))
                {
                    TimeSpan left = deadline - DateTime.UtcNow;
                    Check(left.TotalMilliseconds > 0,
                        "Timed out waiting for " + id + ": " + type);
                    Monitor.Wait(frames, left);
                }
            }
        }
    }

    private class Engine : IOmnivoxCaptureEngine
    {
        internal readonly ManualResetEvent Entered = new ManualResetEvent(false);
        internal readonly ManualResetEvent Release = new ManualResetEvent(false);
        internal readonly ManualResetEvent Returned = new ManualResetEvent(false);
        internal Action OnStop;
        internal string Mode;
        internal bool Progressive;
        internal int RejectedCallbacks;
        internal int SynthesisCalls;
        public string EngineId { get { return "test"; } }
        public string DisplayName { get { return "Test"; } }
        public string Version { get { return "1"; } }
        public string HelperName { get { return "Cancellation test"; } }
        public string DefaultVoiceId { get { return "test"; } }
        public int SampleRate { get { return 11025; } }
        public int Channels { get { return 1; } }
        public bool SupportsProgressiveSynthesis { get { return Progressive; } }
        public OmnivoxHelperVoice[] Voices
        {
            get { return new[] { new OmnivoxHelperVoice("test", "Test", "en", "unknown") }; }
        }
        public OmnivoxHelperCapabilities Capabilities
        {
            get { return new OmnivoxHelperCapabilities { Rate = true, Volume = true }; }
        }

        public OmnivoxCaptureResult Synthesize(string text, string voiceId,
            double rate, double pitch, double? pitchRange, double? stress,
            double? richness, double volume, OmnivoxHelperAnchor[] anchors,
            Func<bool> cancelled, IOmnivoxCaptureSink sink)
        {
            Interlocked.Increment(ref SynthesisCalls);
            byte[] audio = new byte[8];
            if (sink != null) sink.Audio(audio, 0, audio.Length);
            if (text == "hold")
            {
                Entered.Set();
                try
                {
                    Wait(Release, "release native synthesis");
                    if (Mode == "error") throw new InvalidOperationException("native failure");
                    if (sink != null && Mode == "callbacks")
                    {
                        try { sink.Audio(audio, 0, audio.Length); }
                        catch (OperationCanceledException) { ++RejectedCallbacks; }
                        try
                        {
                            sink.Markers(new[] { new OmnivoxHelperMarker(
                                "word", 4, 0, 4, null) });
                        }
                        catch (OperationCanceledException) { ++RejectedCallbacks; }
                    }
                }
                finally { Returned.Set(); }
            }
            return new OmnivoxCaptureResult(sink == null ? audio : new byte[0],
                new OmnivoxHelperMarker[0]);
        }

        public void Stop() { if (OnStop != null) OnStop(); }
        public void Dispose() { }
    }

    private sealed class ParameterEngine : Engine, IOmnivoxParameterEngine, IOmnivoxParameterSynthesis
    {
        public Dictionary<string, object> GetParameters(IDictionary<string, object> query)
        { return OmnivoxParameterWire.Map("status", "busy", "retry_after_ms", 50); }
        public Dictionary<string, object> ExplainParameters(object source)
        { return OmnivoxParameterWire.Map("status", "busy", "retry_after_ms", 50); }
        public IOmnivoxParameterSynthesis PrepareParameters(IDictionary<string, object> settings, object parameters)
        { return this; }
        public OmnivoxCaptureResult Synthesize(string text, OmnivoxHelperAnchor[] anchors,
            Func<bool> cancelled, IOmnivoxCaptureSink sink, Action<Dictionary<string, object>> applied)
        {
            Interlocked.Increment(ref SynthesisCalls);
            byte[] audio = new byte[8];
            if (Mode == "early_audio") sink.Audio(audio, 0, audio.Length);
            if (Mode == "omitted_receipt")
                return new OmnivoxCaptureResult(audio, new OmnivoxHelperMarker[0]);
            if (text == "hold") { Entered.Set(); Wait(Release, "release native preparation"); }
            applied(OmnivoxParameterWire.Map("status", "applied", "plan_id", "test-plan"));
            if (sink != null) sink.Audio(audio, 0, audio.Length);
            return new OmnivoxCaptureResult(sink == null ? audio : new byte[0], new OmnivoxHelperMarker[0]);
        }
    }

    private sealed class Session : IDisposable
    {
        internal readonly Input Input = new Input();
        internal readonly Output Output = new Output();
        internal readonly Engine Engine;
        private readonly int version;
        private readonly Thread loop;
        private Exception failure;

        internal Session(int version, bool progressive, string mode)
        {
            this.version = version;
            Engine = version == 6 ? new ParameterEngine() : new Engine();
            Engine.Progressive = progressive;
            Engine.Mode = mode;
            OmnivoxHelperHost host = new OmnivoxHelperHost(Engine, Input, Output);
            loop = new Thread(delegate()
            {
                try { host.Run(); }
                catch (Exception error) { failure = error; }
            });
            loop.IsBackground = true;
            loop.Start();
            Send(1, "hello", "\"supported_protocol_versions\":[" + version + "]");
            Output.Await(1, "hello");
        }

        internal void Send(int id, string type, string fields)
        {
            Input.Send("{\"protocol_version\":" + version +
                ",\"request_id\":" + id + ",\"type\":\"" + type + "\"" +
                (fields == null ? "" : "," + fields) + "}");
        }

        internal void Speak(int id, string text)
        {
            Send(id, "synthesize", "\"text\":\"" + text +
                "\",\"settings\":{\"voice_id\":null,\"rate\":0.5," +
                "\"pitch\":1.0,\"volume\":1.0" +
                (version >= 3 ? ",\"pitch_range\":null,\"stress\":null,\"richness\":null" : "") +
                "}" + (version >= 2 ? ",\"anchors\":[]" : "") +
                (version == 6 ? ",\"voice_parameters\":null" : ""));
        }

        internal void CheckCancellation()
        {
            bool accepted = false;
            int terminals = 0;
            foreach (Dictionary<string, object> frame in Output.Snapshot())
            {
                int id = Convert.ToInt32(frame["request_id"]);
                string type = (string)frame["type"];
                if (id == 3)
                {
                    Check(type == "cancel_accepted", "extra cancel response: " + type);
                }
                if (id == 3 && type == "cancel_accepted")
                {
                    Check(Convert.ToInt32(frame["target_request_id"]) == 2, "wrong cancel target");
                    accepted = true;
                }
                if (id != 2) continue;
                if (type == "synthesis_cancelled")
                {
                    Check(accepted, "synthesis_cancelled preceded cancel_accepted");
                    ++terminals;
                }
                else
                {
                    Check(type != "synthesis_completed" && type != "error", "wrong terminal after cancellation");
                    Check(!accepted, "speech output followed cancel_accepted: " + type);
                }
            }
            Check(accepted && terminals == 1, "expected acknowledgement and exactly one cancelled terminal");
        }

        internal void FollowUp()
        {
            Send(4, "cancel", "\"target_request_id\":2");
            Output.Await(4, "error");
            Speak(5, "next");
            Output.Await(5, "synthesis_completed");
            Send(6, "ping", null);
            Output.Await(6, "pong");
            Check(!Output.Has(5, "error"), "follow-up synthesis failed");
        }

        public void Dispose()
        {
            Output.Release.Set();
            Engine.Release.Set();
            Input.Close();
            Check(loop.Join(Timeout), "protocol loop did not stop");
            if (failure != null) throw new Exception("protocol loop failed", failure);
        }
    }

    private static void CancelDuringNativeStop(int version, bool progressive, string mode)
    {
        using (Session session = new Session(version, progressive, mode))
        {
            bool acknowledgedBeforeStop = false;
            bool terminalDuringStop = false;
            session.Engine.OnStop = delegate()
            {
                acknowledgedBeforeStop = session.Output.Has(3, "cancel_accepted");
                session.Engine.Release.Set();
                session.Output.Await(2, "synthesis_cancelled");
                terminalDuringStop = true;
                if (mode == "stop_error") throw new InvalidOperationException("native stop failure");
            };
            session.Speak(2, "hold");
            Wait(session.Engine.Entered, "native entry");
            session.Send(3, "cancel", "\"target_request_id\":2");
            session.Output.Await(3, "cancel_accepted");
            session.Output.Await(2, "synthesis_cancelled");
            session.FollowUp(); // Also waits for Stop to return on the protocol thread.
            Check(acknowledgedBeforeStop, "native Stop ran before cancellation acknowledgement");
            Check(terminalDuringStop, "native Stop held the speech output guard");
            session.CheckCancellation();
            if (progressive && mode == "callbacks")
                Check(session.Engine.RejectedCallbacks == 2, "late callbacks were not both rejected");
        }
    }

    private static void CancelWhileAcknowledgementBlocked(int version, bool progressive)
    {
        using (Session session = new Session(version, progressive, "error"))
        {
            session.Output.BlockType = "cancel_accepted";
            session.Speak(2, "hold");
            Wait(session.Engine.Entered, "native entry");
            session.Send(3, "cancel", "\"target_request_id\":2");
            Wait(session.Output.Blocked, "acknowledgement at writer");
            session.Engine.Release.Set();
            Wait(session.Engine.Returned, "native return while acknowledgement blocked");
            Check(!session.Output.Has(2, "synthesis_cancelled"), "terminal overtook blocked acknowledgement");
            session.Output.Release.Set();
            session.Output.Await(2, "synthesis_cancelled");
            session.FollowUp();
            session.CheckCancellation();
        }
    }

    private static void CompletedRequestRejectsCancel(int version)
    {
        using (Session session = new Session(version, version == 5, "return"))
        {
            session.Output.BlockType = "synthesis_completed";
            session.Speak(2, "next");
            Wait(session.Output.Blocked, "completion at writer");
            session.Input.LineRead.Reset();
            session.Send(3, "cancel", "\"target_request_id\":2");
            Wait(session.Input.LineRead, "cancel read while completion blocked");
            session.Output.Release.Set();
            session.Output.Await(2, "synthesis_completed");
            session.Output.Await(3, "error");
            Check(!session.Output.Has(3, "cancel_accepted"), "accepted cancellation of completed request");
            session.FollowUp();
        }
    }

    private static void ShutdownActiveRequest(int version)
    {
        using (Session session = new Session(version, version == 5, "return"))
        {
            session.Engine.OnStop = delegate() { session.Engine.Release.Set(); };
            session.Speak(2, "hold");
            Wait(session.Engine.Entered, "native entry");
            session.Send(3, "shutdown", null);
            session.Output.Await(3, "shutting_down");
            session.Output.Await(2, "synthesis_cancelled");
            Check(!session.Output.Has(3, "cancel_accepted"), "shutdown invented a cancel acknowledgement");
        }
    }

    private static int WireValidation(int version)
    {
        string baseline = "{\"protocol_version\":" + version +
            ",\"request_id\":2,\"type\":\"synthesize\",\"text\":\"hello\"," +
            "\"settings\":{\"voice_id\":null,\"rate\":0.5,\"pitch\":1,\"volume\":1}" +
            (version >= 2 ? ",\"anchors\":[]" : "") + "}";
        List<string> ambiguous = new List<string> {
            baseline.Replace("\"request_id\":2", "\"request_id\":2,\"request_id\":3"),
            baseline.Replace("\"type\":\"synthesize\"", "\"type\":\"ping\",\"type\":\"synthesize\""),
            baseline.Replace("\"rate\":0.5", "\"rate\":0.1,\"rate\":0.5"),
            baseline.Replace("\"rate\":0.5", "\"rate\":0.1,\"\\u0072ate\":0.5"),
            baseline.Replace("\"type\"", "type"),
            baseline.Replace("\"type\"", "'type'"),
            baseline.Substring(0, baseline.Length - 1) + ",}",
            baseline.Replace("0.5", "01"),
            baseline.Replace("0.5", ".5"),
            baseline.Replace("0.5", "1."),
            baseline.Replace("0.5", "1e"),
            baseline + " false",
            baseline.Replace("hello", "bad\\x41"),
            baseline.Replace("hello", "bad\\u00xz"),
            baseline.Replace("hello", "bad\tcontrol")
        };
        if (version >= 2)
            ambiguous.Add(baseline.Replace("\"anchors\":[]",
                "\"anchors\":[{\"id\":\"a\",\"id\":\"b\",\"text_offset\":0,\"affinity\":\"before\"}]"));
        List<string> unknown = new List<string> {
            baseline.Substring(0, baseline.Length - 1) + ",\"voice_parameters\":null}",
            baseline.Substring(0, baseline.Length - 1) + ",\"voice_parameters\":{\"native\":{\"sm\":55}}}",
            baseline.Replace("\"voice_id\":null", "\"voice_id\":null,\"native\":{\"sm\":55}")
        };
        if (version == 1) unknown.Add(baseline.Substring(0, baseline.Length - 1) + ",\"anchors\":null}");
        if (version < 3)
            foreach (string field in new[] { "pitch_range", "stress", "richness" })
                unknown.Add(baseline.Replace("\"voice_id\":null", "\"voice_id\":null,\"" + field + "\":null"));
        if (version >= 2)
            unknown.Add(baseline.Replace("\"anchors\":[]",
                "\"anchors\":[{\"id\":\"a\",\"text_offset\":0,\"affinity\":\"before\",\"native\":null}]"));
        int cases = 0;
        foreach (List<string> group in new[] { ambiguous, unknown })
        {
            foreach (string invalid in group)
            {
                using (Session session = new Session(version, version == 5, "return"))
                {
                    session.Input.Send(invalid);
                    session.Output.Await(group == ambiguous ? 0 : 2, "error");
                    Check(session.Engine.SynthesisCalls == 0, "malformed input reached native synthesis");
                    foreach (var frame in session.Output.Snapshot())
                        if ((string)frame["type"] == "error")
                            Check((string)frame["code"] == "invalid_request", "wrong malformed-input error");
                    session.Speak(5, "next");
                    session.Output.Await(5, "synthesis_completed");
                    Check(session.Engine.SynthesisCalls == 1, "valid recovery did not synthesize exactly once");
                }
                cases++;
            }
        }
        using (Session session = new Session(version, version == 5, "return"))
        {
            string escapedText = new JavaScriptSerializer().Serialize("\"rate\":1,\"rate\":2; {} [] \\ \n λ 🍕");
            string valid = baseline.Replace("\"hello\"", escapedText).Replace("0.5", "5e-1");
            session.Input.Send(valid);
            session.Output.Await(2, "synthesis_completed");
            session.Input.Send("{\"protocol_version\":" + version + ",\"request_id\":8,\"t\\u0079pe\":\"ping\"}");
            session.Output.Await(8, "pong");
            Check(session.Engine.SynthesisCalls == 1, "valid escaped input did not synthesize");
        }
        return cases + 1;
    }

    private static void NativeSpeak(Session session, int id, string text)
    {
        session.Send(id, "synthesize", "\"text\":\"" + text + "\",\"settings\":{" +
            "\"voice_id\":null,\"rate\":0.5,\"pitch\":1,\"volume\":1," +
            "\"pitch_range\":null,\"stress\":null,\"richness\":null},\"anchors\":[],\"voice_parameters\":{}");
    }

    private static int ParameterReceiptOrdering()
    {
        foreach (bool progressive in new[] { false, true })
        {
            using (Session session = new Session(6, progressive, "return"))
            {
                NativeSpeak(session, 2, "hold");
                Wait(session.Engine.Entered, "native preparation");
                Check(!session.Output.Has(2, "synthesis_started"), "receipt preceded native application");
                session.Send(7, "get_engine_parameters_v1", "\"engine_id\":\"test\",\"voice_id\":null," +
                    "\"cursor\":null,\"expected_catalogue_revision\":null");
                session.Output.Await(7, "engine_parameters_v1");
                session.Send(3, "cancel", "\"target_request_id\":2");
                session.Output.Await(3, "cancel_accepted");
                session.Engine.Release.Set();
                session.Output.Await(2, "synthesis_cancelled");
                session.CheckCancellation();
                Check(!session.Output.Has(2, "synthesis_started"), "cancelled native plan published receipt");
                session.FollowUp();
            }
            using (Session session = new Session(6, progressive, "return"))
            {
                NativeSpeak(session, 2, "next");
                session.Output.Await(2, "synthesis_completed");
                bool started = false;
                foreach (var frame in session.Output.Snapshot())
                {
                    if (Convert.ToInt32(frame["request_id"]) != 2) continue;
                    string type = (string)frame["type"];
                    if (type == "synthesis_started")
                    {
                        Check(!started && frame["native_application"] != null, "missing or duplicate receipt");
                        started = true;
                    }
                    else Check(started, "native output preceded receipt");
                }
            }
        }
        foreach (string mode in new[] { "early_audio", "omitted_receipt" })
        {
            using (Session session = new Session(6, mode == "early_audio", mode))
            {
                NativeSpeak(session, 2, "next");
                session.Output.Await(2, "error");
                Check(!session.Output.Has(2, "audio_chunk") && !session.Output.Has(2, "synthesis_started"),
                    "failed application leaked receipt or PCM");
                session.FollowUp();
            }
        }
        return 6;
    }

    public static int Main()
    {
        int cases = 0;
        try
        {
            for (int version = 1; version <= 5; ++version)
            {
                foreach (string mode in new[] { "return", "error", "callbacks", "stop_error" })
                {
                    CancelDuringNativeStop(version, version == 5, mode);
                    ++cases;
                }
                CancelWhileAcknowledgementBlocked(version, version == 5); ++cases;
                CompletedRequestRejectsCancel(version); ++cases;
                ShutdownActiveRequest(version); ++cases;
            }
            CancelDuringNativeStop(5, false, "return"); ++cases;
            CancelWhileAcknowledgementBlocked(5, false); ++cases;
            Console.WriteLine("PASS: {0} helper-6 native receipt ordering cases", ParameterReceiptOrdering());
            CancelDuringNativeStop(6, true, "callbacks"); ++cases;
            CancelWhileAcknowledgementBlocked(6, true); ++cases;
            Console.WriteLine("PASS: {0} deterministic Windows helper cancellation cases", cases);
            int wireCases = 0;
            for (int version = 1; version <= 5; version++) wireCases += WireValidation(version);
            Console.WriteLine("PASS: {0} Windows helper wire validation and recovery cases", wireCases);
            return 0;
        }
        catch (Exception error)
        {
            Console.Error.WriteLine("FAIL after {0} cases: {1}", cases, error);
            return 1;
        }
    }
}
