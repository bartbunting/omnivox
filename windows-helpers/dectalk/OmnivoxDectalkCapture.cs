// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

internal sealed class OmnivoxNativeDectalk : IDisposable
{
    [DllImport("user32.dll", CharSet = CharSet.Ansi, SetLastError = true,
        EntryPoint = "RegisterWindowMessageA")]
    private static extern uint RegisterWindowMessageNative(string message);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    internal delegate void Callback(int parameter1, int parameter2,
        uint userParameter, uint message);

    [UnmanagedFunctionPointer(CallingConvention.Cdecl, CharSet = CharSet.Ansi)]
    private delegate uint StartupFunction(out IntPtr handle,
        uint device, uint options, Callback callback, int instanceParameter,
        string dictionaryPath);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint HandleFunction(IntPtr handle);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint SpeakFunction(IntPtr handle, IntPtr text,
        uint flags);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint ResetFunction(IntPtr handle,
        [MarshalAs(UnmanagedType.Bool)] bool resetModes);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint HandleValueFunction(IntPtr handle, uint value);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint AddBufferFunction(IntPtr handle, IntPtr buffer);
    [UnmanagedFunctionPointer(CallingConvention.Cdecl)]
    private delegate uint VersionFunction(out IntPtr version);

    private readonly OmnivoxNativeLibrary library;
    private readonly StartupFunction startup;
    private readonly HandleFunction shutdown;
    private readonly SpeakFunction speak;
    private readonly ResetFunction reset;
    private readonly HandleFunction sync;
    private readonly HandleValueFunction setRate;
    private readonly HandleValueFunction openInMemory;
    private readonly HandleFunction closeInMemory;
    private readonly AddBufferFunction addBuffer;
    private readonly VersionFunction version;

    internal OmnivoxNativeDectalk(string path)
    {
        string[] requiredExports = new string[]
        {
            "TextToSpeechStartupExFonix", "TextToSpeechShutdown",
            "TextToSpeechSpeak", "TextToSpeechReset", "TextToSpeechSync",
            "TextToSpeechSetRate", "TextToSpeechOpenInMemory",
            "TextToSpeechCloseInMemory", "TextToSpeechAddBuffer",
            "TextToSpeechVersion"
        };
        library = new OmnivoxNativeLibrary(path, "DECtalk.dll",
            requiredExports);
        try
        {
            startup = library.Resolve<StartupFunction>(
                "TextToSpeechStartupExFonix");
            shutdown = library.Resolve<HandleFunction>(
                "TextToSpeechShutdown");
            speak = library.Resolve<SpeakFunction>("TextToSpeechSpeak");
            reset = library.Resolve<ResetFunction>("TextToSpeechReset");
            sync = library.Resolve<HandleFunction>("TextToSpeechSync");
            setRate = library.Resolve<HandleValueFunction>(
                "TextToSpeechSetRate");
            openInMemory = library.Resolve<HandleValueFunction>(
                "TextToSpeechOpenInMemory");
            closeInMemory = library.Resolve<HandleFunction>(
                "TextToSpeechCloseInMemory");
            addBuffer = library.Resolve<AddBufferFunction>(
                "TextToSpeechAddBuffer");
            version = library.Resolve<VersionFunction>("TextToSpeechVersion");
        }
        catch
        {
            library.Dispose();
            throw;
        }
    }

    internal static uint RegisterWindowMessage(string message)
    {
        return RegisterWindowMessageNative(message);
    }

    internal uint TextToSpeechStartupExFonix(out IntPtr handle, uint device,
        uint options, Callback callback, int instanceParameter,
        string dictionaryPath)
    {
        return startup(out handle, device, options, callback,
            instanceParameter, dictionaryPath);
    }

    internal uint TextToSpeechShutdown(IntPtr handle)
    {
        return shutdown(handle);
    }

    internal uint TextToSpeechSpeak(IntPtr handle, IntPtr text, uint flags)
    {
        return speak(handle, text, flags);
    }

    internal uint TextToSpeechReset(IntPtr handle, bool resetModes)
    {
        return reset(handle, resetModes);
    }

    internal uint TextToSpeechSync(IntPtr handle) { return sync(handle); }
    internal uint TextToSpeechSetRate(IntPtr handle, uint rate)
    {
        return setRate(handle, rate);
    }
    internal uint TextToSpeechOpenInMemory(IntPtr handle, uint format)
    {
        return openInMemory(handle, format);
    }
    internal uint TextToSpeechCloseInMemory(IntPtr handle)
    {
        return closeInMemory(handle);
    }
    internal uint TextToSpeechAddBuffer(IntPtr handle, IntPtr buffer)
    {
        return addBuffer(handle, buffer);
    }
    internal uint TextToSpeechVersion(out IntPtr value)
    {
        return version(out value);
    }

    public void Dispose()
    {
        library.Dispose();
    }
}

[StructLayout(LayoutKind.Sequential)]
internal struct OmnivoxDectalkBuffer
{
    internal IntPtr Data;
    internal IntPtr PhonemeArray;
    internal IntPtr IndexArray;
    internal uint MaximumBufferLength;
    internal uint MaximumPhonemeChanges;
    internal uint MaximumIndexMarks;
    internal uint BufferLength;
    internal uint NumberOfPhonemeChanges;
    internal uint NumberOfIndexMarks;
    internal uint Reserved;
}

[StructLayout(LayoutKind.Sequential)]
internal struct OmnivoxDectalkPhoneme
{
    internal uint Phoneme;
    internal uint SampleNumber;
    internal uint Duration;
    internal uint Reserved;
}

[StructLayout(LayoutKind.Sequential)]
internal struct OmnivoxDectalkIndex
{
    internal uint Value;
    internal uint SampleNumber;
    internal uint Reserved;
}

/// <summary>
/// Owns one 32-bit DECtalk instance and captures its in-memory PCM buffers.
/// Native phoneme and index records use DECtalk's utterance-relative sample
/// counter and are returned with the captured PCM.
/// </summary>
internal sealed class OmnivoxDectalkCapture : IDisposable
{
    private sealed class MarkerInsertion
    {
        internal int Position;
        internal int Priority;
        internal int Sequence;
        internal uint IndexValue;
        internal OmnivoxHelperMarker Marker;
    }

    private sealed class BufferSlot
    {
        internal IntPtr Data;
        internal IntPtr Phonemes;
        internal IntPtr Indexes;
        internal IntPtr Buffer;
    }

    private const uint WaveMapper = 0xffffffff;
    private const uint DoNotUseAudioDevice = 0x80000000;
    private const uint WaveFormat11025Mono16 = 0x00000004;
    private const uint TtsForce = 1;
    private const int BufferSamples = 512;
    private const int BufferBytes = BufferSamples * 2;
    private const int BufferCount = 4;
    private const int MarkerRecordsPerBuffer = 512;
    private const int MaximumMarkers = 4096;
    private const int FirstPrivateMarkerIndex = 28672;
    private const int LastPrivateMarkerIndex = 32767;
    private const int MaximumMarkerValueBytes = 16 * 1024;
    private const int MaximumAudioBytes = 128 * 1024 * 1024;
    internal const int SpeechSampleRate = 11025;

    private readonly object synthesisLock = new object();
    private readonly object stateLock = new object();
    private readonly Encoding textEncoding;
    private readonly List<BufferSlot> buffers = new List<BufferSlot>();
    private OmnivoxNativeDectalk native;
    private IntPtr handle;
    private OmnivoxNativeDectalk.Callback callback;
    private uint bufferMessage;
    private MemoryStream capture;
    private List<OmnivoxHelperMarker> markers;
    private Dictionary<uint, OmnivoxHelperMarker> pendingTextMarkers;
    private Dictionary<uint, List<OmnivoxHelperMarker>> pendingAnchorMarkers;
    private List<OmnivoxHelperMarker> leadingAnchorMarkers;
    private List<OmnivoxHelperMarker> trailingAnchorMarkers;
    private Exception callbackError;
    private bool discardAudio;
    private bool nativeSynthesisActive;
    private bool shuttingDown;
    private bool memoryOpen;
    private string runtimeVersion;
    private IOmnivoxCaptureSink progressiveSink;
    private ulong capturedFrames;
    private double outputVolume;
    private byte[] pendingProgressiveAudio;

    internal OmnivoxDectalkCapture(string dllPath)
    {
        native = new OmnivoxNativeDectalk(dllPath);
        dllPath = Path.GetFullPath(dllPath);
        string directory = Path.GetDirectoryName(dllPath);
        string dictionary = Path.Combine(directory, "dtalk_us.dic");
        if (!File.Exists(dictionary))
        {
            native.Dispose();
            native = null;
            throw new OmnivoxRuntimeUnavailableException(
                "DECtalk dictionary was not found at \"" + dictionary + "\"");
        }

        try
        {
            textEncoding = Encoding.GetEncoding(28591,
                EncoderFallback.ExceptionFallback,
                DecoderFallback.ExceptionFallback);
            bufferMessage = OmnivoxNativeDectalk.RegisterWindowMessage(
                "DECtalkBufferMessage");
            if (bufferMessage == 0)
            {
                throw new Win32Exception(Marshal.GetLastWin32Error(),
                    "Could not register the DECtalk buffer message");
            }

            callback = OnDectalkCallback;
            Check(native.TextToSpeechStartupExFonix(
                out handle, WaveMapper, DoNotUseAudioDevice, callback, 0,
                dictionary), "TextToSpeechStartupExFonix");
            if (handle == IntPtr.Zero)
            {
                throw new InvalidOperationException(
                    "DECtalk returned a null speech handle");
            }
            Check(native.TextToSpeechOpenInMemory(handle,
                WaveFormat11025Mono16), "TextToSpeechOpenInMemory");
            memoryOpen = true;
            AllocateBuffers();

            IntPtr versionValue;
            uint versionCode = native.TextToSpeechVersion(out versionValue);
            runtimeVersion = versionValue == IntPtr.Zero ? null :
                Marshal.PtrToStringAnsi(versionValue);
            if (versionCode == 0 || String.IsNullOrEmpty(runtimeVersion))
            {
                throw new OmnivoxRuntimeUnavailableException(
                    "DECtalk.dll did not report a runtime version");
            }
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    internal string Version
    {
        get
        {
            return runtimeVersion;
        }
    }

    internal OmnivoxCaptureResult Synthesize(string text, string voiceCode,
        int rate, int pitch, string voiceParameters, double volume,
        OmnivoxHelperAnchor[] anchors,
        Func<bool> cancellationRequested, IOmnivoxCaptureSink sink)
    {
        lock (synthesisLock)
        {
            ThrowIfCancellationRequested(cancellationRequested);
            BeginCapture(sink, volume);
            try
            {
                ThrowIfCancellationRequested(cancellationRequested);
                Check(native.TextToSpeechSetRate(handle,
                    (uint)rate), "TextToSpeechSetRate");
                ThrowIfCancellationRequested(cancellationRequested);
                string indexedText = BuildTextWithIndexes(text,
                    sink == null ? new OmnivoxHelperAnchor[0] : anchors);
                EmitProgressiveAnchorMarkers(leadingAnchorMarkers, 0);
                Speak("[" + voiceCode + " :dv ap " +
                    pitch.ToString(CultureInfo.InvariantCulture) +
                    voiceParameters + "] " +
                    indexedText);
                ThrowIfCancellationRequested(cancellationRequested);
                lock (stateLock)
                {
                    nativeSynthesisActive = true;
                }
                OmnivoxHelperLog.Event("native_call_started",
                    "engine=dectalk call=TextToSpeechSync");
                Check(native.TextToSpeechSync(handle),
                    "TextToSpeechSync");
                OmnivoxHelperLog.Event("native_call_completed",
                    "engine=dectalk call=TextToSpeechSync");
                ThrowCallbackError();
                EmitProgressiveAnchorMarkers(trailingAnchorMarkers,
                    capturedFrames);
                FlushProgressiveAudio();
                lock (stateLock)
                {
                    byte[] audio = capture == null ? new byte[0] :
                        capture.ToArray();
                    if (capture != null)
                    {
                        markers.Sort(CompareMarkers);
                        ApplyPcmGain(audio, volume);
                    }
                    return new OmnivoxCaptureResult(audio, markers.ToArray());
                }
            }
            finally
            {
                lock (stateLock)
                {
                    nativeSynthesisActive = false;
                    if (capture != null)
                    {
                        capture.Dispose();
                        capture = null;
                    }
                    markers = null;
                    pendingTextMarkers = null;
                    pendingAnchorMarkers = null;
                    leadingAnchorMarkers = null;
                    trailingAnchorMarkers = null;
                    progressiveSink = null;
                    pendingProgressiveAudio = null;
                }
            }
        }
    }

    private static void ThrowIfCancellationRequested(
        Func<bool> cancellationRequested)
    {
        if (cancellationRequested())
        {
            throw new OperationCanceledException(
                "DECtalk synthesis was cancelled");
        }
    }

    internal void Stop()
    {
        bool shouldReset;
        lock (stateLock)
        {
            shouldReset = nativeSynthesisActive;
            if (shouldReset)
            {
                discardAudio = true;
            }
        }
        if (!shouldReset)
        {
            return;
        }
        try
        {
            if (handle != IntPtr.Zero)
            {
                native.TextToSpeechReset(handle, false);
            }
        }
        finally
        {
            lock (stateLock)
            {
                discardAudio = false;
                callbackError = null;
            }
        }
    }

    private void BeginCapture(IOmnivoxCaptureSink sink, double volume)
    {
        lock (stateLock)
        {
            discardAudio = true;
        }
        Check(native.TextToSpeechReset(handle, false),
            "TextToSpeechReset");
        lock (stateLock)
        {
            callbackError = null;
            if (capture != null)
            {
                capture.Dispose();
            }
            progressiveSink = sink;
            capturedFrames = 0;
            outputVolume = volume;
            pendingProgressiveAudio = null;
            capture = sink == null ? new MemoryStream() : null;
            markers = new List<OmnivoxHelperMarker>();
            pendingTextMarkers =
                new Dictionary<uint, OmnivoxHelperMarker>();
            pendingAnchorMarkers =
                new Dictionary<uint, List<OmnivoxHelperMarker>>();
            leadingAnchorMarkers = new List<OmnivoxHelperMarker>();
            trailingAnchorMarkers = new List<OmnivoxHelperMarker>();
            discardAudio = false;
        }
    }

    private void Speak(string text)
    {
        byte[] bytes = textEncoding.GetBytes(text + "\0");
        GCHandle pinned = GCHandle.Alloc(bytes, GCHandleType.Pinned);
        try
        {
            Check(native.TextToSpeechSpeak(handle,
                pinned.AddrOfPinnedObject(), TtsForce), "TextToSpeechSpeak");
        }
        finally
        {
            pinned.Free();
        }
    }

    private string BuildTextWithIndexes(string text,
        OmnivoxHelperAnchor[] anchors)
    {
        HashSet<uint> reservedIndexes = CollectNativeNumbers(text);
        uint[] utf8Offsets = BuildUtf8Offsets(text);
        List<MarkerInsertion> insertions = new List<MarkerInsertion>();
        int sequence = 0;
        int position = 0;
        int candidate = LastPrivateMarkerIndex;

        foreach (OmnivoxTextSpan sentence in
            OmnivoxTextBoundaries.Sentences(text))
        {
            uint indexValue;
            if (insertions.Count >= MaximumMarkers ||
                !TakePrivateMarkerIndex(reservedIndexes, ref candidate,
                    out indexValue))
            {
                break;
            }
            uint textStart = utf8Offsets[sentence.Start];
            uint textLength = checked(
                utf8Offsets[sentence.Start + sentence.Length] - textStart);
            insertions.Add(new MarkerInsertion
            {
                Position = sentence.Start,
                Priority = 0,
                Sequence = sequence++,
                IndexValue = indexValue,
                Marker = new OmnivoxHelperMarker("sentence", 0,
                    textStart, textLength, null)
            });
            reservedIndexes.Add(indexValue);
        }

        while (position < text.Length &&
            insertions.Count < MaximumMarkers)
        {
            int nativeEnd;
            if (TryGetNativeSpanEnd(text, position, out nativeEnd))
            {
                position = nativeEnd;
                continue;
            }

            int scalarLength;
            if (!IsWordCore(text, position, out scalarLength))
            {
                position += scalarLength;
                continue;
            }

            int wordStart = position;
            bool crossesNativeSpan = false;
            position += scalarLength;
            while (position < text.Length)
            {
                if (IsWordCore(text, position, out scalarLength))
                {
                    position += scalarLength;
                    continue;
                }

                if (TryGetNativeSpanEnd(text, position, out nativeEnd) &&
                    StartsWordContinuation(text, nativeEnd))
                {
                    crossesNativeSpan = true;
                    position = nativeEnd;
                    continue;
                }

                if (IsInnerWordConnector(text, position,
                    out scalarLength))
                {
                    int continuation = SkipNativeSpans(text,
                        position + scalarLength);
                    int nextLength;
                    if (continuation < text.Length &&
                        IsWordCore(text, continuation, out nextLength))
                    {
                        if (continuation != position + scalarLength)
                        {
                            crossesNativeSpan = true;
                        }
                        position = continuation + nextLength;
                        continue;
                    }
                }
                break;
            }

            if (crossesNativeSpan)
            {
                continue;
            }

            uint indexValue;
            if (!TakePrivateMarkerIndex(reservedIndexes, ref candidate,
                out indexValue))
            {
                break;
            }

            int wordLength = position - wordStart;
            uint textStart = utf8Offsets[wordStart];
            uint textLength = checked(utf8Offsets[position] - textStart);
            string value = text.Substring(wordStart, wordLength);
            if (Encoding.UTF8.GetByteCount(value) > MaximumMarkerValueBytes)
            {
                value = null;
            }
            insertions.Add(new MarkerInsertion
            {
                Position = wordStart,
                Priority = 1,
                Sequence = sequence++,
                IndexValue = indexValue,
                Marker = new OmnivoxHelperMarker("word", 0,
                    textStart, textLength, value)
            });
            reservedIndexes.Add(indexValue);
        }

        foreach (OmnivoxHelperAnchor anchor in anchors)
        {
            MarkerInsertion selected = SelectWordAnchor(insertions, anchor);
            if (selected == null)
            {
                OmnivoxHelperMarker boundary = new OmnivoxHelperMarker(
                    "requested_anchor", 0, anchor.TextOffset, 0,
                    anchor.Id, "span_boundary");
                if (anchor.Affinity == "before")
                {
                    leadingAnchorMarkers.Add(boundary);
                }
                else
                {
                    trailingAnchorMarkers.Add(boundary);
                }
                continue;
            }
            List<OmnivoxHelperMarker> aliases;
            if (!pendingAnchorMarkers.TryGetValue(selected.IndexValue,
                out aliases))
            {
                aliases = new List<OmnivoxHelperMarker>();
                pendingAnchorMarkers.Add(selected.IndexValue, aliases);
            }
            aliases.Add(new OmnivoxHelperMarker("requested_anchor", 0,
                anchor.TextOffset, 0, anchor.Id, "word_boundary"));
        }

        insertions.Sort(delegate(MarkerInsertion left,
            MarkerInsertion right)
        {
            int order = left.Position.CompareTo(right.Position);
            if (order == 0)
            {
                order = left.Priority.CompareTo(right.Priority);
            }
            return order == 0 ? left.Sequence.CompareTo(right.Sequence) :
                order;
        });

        StringBuilder result = new StringBuilder(text.Length +
            insertions.Count * 24);
        int copiedThrough = 0;
        foreach (MarkerInsertion insertion in insertions)
        {
            result.Append(text, copiedThrough,
                insertion.Position - copiedThrough);
            result.Append("[:index mark ");
            result.Append(insertion.IndexValue.ToString(
                CultureInfo.InvariantCulture));
            result.Append("]");
            pendingTextMarkers.Add(insertion.IndexValue, insertion.Marker);
            copiedThrough = insertion.Position;
        }
        result.Append(text, copiedThrough, text.Length - copiedThrough);
        return result.ToString();
    }

    private static MarkerInsertion SelectWordAnchor(
        List<MarkerInsertion> insertions, OmnivoxHelperAnchor anchor)
    {
        MarkerInsertion selected = null;
        foreach (MarkerInsertion insertion in insertions)
        {
            if (insertion.Marker.Kind != "word" ||
                !insertion.Marker.TextStart.HasValue)
            {
                continue;
            }
            uint textStart = insertion.Marker.TextStart.Value;
            if (anchor.Affinity == "before")
            {
                if (textStart >= anchor.TextOffset &&
                    (selected == null || textStart <
                        selected.Marker.TextStart.Value))
                {
                    selected = insertion;
                }
            }
            else if (textStart <= anchor.TextOffset &&
                (selected == null || textStart >
                    selected.Marker.TextStart.Value))
            {
                selected = insertion;
            }
        }
        return selected;
    }

    private void EmitProgressiveAnchorMarkers(
        List<OmnivoxHelperMarker> pending, ulong frameOffset)
    {
        if (pending == null || pending.Count == 0)
        {
            return;
        }
        IOmnivoxCaptureSink sink;
        OmnivoxHelperMarker[] batch;
        lock (stateLock)
        {
            sink = progressiveSink;
            if (sink == null)
            {
                return;
            }
            if (markers.Count > MaximumMarkers - pending.Count)
            {
                throw new InvalidOperationException(
                    "DECtalk synthesis exceeded the marker limit while resolving anchors");
            }
            batch = pending.ToArray();
            foreach (OmnivoxHelperMarker marker in batch)
            {
                marker.FrameOffset = frameOffset;
                markers.Add(marker);
            }
            pending.Clear();
        }
        sink.Markers(batch);
    }

    private static HashSet<uint> CollectNativeNumbers(string text)
    {
        HashSet<uint> values = new HashSet<uint>();
        int position = 0;
        while (position < text.Length)
        {
            int nativeEnd;
            if (!TryGetNativeSpanEnd(text, position, out nativeEnd))
            {
                ++position;
                continue;
            }

            int numberStart = position + 1;
            while (numberStart < nativeEnd)
            {
                while (numberStart < nativeEnd &&
                    !Char.IsDigit(text[numberStart]))
                {
                    ++numberStart;
                }
                int numberEnd = numberStart;
                while (numberEnd < nativeEnd &&
                    Char.IsDigit(text[numberEnd]))
                {
                    ++numberEnd;
                }
                if (numberEnd > numberStart)
                {
                    uint value;
                    if (UInt32.TryParse(text.Substring(numberStart,
                        numberEnd - numberStart), NumberStyles.None,
                        CultureInfo.InvariantCulture, out value))
                    {
                        values.Add(value);
                    }
                }
                numberStart = numberEnd + 1;
            }
            position = nativeEnd;
        }
        return values;
    }

    private static uint[] BuildUtf8Offsets(string text)
    {
        uint[] offsets = new uint[text.Length + 1];
        uint byteOffset = 0;
        int position = 0;
        while (position < text.Length)
        {
            offsets[position] = byteOffset;
            char value = text[position];
            if (Char.IsHighSurrogate(value) && position + 1 < text.Length &&
                Char.IsLowSurrogate(text[position + 1]))
            {
                offsets[position + 1] = byteOffset;
                byteOffset = checked(byteOffset + 4);
                position += 2;
            }
            else
            {
                byteOffset = checked(byteOffset +
                    (value <= 0x7f ? 1u : value <= 0x7ff ? 2u : 3u));
                ++position;
            }
            offsets[position] = byteOffset;
        }
        return offsets;
    }

    private static void ApplyPcmGain(byte[] audio, double volume)
    {
        if (volume >= 1.0)
        {
            return;
        }
        for (int offset = 0; offset + 1 < audio.Length; offset += 2)
        {
            short sample = (short)(audio[offset] | audio[offset + 1] << 8);
            int scaled = (int)Math.Round(sample * volume,
                MidpointRounding.AwayFromZero);
            scaled = Math.Max(Int16.MinValue, Math.Min(Int16.MaxValue, scaled));
            audio[offset] = (byte)(scaled & 0xff);
            audio[offset + 1] = (byte)((scaled >> 8) & 0xff);
        }
    }

    private static bool TakePrivateMarkerIndex(HashSet<uint> reserved,
        ref int candidate, out uint value)
    {
        while (candidate >= FirstPrivateMarkerIndex)
        {
            value = (uint)candidate--;
            if (!reserved.Contains(value))
            {
                return true;
            }
        }
        value = 0;
        return false;
    }

    private static bool TryGetNativeSpanEnd(string text, int position,
        out int end)
    {
        end = position;
        if (position >= text.Length || text[position] != '[')
        {
            return false;
        }
        int closing = text.IndexOf(']', position + 1);
        end = closing < 0 ? text.Length : closing + 1;
        return true;
    }

    private static int SkipNativeSpans(string text, int position)
    {
        int end;
        while (TryGetNativeSpanEnd(text, position, out end) &&
            end > position)
        {
            position = end;
        }
        return position;
    }

    private static bool StartsWordContinuation(string text, int position)
    {
        position = SkipNativeSpans(text, position);
        if (position >= text.Length)
        {
            return false;
        }
        int scalarLength;
        if (IsWordCore(text, position, out scalarLength))
        {
            return true;
        }
        if (!IsInnerWordConnector(text, position, out scalarLength))
        {
            return false;
        }
        position = SkipNativeSpans(text, position + scalarLength);
        int nextLength;
        return position < text.Length &&
            IsWordCore(text, position, out nextLength);
    }

    private static bool IsWordCore(string text, int position,
        out int scalarLength)
    {
        scalarLength = Char.IsHighSurrogate(text[position]) &&
            position + 1 < text.Length &&
            Char.IsLowSurrogate(text[position + 1]) ? 2 : 1;
        switch (CharUnicodeInfo.GetUnicodeCategory(text, position))
        {
            case UnicodeCategory.UppercaseLetter:
            case UnicodeCategory.LowercaseLetter:
            case UnicodeCategory.TitlecaseLetter:
            case UnicodeCategory.ModifierLetter:
            case UnicodeCategory.OtherLetter:
            case UnicodeCategory.NonSpacingMark:
            case UnicodeCategory.SpacingCombiningMark:
            case UnicodeCategory.DecimalDigitNumber:
            case UnicodeCategory.LetterNumber:
            case UnicodeCategory.OtherNumber:
            case UnicodeCategory.ConnectorPunctuation:
                return true;
            default:
                return false;
        }
    }

    private static bool IsInnerWordConnector(string text, int position,
        out int scalarLength)
    {
        scalarLength = 1;
        char value = text[position];
        return value == '\'' || value == '\u2019' || value == '-';
    }

    private void AllocateBuffers()
    {
        int structureSize = Marshal.SizeOf(typeof(OmnivoxDectalkBuffer));
        int phonemeSize = Marshal.SizeOf(typeof(OmnivoxDectalkPhoneme));
        int indexSize = Marshal.SizeOf(typeof(OmnivoxDectalkIndex));
        for (int index = 0; index < BufferCount; ++index)
        {
            BufferSlot slot = new BufferSlot();
            slot.Data = Marshal.AllocHGlobal(BufferBytes);
            slot.Phonemes = Marshal.AllocHGlobal(
                MarkerRecordsPerBuffer * phonemeSize);
            slot.Indexes = Marshal.AllocHGlobal(
                MarkerRecordsPerBuffer * indexSize);
            slot.Buffer = Marshal.AllocHGlobal(structureSize);
            OmnivoxDectalkBuffer buffer = new OmnivoxDectalkBuffer();
            buffer.Data = slot.Data;
            buffer.PhonemeArray = slot.Phonemes;
            buffer.IndexArray = slot.Indexes;
            buffer.MaximumBufferLength = BufferBytes;
            buffer.MaximumPhonemeChanges = MarkerRecordsPerBuffer;
            buffer.MaximumIndexMarks = MarkerRecordsPerBuffer;
            Marshal.StructureToPtr(buffer, slot.Buffer, false);
            buffers.Add(slot);
            Check(native.TextToSpeechAddBuffer(handle,
                slot.Buffer), "TextToSpeechAddBuffer");
        }
    }

    private void OnDectalkCallback(int parameter1, int parameter2,
        uint userParameter, uint message)
    {
        if (message != bufferMessage || parameter2 == 0)
        {
            return;
        }

        IntPtr bufferPointer = new IntPtr(parameter2);
        try
        {
            OmnivoxDectalkBuffer buffer =
                (OmnivoxDectalkBuffer)Marshal.PtrToStructure(bufferPointer,
                    typeof(OmnivoxDectalkBuffer));
            if (buffer.BufferLength > BufferBytes ||
                (buffer.BufferLength & 1) != 0 ||
                buffer.NumberOfPhonemeChanges > MarkerRecordsPerBuffer ||
                buffer.NumberOfIndexMarks > MarkerRecordsPerBuffer)
            {
                throw new InvalidOperationException(
                    "DECtalk returned invalid buffer metadata");
            }

            byte[] audio = null;
            byte[] readyAudio = null;
            OmnivoxHelperMarker[] markerBatch = null;
            IOmnivoxCaptureSink sink = null;
            lock (stateLock)
            {
                if (!discardAudio && !shuttingDown)
                {
                    int firstMarker = markers.Count;
                    CaptureMarkers(buffer);
                    sink = progressiveSink;
                    if (sink != null && markers.Count > firstMarker)
                    {
                        List<OmnivoxHelperMarker> batch = markers.GetRange(
                            firstMarker, markers.Count - firstMarker);
                        batch.Sort(CompareMarkers);
                        markerBatch = batch.ToArray();
                    }
                    if (buffer.BufferLength > 0)
                    {
                        int count = checked((int)buffer.BufferLength);
                        if (capturedFrames >
                            (ulong)(MaximumAudioBytes / 2 - count / 2))
                        {
                            throw new InvalidOperationException(
                                "DECtalk synthesis exceeded the 128 MiB audio limit");
                        }
                        audio = new byte[count];
                        Marshal.Copy(buffer.Data, audio, 0, count);
                        if (sink == null)
                        {
                            capture.Write(audio, 0, count);
                            capturedFrames += (ulong)(count / 2);
                        }
                        else
                        {
                            ApplyPcmGain(audio, outputVolume);
                            // DECtalk can report a marker a few native samples
                            // after the audio callback containing that frame.
                            // Retain exactly one bounded 512-sample block so
                            // the next callback can publish those markers first.
                            readyAudio = pendingProgressiveAudio;
                            pendingProgressiveAudio = audio;
                            capturedFrames += (ulong)(count / 2);
                        }
                    }
                }
            }
            if (sink != null)
            {
                if (markerBatch != null)
                {
                    sink.Markers(markerBatch);
                }
                if (readyAudio != null)
                {
                    sink.Audio(readyAudio, 0, readyAudio.Length);
                }
            }
        }
        catch (Exception error)
        {
            SetCallbackError(error);
        }
        finally
        {
            try
            {
                OmnivoxDectalkBuffer buffer =
                    (OmnivoxDectalkBuffer)Marshal.PtrToStructure(
                        bufferPointer, typeof(OmnivoxDectalkBuffer));
                buffer.BufferLength = 0;
                buffer.NumberOfPhonemeChanges = 0;
                buffer.NumberOfIndexMarks = 0;
                Marshal.StructureToPtr(buffer, bufferPointer, false);
                if (!IsShuttingDown())
                {
                    Check(native.TextToSpeechAddBuffer(handle,
                        bufferPointer), "TextToSpeechAddBuffer");
                }
            }
            catch (Exception error)
            {
                SetCallbackError(error);
            }
        }
    }

    private void FlushProgressiveAudio()
    {
        IOmnivoxCaptureSink sink;
        byte[] audio;
        lock (stateLock)
        {
            sink = progressiveSink;
            audio = pendingProgressiveAudio;
            pendingProgressiveAudio = null;
        }
        if (sink != null && audio != null)
        {
            sink.Audio(audio, 0, audio.Length);
        }
    }

    private void CaptureMarkers(OmnivoxDectalkBuffer buffer)
    {
        if (markers == null || markers.Count >= MaximumMarkers)
        {
            return;
        }
        int phonemeSize = Marshal.SizeOf(typeof(OmnivoxDectalkPhoneme));
        for (uint index = 0; index < buffer.NumberOfPhonemeChanges &&
            markers.Count < MaximumMarkers; ++index)
        {
            IntPtr address = new IntPtr(buffer.PhonemeArray.ToInt64() +
                checked((long)index * phonemeSize));
            OmnivoxDectalkPhoneme phoneme =
                (OmnivoxDectalkPhoneme)Marshal.PtrToStructure(address,
                    typeof(OmnivoxDectalkPhoneme));
            markers.Add(new OmnivoxHelperMarker("phoneme",
                phoneme.SampleNumber, null, null,
                phoneme.Phoneme.ToString(CultureInfo.InvariantCulture)));
        }

        int indexSize = Marshal.SizeOf(typeof(OmnivoxDectalkIndex));
        for (uint index = 0; index < buffer.NumberOfIndexMarks &&
            markers.Count < MaximumMarkers; ++index)
        {
            IntPtr address = new IntPtr(buffer.IndexArray.ToInt64() +
                checked((long)index * indexSize));
            OmnivoxDectalkIndex marker =
                (OmnivoxDectalkIndex)Marshal.PtrToStructure(address,
                    typeof(OmnivoxDectalkIndex));
            OmnivoxHelperMarker textMarker;
            if (pendingTextMarkers != null &&
                pendingTextMarkers.TryGetValue(marker.Value, out textMarker))
            {
                pendingTextMarkers.Remove(marker.Value);
                textMarker.FrameOffset = marker.SampleNumber;
                markers.Add(textMarker);
                List<OmnivoxHelperMarker> anchorMarkers;
                if (pendingAnchorMarkers != null &&
                    pendingAnchorMarkers.TryGetValue(marker.Value,
                        out anchorMarkers))
                {
                    if (markers.Count > MaximumMarkers -
                        anchorMarkers.Count)
                    {
                        throw new InvalidOperationException(
                            "DECtalk synthesis exceeded the marker limit while resolving anchors");
                    }
                    pendingAnchorMarkers.Remove(marker.Value);
                    foreach (OmnivoxHelperMarker anchorMarker in
                        anchorMarkers)
                    {
                        anchorMarker.FrameOffset = marker.SampleNumber;
                        markers.Add(anchorMarker);
                    }
                }
            }
            else
            {
                markers.Add(new OmnivoxHelperMarker("native_index",
                    marker.SampleNumber, null, null,
                    marker.Value.ToString(CultureInfo.InvariantCulture)));
            }
        }
    }

    private static int CompareMarkers(OmnivoxHelperMarker left,
        OmnivoxHelperMarker right)
    {
        int order = left.FrameOffset.CompareTo(right.FrameOffset);
        if (order != 0)
        {
            return order;
        }
        order = String.CompareOrdinal(left.Kind, right.Kind);
        return order != 0 ? order : String.CompareOrdinal(
            left.Value, right.Value);
    }

    private bool IsShuttingDown()
    {
        lock (stateLock)
        {
            return shuttingDown;
        }
    }

    private void SetCallbackError(Exception error)
    {
        lock (stateLock)
        {
            if (callbackError == null)
            {
                callbackError = error;
            }
        }
    }

    private void ThrowCallbackError()
    {
        Exception error;
        lock (stateLock)
        {
            error = callbackError;
            callbackError = null;
        }
        if (error != null)
        {
            throw new InvalidOperationException(
                "DECtalk PCM capture failed", error);
        }
    }

    private static void Check(uint result, string operation)
    {
        if (result != 0)
        {
            throw new InvalidOperationException(operation +
                " failed with DECtalk error " + result);
        }
    }

    public void Dispose()
    {
        lock (stateLock)
        {
            shuttingDown = true;
            discardAudio = true;
        }
        if (handle != IntPtr.Zero)
        {
            native.TextToSpeechReset(handle, false);
            if (memoryOpen)
            {
                native.TextToSpeechCloseInMemory(handle);
                memoryOpen = false;
            }
            native.TextToSpeechShutdown(handle);
            handle = IntPtr.Zero;
        }
        for (int index = 0; index < buffers.Count; ++index)
        {
            if (buffers[index].Buffer != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(buffers[index].Buffer);
            }
            if (buffers[index].Data != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(buffers[index].Data);
            }
            if (buffers[index].Phonemes != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(buffers[index].Phonemes);
            }
            if (buffers[index].Indexes != IntPtr.Zero)
            {
                Marshal.FreeHGlobal(buffers[index].Indexes);
            }
        }
        buffers.Clear();
        lock (stateLock)
        {
            if (capture != null)
            {
                capture.Dispose();
                capture = null;
            }
            markers = null;
            pendingTextMarkers = null;
        }
        if (native != null)
        {
            native.Dispose();
            native = null;
        }
    }
}
