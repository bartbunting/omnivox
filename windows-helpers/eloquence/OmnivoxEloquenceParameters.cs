// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.IO;
using System.Security.Cryptography;

// Engine-owned, immutable edits in ECI units. Null means the selected preset's
// pristine value; an absent key inherits the common mapping. Wire decoding and
// catalogue/receipt publication remain separate from this native execution layer.
internal sealed class OmnivoxEloquenceParameters
{
    internal const string SchemaId = "eloquence.eci-units.v1";
    internal const string ProfileId = "eloquence.windows.6_1.en_us.v1";
    private const string QualifiedDllSha256 =
        "da99080288cdca14a7effba20274af1d6d5878840e32be5a315bd8691124703b";
    private static readonly string[] Ids = {
        "gender", "head_size", "pitch_baseline", "pitch_fluctuation",
        "roughness", "breathiness", "speed", "volume"
    };
    private static readonly string[] ContextDimensions = {
        "rate", "rate_offset", "average_pitch", "pitch_range", "stress",
        "richness", "volume", "gain", "low_pass", "high_pass", "pan",
        "reverb", "echo", "chorus"
    };
    private readonly bool[] selected = new bool[8];
    private readonly int?[] values = new int?[8];
    private readonly bool[] contextual = new bool[8];

    internal OmnivoxEloquenceParameters(IDictionary<string, int?> parameters,
        IEnumerable<string> contextDimensions)
    {
        if (parameters == null || contextDimensions == null)
            throw new ArgumentNullException("parameters/contextDimensions");
        if (parameters.Count > Ids.Length)
            throw new ArgumentException("Too many ECI voice parameters");
        // Validate every requested value, including values that context masks.
        foreach (KeyValuePair<string, int?> entry in parameters)
        {
            int index = Array.IndexOf(Ids, entry.Key);
            if (index < 0) throw new ArgumentException("Unknown ECI parameter: " + entry.Key);
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
                case "rate":
                case "rate_offset": contextual[6] = true; break;
                case "average_pitch": contextual[2] = true; break;
                case "pitch_range": contextual[3] = true; break;
                case "stress": contextual[4] = true; break;
                case "richness": contextual[5] = contextual[7] = true; break;
                case "volume": contextual[7] = true; break;
            }
        }
    }

    internal int[] Compose(int[] pristine, int?[] common)
    {
        if (pristine == null || pristine.Length != 8 || common == null || common.Length != 8)
            throw new ArgumentException("Incomplete ECI voice state");
        int[] result = (int[])pristine.Clone();
        for (int index = 0; index < result.Length; index++)
        {
            ValidateValue(index, pristine[index]);
            if (common[index].HasValue)
            {
                ValidateValue(index, common[index].Value);
                result[index] = common[index].Value;
            }
            if (selected[index] && !contextual[index])
                result[index] = values[index] ?? pristine[index];
        }
        return result;
    }

    internal string[] MaskedParameters
    {
        get
        {
            List<string> masked = new List<string>();
            for (int index = 0; index < Ids.Length; index++)
                if (selected[index] && contextual[index]) masked.Add(Ids[index]);
            return masked.ToArray();
        }
    }

    internal static void ValidateValue(int index, int value)
    {
        if (index < 0 || index >= 8)
            throw new ArgumentOutOfRangeException("index");
        int maximum = index == 0 ? 1 : index == 6 ? 250 : 100;
        if (value < 0 || value > maximum)
            throw new ArgumentOutOfRangeException(Ids[index]);
    }

    internal static void RequireReadback(int[] expected, int[] actual)
    {
        if (actual == null || actual.Length != expected.Length)
            throw new InvalidOperationException("Incomplete ECI voice readback");
        for (int index = 0; index < expected.Length; index++)
            if (actual[index] != expected[index])
                throw new InvalidOperationException("ECI voice readback differs for " + Ids[index]);
    }

    internal static void RequireQualifiedRuntime(string version, string dllPath)
    {
        if (version != "6.1.0.0")
            throw new NotSupportedException("This ECI runtime has no qualified native parameter profile");
        string digest;
        using (FileStream input = File.OpenRead(dllPath))
        using (SHA256 sha = SHA256.Create())
            digest = BitConverter.ToString(sha.ComputeHash(input)).Replace("-", "").ToLowerInvariant();
        if (digest != QualifiedDllSha256)
            throw new NotSupportedException("This ECI runtime has no qualified native parameter profile");
    }
}
