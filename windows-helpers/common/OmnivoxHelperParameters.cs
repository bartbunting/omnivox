// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Text;
using System.Web.Script.Serialization;

// Optional helper-6 boundary. Engines without this interface retain versions 1-5.
internal interface IOmnivoxParameterEngine
{
    Dictionary<string, object> GetParameters(IDictionary<string, object> query);
    Dictionary<string, object> ExplainParameters(object source);
    IOmnivoxParameterSynthesis PrepareParameters(IDictionary<string, object> settings, object parameters);
}

internal interface IOmnivoxParameterSynthesis
{
    OmnivoxCaptureResult Synthesize(string text, OmnivoxHelperAnchor[] anchors,
        Func<bool> cancelled, IOmnivoxCaptureSink sink,
        Action<Dictionary<string, object>> applied);
}

internal sealed class OmnivoxParameterException : Exception
{
    internal readonly string Code;
    internal readonly bool Retryable;
    internal OmnivoxParameterException(string code, string message, bool retryable)
        : base(message) { Code = code; Retryable = retryable; }
}

internal sealed class OmnivoxParameterSettings
{
    internal string Voice;
    internal double Rate, Pitch, Volume;
    internal double? PitchRange, Stress, Richness;

    internal static OmnivoxParameterSettings Read(object value, IOmnivoxCaptureEngine engine)
    {
        IDictionary<string, object> s = Normalize(value);
        string voice = OmnivoxParameterWire.Text(s["voice_id"], 4096, true) ?? engine.DefaultVoiceId;
        bool found = false;
        foreach (OmnivoxHelperVoice item in engine.Voices) if (item.Id == voice) found = true;
        if (!found) throw new OmnivoxParameterException("voice_not_found", "Voice is unavailable", false);
        return new OmnivoxParameterSettings {
            Voice = voice, Rate = Number(s["rate"], 0, 2), Pitch = Number(s["pitch"], 0.5, 2),
            Volume = Number(s["volume"], 0, 1), PitchRange = Optional(s["pitch_range"]),
            Stress = Optional(s["stress"]), Richness = Optional(s["richness"])
        };
    }
    internal static IDictionary<string, object> Normalize(object value)
    {
        Dictionary<string, object> s = new Dictionary<string, object>(OmnivoxParameterWire.Object(value));
        foreach (string field in new[] { "voice_id", "pitch_range", "stress", "richness" })
            if (!s.ContainsKey(field)) s.Add(field, null);
        OmnivoxParameterWire.Object(s, "voice_id", "rate", "pitch", "volume", "pitch_range", "stress", "richness");
        return s;
    }
    private static double? Optional(object value) { return value == null ? (double?)null : Number(value, 0, 1); }
    private static double Number(object value, double minimum, double maximum)
    {
        double number = OmnivoxParameterWire.Number(value);
        if (number < minimum || number > maximum) OmnivoxParameterWire.Invalid("common parameter range");
        return number;
    }
}

internal static class OmnivoxParameterWire
{
    internal static Dictionary<string, object> Map(params object[] pairs)
    {
        Dictionary<string, object> result = new Dictionary<string, object>(StringComparer.Ordinal);
        for (int i = 0; i < pairs.Length; i += 2) result.Add((string)pairs[i], pairs[i + 1]);
        return result;
    }
    internal static void Invalid(string detail)
    {
        throw new OmnivoxParameterException("invalid_parameter", "Invalid parameter data: " + detail, false);
    }
    internal static IDictionary<string, object> Object(object value, params string[] fields)
    {
        IDictionary<string, object> map = value as IDictionary<string, object>;
        if (map == null) Invalid("object required");
        if (fields.Length != 0)
        {
            if (map.Count != fields.Length) Invalid("unknown or missing field");
            foreach (string field in fields) if (!map.ContainsKey(field)) Invalid("missing " + field);
        }
        return map;
    }
    internal static object[] Array(object value, int maximum)
    {
        object[] array = value as object[];
        if (array == null || array.Length > maximum) Invalid("bounded array required");
        return array;
    }
    internal static string Text(object value, int maximum, bool nullable)
    {
        if (value == null && nullable) return null;
        string text = value as string;
        if (String.IsNullOrEmpty(text) || Encoding.UTF8.GetByteCount(text) > maximum) Invalid("bounded text required");
        for (int i = 0; i < text.Length; i++)
        {
            char c = text[i];
            if (Char.IsControl(c)) Invalid("control character");
            if (Char.IsHighSurrogate(c))
            {
                if (++i >= text.Length || !Char.IsLowSurrogate(text[i])) Invalid("unpaired surrogate");
            }
            else if (Char.IsLowSurrogate(c)) Invalid("unpaired surrogate");
        }
        return text;
    }
    internal static string Id(object value)
    {
        string id = Text(value, 128, false);
        foreach (char c in id)
            if (!(c >= 'a' && c <= 'z') && !(c >= 'A' && c <= 'Z') &&
                !(c >= '0' && c <= '9') && c != '_' && c != '.' && c != '-') Invalid("identifier");
        return id;
    }
    internal static string Revision(object value, bool nullable)
    {
        string revision = Text(value, 64, nullable);
        if (revision == null) return null;
        if (revision.Length != 64) Invalid("revision");
        foreach (char c in revision) if (!(c >= '0' && c <= '9') && !(c >= 'a' && c <= 'f')) Invalid("revision");
        return revision;
    }
    internal static double Number(object value)
    {
        if (!(value is int || value is long || value is decimal || value is double)) Invalid("number required");
        double number = Convert.ToDouble(value, CultureInfo.InvariantCulture);
        if (Double.IsNaN(number) || Double.IsInfinity(number)) Invalid("finite number required");
        return number;
    }
    internal static int Integer(object value)
    {
        if (!(value is int || value is long)) Invalid("integer required");
        long number = Convert.ToInt64(value, CultureInfo.InvariantCulture);
        if (number < Int32.MinValue || number > Int32.MaxValue) Invalid("integer range");
        return (int)number;
    }
    internal static void Identity(object value)
    {
        IDictionary<string, object> identity = Object(value,
            "schema_id", "profile_id", "catalogue_revision", "runtime_generation");
        Id(identity["schema_id"]); Id(identity["profile_id"]); Revision(identity["catalogue_revision"], false);
        object generation = identity["runtime_generation"];
        // JavaScriptSerializer represents unsigned values above Int64 as Decimal.
        if (!(generation is int || generation is long || generation is decimal)) Invalid("runtime generation");
        decimal number = Convert.ToDecimal(generation, CultureInfo.InvariantCulture);
        if ((generation is decimal && number <= Int64.MaxValue) || number <= 0 || number > UInt64.MaxValue || Decimal.Truncate(number) != number) Invalid("runtime generation");
    }
    internal static void Bounded(object value)
    {
        if (Encoding.UTF8.GetByteCount(new JavaScriptSerializer().Serialize(value)) > 256 * 1024)
            throw new OmnivoxParameterException("payload_too_large", "Parameter metadata exceeds 256 KiB", false);
    }
    // Explicit key ordering makes descriptor revisions independent of dictionary iteration.
    internal static string Canonical(object value)
    {
        IDictionary<string, object> map = value as IDictionary<string, object>;
        JavaScriptSerializer json = new JavaScriptSerializer();
        if (map != null)
        {
            List<string> keys = new List<string>(map.Keys);
            keys.Sort(StringComparer.Ordinal);
            List<string> entries = new List<string>();
            foreach (string key in keys) entries.Add(json.Serialize(key) + ":" + Canonical(map[key]));
            return "{" + String.Join(",", entries.ToArray()) + "}";
        }
        IEnumerable sequence = value as IEnumerable;
        if (sequence != null && !(value is string))
        {
            List<string> entries = new List<string>();
            foreach (object item in sequence) entries.Add(Canonical(item));
            return "[" + String.Join(",", entries.ToArray()) + "]";
        }
        return json.Serialize(value);
    }
}
