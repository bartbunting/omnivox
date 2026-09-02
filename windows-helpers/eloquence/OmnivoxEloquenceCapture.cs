// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

internal sealed class OmnivoxNativeEci : IDisposable
{
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    internal delegate int Callback(IntPtr handle, int message, int parameter,
        IntPtr data);

    [UnmanagedFunctionPointer(CallingConvention.StdCall, CharSet = CharSet.Ansi)]
    private delegate void VersionFunction(StringBuilder buffer);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate IntPtr NewExFunction(int languageDialect);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate void DeleteFunction(IntPtr handle);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private delegate bool BooleanHandleFunction(IntPtr handle);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private delegate bool AddTextFunction(IntPtr handle, IntPtr text);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private delegate bool InsertIndexFunction(IntPtr handle, int index);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate int SetParamFunction(IntPtr handle, int parameter,
        int value);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    private delegate void RegisterCallbackFunction(IntPtr handle,
        Callback callback, IntPtr data);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private delegate bool SetOutputBufferFunction(IntPtr handle, int samples,
        IntPtr buffer);

    private readonly OmnivoxNativeLibrary library;
    private readonly VersionFunction version;
    private readonly NewExFunction newEx;
    private readonly DeleteFunction delete;
    private readonly BooleanHandleFunction stop;
    private readonly BooleanHandleFunction clearInput;
    private readonly BooleanHandleFunction synthesize;
    private readonly BooleanHandleFunction synchronize;
    private readonly AddTextFunction addText;
    private readonly InsertIndexFunction insertIndex;
    private readonly SetParamFunction setParam;
    private readonly RegisterCallbackFunction registerCallback;
    private readonly SetOutputBufferFunction setOutputBuffer;

    internal OmnivoxNativeEci(string path)
    {
        string[] requiredExports = new string[]
        {
            "eciVersion", "eciNewEx", "eciDelete", "eciStop",
            "eciClearInput", "eciSynthesize", "eciSynchronize",
            "eciAddText", "eciInsertIndex", "eciSetParam",
            "eciRegisterCallback", "eciSetOutputBuffer"
        };
        library = new OmnivoxNativeLibrary(path, "Eloquence ECI.DLL",
            requiredExports);
        try
        {
            version = library.Resolve<VersionFunction>("eciVersion");
            newEx = library.Resolve<NewExFunction>("eciNewEx");
            delete = library.Resolve<DeleteFunction>("eciDelete");
            stop = library.Resolve<BooleanHandleFunction>("eciStop");
            clearInput = library.Resolve<BooleanHandleFunction>(
                "eciClearInput");
            synthesize = library.Resolve<BooleanHandleFunction>(
                "eciSynthesize");
            synchronize = library.Resolve<BooleanHandleFunction>(
                "eciSynchronize");
            addText = library.Resolve<AddTextFunction>("eciAddText");
            insertIndex = library.Resolve<InsertIndexFunction>(
                "eciInsertIndex");
            setParam = library.Resolve<SetParamFunction>("eciSetParam");
            registerCallback = library.Resolve<RegisterCallbackFunction>(
                "eciRegisterCallback");
            setOutputBuffer = library.Resolve<SetOutputBufferFunction>(
                "eciSetOutputBuffer");
        }
        catch
        {
            library.Dispose();
            throw;
        }
    }

    internal void Version(StringBuilder buffer) { version(buffer); }
    internal IntPtr NewEx(int dialect) { return newEx(dialect); }
    internal void Delete(IntPtr handle) { delete(handle); }
    internal bool Stop(IntPtr handle) { return stop(handle); }
    internal bool ClearInput(IntPtr handle) { return clearInput(handle); }
    internal bool Synthesize(IntPtr handle) { return synthesize(handle); }
    internal bool Synchronize(IntPtr handle) { return synchronize(handle); }
    internal bool AddText(IntPtr handle, IntPtr text)
    {
        return addText(handle, text);
    }
    internal bool InsertIndex(IntPtr handle, int index)
    {
        return insertIndex(handle, index);
    }
    internal int SetParam(IntPtr handle, int parameter, int value)
    {
        return setParam(handle, parameter, value);
    }
    internal void RegisterCallback(IntPtr handle, Callback callback,
        IntPtr data)
    {
        registerCallback(handle, callback, data);
    }
    internal bool SetOutputBuffer(IntPtr handle, int samples, IntPtr buffer)
    {
        return setOutputBuffer(handle, samples, buffer);
    }

    public void Dispose()
    {
        library.Dispose();
    }
}

/// <summary>
/// Owns one 32-bit ECI instance and captures its native mono PCM.  This is
/// deliberately independent of EloquenceEngine, whose callbacks feed waveOut
/// for the existing standalone Emacsvox server.
/// </summary>
internal sealed class OmnivoxEloquenceCapture : IDisposable
{
    private sealed class MarkerInsertion
    {
        internal int Position;
        internal int Priority;
        internal int Sequence;
        internal OmnivoxHelperMarker Marker;
    }

    private const int GeneralAmericanEnglish = 0x00010000;
    private const int SynthMode = 0;
    private const int InputType = 1;
    private const int SampleRate = 5;
    private const int OutputBufferSamples = 512;
    private const int WaveformBufferMessage = 0;
    private const int IndexReplyMessage = 2;
    private const int CallbackDataProcessed = 1;
    private const int CallbackAbort = 2;
    private const int FirstMarkerIndex = 1;
    private const int MaximumMarkers = 4096;
    private const int MaximumMarkerValueBytes = 16 * 1024;
    internal const int SpeechSampleRate = 11025;
    internal const int MaximumAudioBytes = 128 * 1024 * 1024;

    private readonly Encoding textEncoding;
    private readonly object synthesisLock = new object();
    private OmnivoxNativeEci native;
    private string runtimeVersion;
    private IntPtr handle;
    private IntPtr outputBuffer;
    private OmnivoxNativeEci.Callback callback;
    private MemoryStream capture;
    private Dictionary<int, OmnivoxHelperMarker> pendingMarkers;
    private List<OmnivoxHelperMarker> reachedMarkers;
    private Exception callbackError;
    private Func<bool> cancellationRequested;
    private IOmnivoxCaptureSink progressiveSink;
    private ulong capturedFrames;

    internal OmnivoxEloquenceCapture(string dllPath)
    {
        native = new OmnivoxNativeEci(dllPath);
        try
        {
            handle = native.NewEx(GeneralAmericanEnglish);
            if (handle == IntPtr.Zero)
            {
                throw new InvalidOperationException(
                    "ECI could not create an American English engine instance");
            }

            textEncoding = Encoding.GetEncoding(1252,
                EncoderFallback.ExceptionFallback,
                DecoderFallback.ExceptionFallback);
            outputBuffer = Marshal.AllocHGlobal(OutputBufferSamples * 2);
            callback = OnEciCallback;
            native.RegisterCallback(handle, callback, IntPtr.Zero);
            Configure();

            StringBuilder versionBuffer = new StringBuilder(32);
            native.Version(versionBuffer);
            if (versionBuffer.Length == 0)
            {
                throw new OmnivoxRuntimeUnavailableException(
                    "Eloquence ECI.DLL did not report a runtime version");
            }
            runtimeVersion = versionBuffer.ToString();
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

    internal OmnivoxCaptureResult Synthesize(string text, string voiceId,
        int rate, int pitch, string voiceParameters, int volume,
        OmnivoxHelperAnchor[] anchors, Func<bool> cancellationRequested,
        IOmnivoxCaptureSink sink)
    {
        lock (synthesisLock)
        {
            if (cancellationRequested())
            {
                throw new OperationCanceledException(
                    "Eloquence synthesis was cancelled before dispatch");
            }
            callbackError = null;
            this.cancellationRequested = cancellationRequested;
            progressiveSink = sink;
            capturedFrames = 0;
            capture = sink == null ? new MemoryStream() : null;
            pendingMarkers = new Dictionary<int, OmnivoxHelperMarker>();
            reachedMarkers = new List<OmnivoxHelperMarker>();
            try
            {
                // A cancelled synthesis can leave queued input behind.  Start
                // every request from a known empty native state.
                native.Stop(handle);
                Check(native.ClearInput(handle), "eciClearInput");
                Configure();
                AddText(" `" + voiceId + " `vs" +
                    rate.ToString(CultureInfo.InvariantCulture) + " `vb" +
                    pitch.ToString(CultureInfo.InvariantCulture) +
                    voiceParameters + " `vv" +
                    volume.ToString(CultureInfo.InvariantCulture) + " ");
                AddTextWithIndexes(text, anchors);
                Check(native.Synthesize(handle), "eciSynthesize");
                OmnivoxHelperLog.Event("native_call_started",
                    "engine=eloquence call=eciSynchronize");
                // ECI invokes this instance's callback on its owner thread.
                // A cancellation therefore aborts from OnEciCallback without
                // making an unsupported cross-thread native call.
                bool synchronized = native.Synchronize(handle);
                if (cancellationRequested())
                {
                    native.Stop(handle);
                    throw new OperationCanceledException(
                        "Eloquence synthesis was cancelled");
                }
                Check(synchronized, "eciSynchronize");
                OmnivoxHelperLog.Event("native_call_completed",
                    "engine=eloquence call=eciSynchronize frames=" +
                    capturedFrames.ToString(
                        CultureInfo.InvariantCulture));
                ThrowCallbackError();
                return new OmnivoxCaptureResult(
                    capture == null ? new byte[0] : capture.ToArray(),
                    reachedMarkers.ToArray());
            }
            finally
            {
                if (capture != null)
                {
                    capture.Dispose();
                }
                capture = null;
                pendingMarkers = null;
                reachedMarkers = null;
                this.cancellationRequested = null;
                progressiveSink = null;
                native.ClearInput(handle);
            }
        }
    }

    private void AddText(string text)
    {
        byte[] bytes = textEncoding.GetBytes(text + "\0");
        GCHandle pinned = GCHandle.Alloc(bytes, GCHandleType.Pinned);
        try
        {
            Check(native.AddText(handle,
                pinned.AddrOfPinnedObject()), "eciAddText");
        }
        finally
        {
            pinned.Free();
        }
    }

    private void AddTextWithIndexes(string text,
        OmnivoxHelperAnchor[] anchors)
    {
        List<MarkerInsertion> insertions = new List<MarkerInsertion>();
        int sequence = 0;
        foreach (OmnivoxHelperAnchor anchor in anchors)
        {
            insertions.Add(new MarkerInsertion
            {
                Position = Utf8OffsetToCharPosition(text, anchor.TextOffset),
                Priority = anchor.Affinity == "before" ? 0 : 3,
                Sequence = sequence++,
                Marker = new OmnivoxHelperMarker("requested_anchor", 0,
                    anchor.TextOffset, 0, anchor.Id)
            });
        }

        foreach (OmnivoxTextSpan sentence in
            OmnivoxTextBoundaries.Sentences(text))
        {
            if (insertions.Count >= MaximumMarkers)
            {
                break;
            }
            uint textStart = checked((uint)Encoding.UTF8.GetByteCount(
                text.Substring(0, sentence.Start)));
            uint textLength = checked((uint)Encoding.UTF8.GetByteCount(
                text.Substring(sentence.Start, sentence.Length)));
            insertions.Add(new MarkerInsertion
            {
                Position = sentence.Start,
                Priority = 1,
                Sequence = sequence++,
                Marker = new OmnivoxHelperMarker("sentence", 0,
                    textStart, textLength, null)
            });
        }

        int position = 0;
        while (position < text.Length && insertions.Count < MaximumMarkers)
        {
            int scalarLength;
            if (!IsWordCore(text, position, out scalarLength))
            {
                position += scalarLength;
                continue;
            }

            int wordStart = position;
            position += scalarLength;
            while (position < text.Length)
            {
                if (IsWordCore(text, position, out scalarLength))
                {
                    position += scalarLength;
                    continue;
                }
                if (IsInnerWordConnector(text, position, out scalarLength) &&
                    position + scalarLength < text.Length)
                {
                    int nextLength;
                    if (IsWordCore(text, position + scalarLength,
                        out nextLength))
                    {
                        position += scalarLength + nextLength;
                        continue;
                    }
                }
                break;
            }

            int wordLength = position - wordStart;
            uint textStart = checked((uint)Encoding.UTF8.GetByteCount(
                text.Substring(0, wordStart)));
            uint textLength = checked((uint)Encoding.UTF8.GetByteCount(
                text.Substring(wordStart, wordLength)));
            string value = text.Substring(wordStart, wordLength);
            if (Encoding.UTF8.GetByteCount(value) > MaximumMarkerValueBytes)
            {
                value = null;
            }
            insertions.Add(new MarkerInsertion
            {
                Position = wordStart,
                Priority = 2,
                Sequence = sequence++,
                Marker = new OmnivoxHelperMarker("word", 0, textStart,
                    textLength, value)
            });
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

        int cursor = 0;
        int markerIndex = FirstMarkerIndex;
        foreach (MarkerInsertion insertion in insertions)
        {
            AddTextSegment(text, cursor, insertion.Position - cursor);
            Check(native.InsertIndex(handle, markerIndex),
                "eciInsertIndex");
            pendingMarkers.Add(markerIndex++, insertion.Marker);
            cursor = insertion.Position;
        }
        AddTextSegment(text, cursor, text.Length - cursor);
    }

    private static int Utf8OffsetToCharPosition(string text,
        uint targetOffset)
    {
        uint byteOffset = 0;
        int position = 0;
        while (position < text.Length)
        {
            if (byteOffset == targetOffset)
            {
                return position;
            }
            int length = Char.IsHighSurrogate(text[position]) &&
                position + 1 < text.Length &&
                Char.IsLowSurrogate(text[position + 1]) ? 2 : 1;
            byteOffset = checked(byteOffset +
                (uint)Encoding.UTF8.GetByteCount(
                    text.Substring(position, length)));
            position += length;
        }
        if (byteOffset == targetOffset)
        {
            return position;
        }
        throw new ArgumentException("anchor is not on a UTF-8 boundary",
            "targetOffset");
    }

    private void AddTextSegment(string text, int start, int length)
    {
        if (length > 0)
        {
            AddText(text.Substring(start, length));
        }
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

    private void Configure()
    {
        CheckParameter(native.SetParam(handle, InputType, 1),
            "eciInputType");
        CheckParameter(native.SetParam(handle, SynthMode, 1),
            "eciSynthMode");
        CheckParameter(native.SetParam(handle, SampleRate, 1),
            "eciSampleRate");
        Check(native.SetOutputBuffer(handle, OutputBufferSamples,
            outputBuffer), "eciSetOutputBuffer");
    }

    private int OnEciCallback(IntPtr callbackHandle, int message,
        int parameter, IntPtr data)
    {
        if (cancellationRequested != null && cancellationRequested())
        {
            return CallbackAbort;
        }
        try
        {
            if (message == IndexReplyMessage)
            {
                OmnivoxHelperMarker marker;
                if (pendingMarkers != null &&
                    pendingMarkers.TryGetValue(parameter, out marker))
                {
                    pendingMarkers.Remove(parameter);
                    marker.FrameOffset = capturedFrames;
                    if (progressiveSink == null)
                    {
                        reachedMarkers.Add(marker);
                    }
                    else
                    {
                        progressiveSink.Markers(
                            new OmnivoxHelperMarker[] { marker });
                    }
                }
                return CallbackDataProcessed;
            }
            if (message != WaveformBufferMessage || parameter <= 0)
            {
                return CallbackDataProcessed;
            }
            int byteCount = checked(parameter * 2);
            if (capturedFrames >
                (ulong)(MaximumAudioBytes / 2 - byteCount / 2))
            {
                throw new InvalidOperationException(
                    "Eloquence synthesis exceeded the 128 MiB audio limit");
            }
            byte[] bytes = new byte[byteCount];
            Marshal.Copy(outputBuffer, bytes, 0, byteCount);
            if (progressiveSink == null)
            {
                capture.Write(bytes, 0, bytes.Length);
            }
            else
            {
                progressiveSink.Audio(bytes, 0, bytes.Length);
            }
            capturedFrames += (ulong)parameter;
            return CallbackDataProcessed;
        }
        catch (Exception error)
        {
            callbackError = error;
            return CallbackAbort;
        }
    }

    private void ThrowCallbackError()
    {
        if (callbackError != null)
        {
            Exception error = callbackError;
            callbackError = null;
            throw new InvalidOperationException(
                "Eloquence PCM capture failed", error);
        }
    }

    private static void Check(bool result, string operation)
    {
        if (!result)
        {
            throw new InvalidOperationException(operation + " failed");
        }
    }

    private static void CheckParameter(int result, string parameter)
    {
        if (result == -1)
        {
            throw new InvalidOperationException(
                "eciSetParam failed for " + parameter);
        }
    }

    public void Dispose()
    {
        if (handle != IntPtr.Zero)
        {
            native.Stop(handle);
            native.Delete(handle);
            handle = IntPtr.Zero;
        }
        if (outputBuffer != IntPtr.Zero)
        {
            Marshal.FreeHGlobal(outputBuffer);
            outputBuffer = IntPtr.Zero;
        }
        if (capture != null)
        {
            capture.Dispose();
            capture = null;
        }
        if (native != null)
        {
            native.Dispose();
            native = null;
        }
    }
}
