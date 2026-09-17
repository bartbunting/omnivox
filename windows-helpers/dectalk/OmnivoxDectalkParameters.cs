// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Security.Cryptography;
using System.Text;

// Immutable typed edits; vendor commands are constructed only inside the helper.
internal sealed class OmnivoxDectalkParameters
{
    internal const string SchemaId = "dectalk.design-voice.v1";
    internal const string ProfileId = "dectalk.windows.4_99.v1";
    private static readonly string[] Ids = {
        "sx", "sm", "as", "ap", "pr", "br", "ri",
        "nf", "la", "hs", "f4", "b4", "f5", "b5",
        "gf", "gh", "gv", "gn", "g1", "g2", "g3",
        "g4", "g5", "bf", "lx", "qu", "hr", "sr"
    };
    private static readonly int[] Minimum = {
        0, 0, 0, 50, 0, 0, 0,
        0, 0, 65, 2000, 100, 2500, 100,
        0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 2, 1
    };
    private static readonly int[] Maximum = {
        1, 100, 200, 350, 250, 72, 100,
        100, 100, 145, 6000, 6000, 6000, 6000,
        87, 87, 87, 87, 87, 87, 87,
        87, 87, 90, 100, 100, 100, 100
    };
    private static readonly string[] ContextDimensions = {
        "rate", "rate_offset", "average_pitch", "pitch_range", "stress",
        "richness", "volume", "gain", "low_pass", "high_pass", "pan",
        "reverb", "echo", "chorus"
    };
    private readonly bool[] selected = new bool[28];
    private readonly int?[] values = new int?[28];
    private readonly bool[] contextual = new bool[28];

    internal OmnivoxDectalkParameters(IDictionary<string, int?> parameters,
        IEnumerable<string> contextDimensions)
    {
        if (parameters == null || contextDimensions == null)
            throw new ArgumentNullException("parameters/contextDimensions");
        if (parameters.Count > Ids.Length)
            throw new ArgumentException("Too many DECtalk parameters");
        foreach (KeyValuePair<string, int?> entry in parameters)
        {
            int index = Array.IndexOf(Ids, entry.Key);
            if (index < 0) throw new ArgumentException("Unknown DECtalk parameter: " + entry.Key);
            if (entry.Value.HasValue) ValidateValue(index, entry.Value.Value);
            selected[index] = true;
            values[index] = entry.Value;
        }
        HashSet<string> seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (string dimension in contextDimensions)
        {
            if (Array.IndexOf(ContextDimensions, dimension) < 0 || !seen.Add(dimension))
                throw new ArgumentException("Unknown or duplicate contextual dimension");
            switch (dimension)
            {
                case "average_pitch": contextual[3] = true; break;
                case "pitch_range": contextual[2] = contextual[4] = true; break;
                case "stress": contextual[23] = contextual[25] = contextual[26] = contextual[27] = true; break;
                case "richness": contextual[1] = contextual[6] = true; break;
            }
        }
    }

    internal int[] Compose(int[] pristine, int[] common)
    {
        if (pristine == null || common == null || pristine.Length != 28 || common.Length != 28)
            throw new ArgumentException("Incomplete DECtalk state");
        int[] result = (int[])common.Clone();
        for (int i = 0; i < result.Length; i++)
        {
            ValidateValue(i, pristine[i]);
            ValidateValue(i, common[i]);
            if (selected[i] && !contextual[i]) result[i] = values[i] ?? pristine[i];
        }
        return result;
    }

    internal string Commands(int[] plan)
    {
        if (plan == null || plan.Length != 28) throw new ArgumentException("Incomplete DECtalk plan");
        StringBuilder result = new StringBuilder("[:dv");
        bool any = false;
        for (int i = 0; i < plan.Length; i++)
        {
            ValidateValue(i, plan[i]);
            if (!selected[i] || contextual[i]) continue;
            any = true;
            result.Append(" ").Append(Ids[i]).Append(" ").Append(plan[i].ToString(CultureInfo.InvariantCulture));
        }
        return any ? result.Append("]").ToString() : "";
    }

    internal string[] MaskedParameters
    {
        get
        {
            List<string> result = new List<string>();
            for (int i = 0; i < Ids.Length; i++)
                if (selected[i] && contextual[i]) result.Add(Ids[i]);
            return result.ToArray();
        }
    }

    internal const int Count = 28;

    // Labels/units follow DECtalk's SPDEFS and define_options tables. Limits
    // belong to this qualified runtime, not an inferred scale in the UI.
    internal static Dictionary<string, object> Descriptor(int index)
    {
        string[] labels = {
            "Sex", "Smoothness", "Assertiveness", "Average pitch", "Pitch range",
            "Breathiness", "Richness", "Fixed open-glottis samples", "Laryngealization",
            "Head size", "Fourth formant frequency", "Fourth formant bandwidth",
            "Fifth formant frequency", "Fifth formant bandwidth", "Frication gain",
            "Aspiration gain", "Voicing gain", "Nasalization gain", "Cascade gain G1",
            "Cascade gain G2", "Cascade gain G3", "Cascade gain G4", "Loudness",
            "Baseline fall", "Lax breathiness", "Quickness", "Hat rise", "Stress rise"
        };
        string[] units = {
            null, "percent", "percent", "hertz", "percent", "decibels", "percent",
            "samples", "percent", "percent", "hertz", "hertz", "hertz", "hertz",
            "decibels", "decibels", "decibels", "decibels", "decibels", "decibels",
            "decibels", "decibels", "decibels", "hertz", "percent", "percent", "hertz", "hertz"
        };
        string help = index == 0 ? "Voice sex: 0 female, 1 male. Default restores the selected preset." :
            "Native DECtalk " + Ids[index] + " control. Default restores the selected preset.";
        return OmnivoxParameterWire.Map("id", Ids[index], "label", labels[index],
            "help", help, "group", "voice", "order", index, "unit", units[index],
            "value_type", OmnivoxParameterWire.Map("kind", "integer", "minimum", Minimum[index],
                "maximum", Maximum[index], "step", 1), "scope", "voice", "adjustable", true,
            "availability", OmnivoxParameterWire.Map("status", "supported", "reason", null),
            "default", OmnivoxParameterWire.Map("source", "unknown", "value", null, "reset_supported", true),
            "side_effects", new string[0]);
    }

    internal object[] Explain(int?[] common, int[] actual)
    {
        if (common == null || common.Length != Count || (actual != null && actual.Length != Count))
            throw new ArgumentException("Incomplete DECtalk explanation");
        object[] rows = new object[Count];
        for (int i = 0; i < Count; i++)
        {
            int? value = common[i];
            string origin = value.HasValue ? (contextual[i] ? "context_mapping" : "common_mapping") : "engine_default";
            if (selected[i] && !contextual[i])
            {
                value = values[i];
                origin = value.HasValue ? "native_set" : "native_default";
            }
            rows[i] = OmnivoxParameterWire.Map("id", Ids[i],
                "value", actual == null ? (object)value : actual[i], "origin", origin,
                "masked_native", selected[i] && contextual[i], "read_back", actual != null);
        }
        return rows;
    }

    internal static void ValidateValue(int index, int value)
    {
        if (index < 0 || index >= Ids.Length) throw new ArgumentOutOfRangeException("index");
        if (value < Minimum[index] || value > Maximum[index]) throw new ArgumentOutOfRangeException(Ids[index]);
    }

    internal static void RequireReadback(int[] expected, int[] actual)
    {
        if (expected == null || actual == null || expected.Length != 28 || actual.Length != 28)
            throw new InvalidOperationException("Incomplete DECtalk readback");
        for (int i = 0; i < expected.Length; i++)
            if (expected[i] != actual[i]) throw new InvalidOperationException("DECtalk readback differs for " + Ids[i]);
    }

    internal static void RequireLimits(int[][] rows)
    {
        if (rows == null || rows.Length != 4) throw new InvalidOperationException("Incomplete DECtalk limits");
        RequireReadback(Minimum, rows[1]);
        RequireReadback(Maximum, rows[2]);
    }

    internal static void RequireQualifiedRuntime(string version, string path)
    {
        if (version != "v4.99 Github NORMAL ACCESS32 ")
            throw new NotSupportedException("This DECtalk runtime has no qualified native parameter profile");
        string digest;
        using (FileStream input = File.OpenRead(path))
        using (SHA256 sha = SHA256.Create())
            digest = BitConverter.ToString(sha.ComputeHash(input)).Replace("-", "").ToLowerInvariant();
        if (digest != "af25879d858846aaaa80b8f9626b1cbf4e57a1ab0467e8e2d82990558033c852")
            throw new NotSupportedException("This DECtalk runtime has no qualified native parameter profile");
    }
}
