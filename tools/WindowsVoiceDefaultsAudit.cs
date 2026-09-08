// Test-only native parameter queries; no synthesis helper protocol extension.
// Run through check_windows_voice_defaults.ps1 on one x86 STA thread.
using System;
using System.Collections;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;

public static class NativeDefaultsAudit
{
    private const BindingFlags Hidden = BindingFlags.Public |
        BindingFlags.NonPublic | BindingFlags.Instance | BindingFlags.Static;

    [DllImport("kernel32", CharSet = CharSet.Unicode)]
    private static extern IntPtr GetModuleHandle(string path);
    [DllImport("kernel32", CharSet = CharSet.Ansi)]
    private static extern IntPtr GetProcAddress(IntPtr module, string name);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int EciGet(IntPtr handle, int voice, int parameter);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate uint DtGet(IntPtr handle, uint index, out IntPtr current,
        out IntPtr low, out IntPtr high, out IntPtr defaults);

    private static int[] Read(object capture, string dll, bool eci)
    {
        IntPtr handle = (IntPtr)capture.GetType().GetField("handle", Hidden)
            .GetValue(capture);
        // Query the exact module already loaded by the helper's validated loader.
        IntPtr module = GetModuleHandle(dll);
        string name = eci ? "eciGetVoiceParam" : "TextToSpeechGetSpeakerParams";
        IntPtr address = GetProcAddress(module, name);
        if (address == IntPtr.Zero)
            throw new Exception("Missing native query export: " + name);
        if (eci)
        {
            EciGet get = (EciGet)Marshal.GetDelegateForFunctionPointer(
                address, typeof(EciGet));
            int[] values = new int[8];
            for (int i = 0; i < values.Length; i++)
            {
                values[i] = get(handle, 0, i);
                if (values[i] < 0)
                    throw new Exception("Invalid ECI parameter");
            }
            return values;
        }

        DtGet read = (DtGet)Marshal.GetDelegateForFunctionPointer(
            address, typeof(DtGet));
        IntPtr current, low, high, defaults;
        uint status = read(handle, 0, out current, out low, out high, out defaults);
        if (status != 0)
            throw new Exception("DECtalk parameter query failed: " + status);
        try
        {
            // SPDEFS: smoothness, assertiveness, average pitch, pitch range,
            // richness, baseline fall, quickness, hat rise, stress rise.
            int[] indices = new int[] { 1, 2, 3, 4, 6, 26, 28, 29, 30 };
            int[] values = new int[indices.Length];
            for (int i = 0; i < values.Length; i++)
                values[i] = Marshal.ReadInt16(current, 2 * indices[i]);
            return values;
        }
        finally
        {
            Marshal.FreeCoTaskMem(current);
            Marshal.FreeCoTaskMem(low);
            Marshal.FreeCoTaskMem(high);
            Marshal.FreeCoTaskMem(defaults);
        }
    }

    public static void Run(string helper, string dll, bool eci)
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
            MethodInfo synth = type.GetMethod("Synthesize", Hidden);
            MethodInfo map = adapter.GetMethod("MapExtendedAcss", Hidden);
            Array anchors = Array.CreateInstance(
                assembly.GetType("OmnivoxHelperAnchor", true), 0);
            IDictionary voices = (IDictionary)adapter.GetField(
                eci ? "VoicePitchBaselines" : "VoiceAveragePitch", Hidden)
                .GetValue(null);
            IDictionary codes = eci ? null : (IDictionary)adapter.GetField(
                "VoiceCodes", Hidden).GetValue(null);
            if (voices.Count == 0)
                throw new Exception("No advertised voices to test");
            foreach (string voice in voices.Keys)
            {
                string baseline = null, tuned = null;
                for (int i = 0; i < 5; i++)
                {
                    bool custom = i == 1 || i == 3;
                    object[] patch = custom
                        ? new object[] { (double?)0.0, (double?)0.9, (double?)0.1 }
                        : new object[] { null, null, null };
                    string parameters = (string)map.Invoke(null, patch);
                    object volume = eci ? (object)100 : (object)1.0;
                    object[] args = new object[] {
                        "The quick brown fox jumps over the lazy dog.",
                        eci ? voice : (string)codes[voice],
                        custom ? (eci ? 100 : 220) : (eci ? 75 : 180),
                        custom ? (eci ? 85 : 170) : (int)voices[voice],
                        parameters, volume, anchors, new Func<bool>(() => false),
                        null // Buffer PCM in memory; never open an audio device.
                    };
                    object result = synth.Invoke(capture, args);
                    byte[] audio = (byte[])result.GetType().GetField("Audio", Hidden)
                        .GetValue(result);
                    if (audio.Length == 0)
                        throw new Exception("Empty native synthesis");
                    string values = String.Join(",", Read(capture, dll, eci));
                    Console.WriteLine(prefix + " " + voice + " " + i + " " +
                        (custom ? "set" : "default") + " " + values +
                        " pcm_bytes=" + audio.Length);
                    if (i == 0)
                        baseline = values;
                    else if (custom)
                    {
                        if (values == baseline)
                            throw new Exception("Tuning had no native effect");
                        if (tuned != null && tuned != values)
                            throw new Exception("Repeated tuning differs");
                        tuned = values;
                    }
                    else if (values != baseline)
                        throw new Exception("Default did not reset: " + values +
                            "; expected " + baseline);
                }
            }
        }
        finally
        {
            ((IDisposable)capture).Dispose();
        }
    }
}
