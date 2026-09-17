// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

// The protocol thread reads immutable metadata. It never switches a native
// preset to answer a query. Verified application/readback stays on the ECI owner.
internal sealed class OmnivoxEloquenceParameterService
{
    private readonly OmnivoxEloquenceAdapter engine;
    private readonly Dictionary<string, object> identity;
    private readonly object[] descriptors;
    private readonly object[] mappings;
    private readonly string dllPath;
    private readonly bool bindingsAvailable;
    private readonly object gate = new object();
    private readonly Dictionary<string, Dictionary<string, object>> plans =
        new Dictionary<string, Dictionary<string, object>>(StringComparer.Ordinal);
    private readonly Queue<string> planOrder = new Queue<string>();
    private int retainedBytes;
    private bool qualificationStarted, qualificationFinished;
    private string qualificationError;
    private readonly Stopwatch qualificationTime = new Stopwatch();

    internal OmnivoxEloquenceParameterService(OmnivoxEloquenceAdapter engine,
        string dllPath, bool bindingsAvailable)
    {
        this.engine = engine; this.dllPath = dllPath; this.bindingsAvailable = bindingsAvailable;
        List<object> rows = new List<object>();
        string[] labels = { "Gender", "Head size", "Pitch baseline", "Pitch fluctuation",
            "Roughness", "Breathiness", "Speed", "Volume" };
        for (int i = 0; i < 8; i++)
            rows.Add(OmnivoxParameterWire.Map("id", OmnivoxEloquenceParameters.Id(i),
                "label", labels[i], "help", "ECI voice parameter in native units. Default restores the selected preset.",
                "group", "voice", "order", i, "unit", "eci",
                "value_type", OmnivoxParameterWire.Map("kind", "integer", "minimum", 0,
                    "maximum", i == 0 ? 1 : i == 6 ? 250 : 100, "step", 1),
                "scope", "voice", "adjustable", true,
                "availability", OmnivoxParameterWire.Map("status", "supported", "reason", null),
                "default", OmnivoxParameterWire.Map("source", "unknown", "value", null, "reset_supported", true),
                "side_effects", new string[0]));
        descriptors = rows.ToArray();
        mappings = new object[] {
            Mapping(new string[] { "rate", "rate_offset" }, "speed"),
            Mapping(new string[] { "average_pitch" }, "pitch_baseline"),
            Mapping(new string[] { "pitch_range" }, "pitch_fluctuation"),
            Mapping(new string[] { "stress" }, "roughness"),
            Mapping(new string[] { "richness" }, "breathiness", "volume"),
            Mapping(new string[] { "volume" }, "volume")
        };
        string content = OmnivoxParameterWire.Canonical(OmnivoxParameterWire.Map(
            "schema_id", OmnivoxEloquenceParameters.SchemaId, "profile_id", OmnivoxEloquenceParameters.ProfileId,
            "parameters", descriptors, "mappings", mappings));
        string revision;
        using (SHA256 sha = SHA256.Create())
            revision = BitConverter.ToString(sha.ComputeHash(Encoding.UTF8.GetBytes(content))).Replace("-", "").ToLowerInvariant();
        byte[] random = new byte[8];
        using (RandomNumberGenerator rng = RandomNumberGenerator.Create()) rng.GetBytes(random);
        // Positive Int64 is also a valid u64 and round-trips through Framework JSON.
        long generation = (BitConverter.ToInt64(random, 0) & Int64.MaxValue) | 1;
        identity = OmnivoxParameterWire.Map("schema_id", OmnivoxEloquenceParameters.SchemaId,
            "profile_id", OmnivoxEloquenceParameters.ProfileId, "catalogue_revision", revision,
            "runtime_generation", generation);
    }

    private static object Mapping(string[] inputs, params string[] outputs)
    { return OmnivoxParameterWire.Map("common_inputs", inputs, "native_outputs", outputs); }

    // Only one bounded lookup is launched. A slow/unreadable file cannot block
    // protocol admission or monopolize the owner; after ten seconds it is unavailable.
    private string Qualification(out bool busy)
    {
        lock (gate)
        {
            busy = false;
            if (!bindingsAvailable) return "ECI parameter bindings or native units are unavailable";
            if (!qualificationStarted)
            {
                qualificationStarted = true;
                qualificationTime.Start();
                ThreadPool.QueueUserWorkItem(delegate(object unused) {
                    string error = null;
                    try { OmnivoxEloquenceParameters.RequireQualifiedRuntime(engine.Version, dllPath); }
                    catch (Exception) { error = "This ECI runtime has no qualified native parameter profile"; }
                    lock (gate) { qualificationError = error; qualificationFinished = true; }
                });
            }
            if (!qualificationFinished)
            {
                if (qualificationTime.ElapsedMilliseconds >= 10000) return "ECI parameter qualification timed out";
                busy = true; return null;
            }
            return qualificationError;
        }
    }

    private static Dictionary<string, object> Unavailable(string reason, string message)
    { return OmnivoxParameterWire.Map("status", "unavailable", "reason", reason, "message", message); }
    private static Dictionary<string, object> Busy()
    { return OmnivoxParameterWire.Map("status", "busy", "retry_after_ms", 50); }

    internal Dictionary<string, object> Catalogue(IDictionary<string, object> query)
    {
        string engineId = OmnivoxParameterWire.Id(query["engine_id"]);
        string voice = OmnivoxParameterWire.Text(query["voice_id"], 4096, true);
        string revision = OmnivoxParameterWire.Revision(query["expected_catalogue_revision"], true);
        string cursor = OmnivoxParameterWire.Text(query["cursor"], 128, true);
        if (cursor != null)
        {
            foreach (char c in cursor) if (c < 33 || c > 126) OmnivoxParameterWire.Invalid("cursor");
            if (revision == null) OmnivoxParameterWire.Invalid("cursor requires revision");
        }
        if (engineId != engine.EngineId) OmnivoxParameterWire.Invalid("engine is not hosted here");
        if (voice != null && !HasVoice(voice)) return Unavailable("voice_unavailable", "Voice is unavailable");
        // Eight descriptors fit one page. No continuation cursor is ever issued.
        if (cursor != null) OmnivoxParameterWire.Invalid("unknown catalogue cursor");
        if (revision != null && revision != (string)identity["catalogue_revision"])
            return Unavailable("not_described", "Catalogue revision changed; start a fresh query");
        bool busy;
        string error = Qualification(out busy);
        if (busy) return Busy();
        if (error != null) return Unavailable("not_described", error);
        return OmnivoxParameterWire.Map("status", "ready", "identity", identity,
            "voice_id", voice, "parameters", descriptors, "mappings", mappings, "next_cursor", null);
    }

    private bool HasVoice(string id)
    {
        foreach (OmnivoxHelperVoice voice in engine.Voices) if (voice.Id == id) return true;
        return false;
    }

    private sealed class Plan : IOmnivoxParameterSynthesis
    {
        internal OmnivoxEloquenceParameterService Service;
        internal OmnivoxParameterSettings Settings;
        internal OmnivoxEloquenceParameters Edits;
        internal Dictionary<string, int?> Values;
        internal string[] Context;
        internal string UnavailableReason;
        internal bool Busy;

        public OmnivoxCaptureResult Synthesize(string text, OmnivoxHelperAnchor[] anchors,
            Func<bool> cancelled, IOmnivoxCaptureSink sink, Action<Dictionary<string, object>> applied)
        {
            if (cancelled()) throw new OperationCanceledException();
            if (UnavailableReason != null)
            {
                applied(OmnivoxParameterWire.Map("status", "common_only", "plan_id", null,
                    "identity", null, "masked_parameters", new string[0], "reason", UnavailableReason));
                return Service.engine.Synthesize(text, Settings.Voice, Settings.Rate, Settings.Pitch,
                    Settings.PitchRange, Settings.Stress, Settings.Richness, Settings.Volume, anchors, cancelled, sink);
            }
            return Service.engine.SynthesizeWithParameters(text, Settings.Voice, Settings.Rate, Settings.Pitch,
                Settings.PitchRange, Settings.Stress, Settings.Richness, Settings.Volume, anchors, cancelled, sink,
                Values, Context, delegate(int[] actual) {
                    if (cancelled()) throw new OperationCanceledException();
                    string id = "eci-" + Guid.NewGuid().ToString("N");
                    Dictionary<string, object> explanation = Service.Explanation(this, actual, id);
                    Service.Retain(id, explanation);
                    try {
                        applied(OmnivoxParameterWire.Map("status", "applied", "plan_id", id,
                            "identity", Service.identity, "masked_parameters", Edits.MaskedParameters, "reason", null));
                    }
                    catch { Service.Forget(id); throw; }
                });
        }
    }

    internal IOmnivoxParameterSynthesis Prepare(IDictionary<string, object> settings, object parameters)
    {
        Plan plan = Parse(OmnivoxParameterSettings.Read(settings, engine), parameters);
        IDictionary<string, object> data = OmnivoxParameterWire.Object(parameters);
        if (plan.UnavailableReason != null && (string)data["unavailable_policy"] == "require")
            throw new OmnivoxParameterException(plan.Busy ? "busy" : "invalid_parameter",
                plan.UnavailableReason, plan.Busy);
        return plan;
    }

    private Plan Parse(OmnivoxParameterSettings settings, object parameters)
    {
        Plan plan = new Plan { Service = this, Settings = settings,
            Values = new Dictionary<string, int?>(StringComparer.Ordinal), Context = new string[0] };
        string mismatch = null;
        if (parameters != null)
        {
            OmnivoxParameterWire.Bounded(parameters);
            IDictionary<string, object> data = OmnivoxParameterWire.Object(parameters,
                "native", "context_dimensions", "expected_identity", "unavailable_policy");
            IDictionary<string, object> native = OmnivoxParameterWire.Object(data["native"],
                "engine_id", "schema_id", "parameters");
            string engineId = OmnivoxParameterWire.Id(native["engine_id"]);
            string schema = OmnivoxParameterWire.Id(native["schema_id"]);
            OmnivoxParameterWire.Identity(data["expected_identity"]);
            string policy = OmnivoxParameterWire.Text(data["unavailable_policy"], 32, false);
            if (policy != "require" && policy != "common_only") OmnivoxParameterWire.Invalid("unavailable policy");
            bool ours = engineId == engine.EngineId && schema == OmnivoxEloquenceParameters.SchemaId;
            IDictionary<string, object> operations = OmnivoxParameterWire.Object(native["parameters"]);
            if (operations.Count > 64) OmnivoxParameterWire.Invalid("too many native operations");
            foreach (KeyValuePair<string, object> entry in operations)
            {
                OmnivoxParameterWire.Id(entry.Key);
                IDictionary<string, object> operation = OmnivoxParameterWire.Object(entry.Value);
                if (!operation.ContainsKey("op")) OmnivoxParameterWire.Invalid("missing operation");
                string op = OmnivoxParameterWire.Text(operation["op"], 16, false);
                int? value = null;
                if (op == "set")
                {
                    OmnivoxParameterWire.Object(operation, "op", "value");
                    object scalar = operation["value"];
                    if (scalar is string) OmnivoxParameterWire.Id(scalar);
                    else if (!(scalar is bool)) OmnivoxParameterWire.Number(scalar);
                    if (ours) value = OmnivoxParameterWire.Integer(scalar);
                }
                else if (op == "default") OmnivoxParameterWire.Object(operation, "op");
                else OmnivoxParameterWire.Invalid("unknown operation");
                if (ours) plan.Values.Add(entry.Key, value);
            }
            object[] context = OmnivoxParameterWire.Array(data["context_dimensions"], 14);
            plan.Context = new string[context.Length];
            for (int i = 0; i < context.Length; i++) plan.Context[i] = OmnivoxParameterWire.Id(context[i]);
            if (!ours) mismatch = "Native engine or schema does not match this helper";
            else if (OmnivoxParameterWire.Canonical(data["expected_identity"]) != OmnivoxParameterWire.Canonical(identity))
                mismatch = "Native parameter catalogue identity is stale";
        }
        // Validate unknown IDs, ranges and context even when the identity is stale
        // or common_only would otherwise permit degradation.
        try { plan.Edits = new OmnivoxEloquenceParameters(plan.Values, plan.Context); }
        catch (ArgumentException) { OmnivoxParameterWire.Invalid("ECI parameter ID, value or contextual dimension"); }
        string error = Qualification(out plan.Busy);
        plan.UnavailableReason = mismatch ?? error ?? (plan.Busy ? "ECI parameter qualification is busy" : null);
        return plan;
    }

    internal Dictionary<string, object> Explain(object value)
    {
        IDictionary<string, object> source = OmnivoxParameterWire.Object(value);
        if (!source.ContainsKey("mode")) OmnivoxParameterWire.Invalid("missing explanation mode");
        string mode = OmnivoxParameterWire.Text(source["mode"], 16, false);
        if (mode == "applied")
        {
            OmnivoxParameterWire.Object(source, "mode", "plan_id");
            string id = OmnivoxParameterWire.Id(source["plan_id"]);
            lock (gate)
            {
                Dictionary<string, object> result;
                return plans.TryGetValue(id, out result) ? result : Unavailable("plan_expired", "Applied plan is no longer retained");
            }
        }
        if (mode != "draft") OmnivoxParameterWire.Invalid("unknown explanation mode");
        OmnivoxParameterWire.Object(source, "mode", "settings", "voice_parameters");
        OmnivoxParameterSettings settings;
        try { settings = OmnivoxParameterSettings.Read(source["settings"], engine); }
        catch (OmnivoxParameterException e) {
            if (e.Code == "voice_not_found") return Unavailable("voice_unavailable", "Voice is unavailable");
            throw;
        }
        Plan plan = Parse(settings, source["voice_parameters"]);
        if (plan.Busy) return Busy();
        if (plan.UnavailableReason != null) return Unavailable("native_unavailable", plan.UnavailableReason);
        return Explanation(plan, null, null);
    }

    private Dictionary<string, object> Explanation(Plan plan, int[] actual, string id)
    {
        OmnivoxParameterSettings s = plan.Settings;
        int?[] common = OmnivoxEloquenceAdapter.MapCommonParameters(s.Voice, s.Rate, s.Pitch,
            s.PitchRange, s.Stress, s.Richness, s.Volume);
        return OmnivoxParameterWire.Map("status", "ready", "evidence", actual == null ? "planned" : "adapter_applied",
            "plan_id", id, "realized", OmnivoxParameterWire.Map("engine_id", engine.EngineId, "voice_id", s.Voice),
            "identity", identity, "parameters", plan.Edits.Explain(common, actual));
    }

    private void Retain(string id, Dictionary<string, object> explanation)
    {
        int bytes = Size(explanation);
        if (bytes > 256 * 1024) throw new InvalidOperationException("Applied plan exceeds retention bound");
        lock (gate)
        {
            while (planOrder.Count >= 64 || retainedBytes + bytes > 256 * 1024)
            {
                string expired = planOrder.Dequeue();
                Dictionary<string, object> old;
                if (plans.TryGetValue(expired, out old)) { retainedBytes -= Size(old); plans.Remove(expired); }
            }
            plans.Add(id, explanation); planOrder.Enqueue(id); retainedBytes += bytes;
        }
    }
    private void Forget(string id)
    {
        lock (gate)
        {
            Dictionary<string, object> old;
            if (plans.TryGetValue(id, out old)) { retainedBytes -= Size(old); plans.Remove(id); }
        }
    }
    private static int Size(object value)
    { return Encoding.UTF8.GetByteCount(new JavaScriptSerializer().Serialize(value)); }
}
