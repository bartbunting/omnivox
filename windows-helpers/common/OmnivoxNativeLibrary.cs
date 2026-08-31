// Copyright (C) 2026 Bart Bunting
// SPDX-License-Identifier: GPL-2.0-or-later
//
// This file is not part of GNU Emacs, but the same permissions apply.
// See the file COPYING in this distribution.

using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;

internal sealed class OmnivoxRuntimeUnavailableException : Exception
{
    internal OmnivoxRuntimeUnavailableException(string message)
        : base(message)
    {
    }

    internal OmnivoxRuntimeUnavailableException(string message,
        Exception innerException)
        : base(message, innerException)
    {
    }
}

/// <summary>
/// Owns one explicitly selected x86 native library.  The selected image and
/// every required export are validated before an engine adapter invokes it.
/// Dependencies may resolve only beside that image or from System32.
/// </summary>
internal sealed class OmnivoxNativeLibrary : IDisposable
{
    private const ushort ImageFileMachineI386 = 0x014c;
    private const uint LoadLibrarySearchDllLoadDir = 0x00000100;
    private const uint LoadLibrarySearchSystem32 = 0x00000800;

    [DllImport("kernel32.dll", CharSet = CharSet.Unicode,
        SetLastError = true, EntryPoint = "LoadLibraryExW")]
    private static extern IntPtr LoadLibraryEx(string fileName,
        IntPtr file, uint flags);

    [DllImport("kernel32.dll", CharSet = CharSet.Ansi,
        ExactSpelling = true, SetLastError = true)]
    private static extern IntPtr GetProcAddress(IntPtr module,
        string procedureName);

    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    private static extern bool FreeLibrary(IntPtr module);

    private readonly string displayName;
    private IntPtr module;

    internal OmnivoxNativeLibrary(string path, string displayName,
        string[] requiredExports)
    {
        if (IntPtr.Size != 4)
        {
            throw new OmnivoxRuntimeUnavailableException(
                displayName + " requires the 32-bit x86 helper executable");
        }
        if (String.IsNullOrEmpty(path) || !Path.IsPathRooted(path))
        {
            throw new OmnivoxRuntimeUnavailableException(
                displayName + " requires an absolute DLL path");
        }
        if (requiredExports == null || requiredExports.Length == 0)
        {
            throw new ArgumentException(
                "at least one required native export must be supplied",
                "requiredExports");
        }

        this.displayName = displayName;
        FullPath = Path.GetFullPath(path);
        ValidateX86PortableExecutable(FullPath, displayName);

        module = LoadLibraryEx(FullPath, IntPtr.Zero,
            LoadLibrarySearchDllLoadDir | LoadLibrarySearchSystem32);
        if (module == IntPtr.Zero)
        {
            int errorCode = Marshal.GetLastWin32Error();
            Win32Exception error = new Win32Exception(errorCode);
            throw new OmnivoxRuntimeUnavailableException(
                "Could not securely load " + displayName + " from \"" +
                FullPath + "\" (Windows error " + errorCode + ": " +
                error.Message + "). Its native dependencies must be beside " +
                "the DLL or available from System32.", error);
        }

        try
        {
            for (int index = 0; index < requiredExports.Length; ++index)
            {
                string export = requiredExports[index];
                if (String.IsNullOrEmpty(export) ||
                    GetProcAddress(module, export) == IntPtr.Zero)
                {
                    throw new OmnivoxRuntimeUnavailableException(
                        displayName + " is missing required export " + export);
                }
            }
        }
        catch
        {
            Dispose();
            throw;
        }
    }

    internal string FullPath { get; private set; }

    internal T Resolve<T>(string export) where T : class
    {
        if (module == IntPtr.Zero)
        {
            throw new ObjectDisposedException("OmnivoxNativeLibrary");
        }
        IntPtr address = GetProcAddress(module, export);
        if (address == IntPtr.Zero)
        {
            throw new OmnivoxRuntimeUnavailableException(
                displayName + " is missing required export " + export);
        }
        return (T)(object)Marshal.GetDelegateForFunctionPointer(address,
            typeof(T));
    }

    public void Dispose()
    {
        if (module != IntPtr.Zero)
        {
            FreeLibrary(module);
            module = IntPtr.Zero;
        }
    }

    private static void ValidateX86PortableExecutable(string path,
        string displayName)
    {
        if (!File.Exists(path))
        {
            throw new OmnivoxRuntimeUnavailableException(
                displayName + " was not found at \"" + path + "\"");
        }

        try
        {
            using (FileStream stream = new FileStream(path, FileMode.Open,
                FileAccess.Read, FileShare.Read))
            using (BinaryReader reader = new BinaryReader(stream))
            {
                if (stream.Length < 64 || reader.ReadUInt16() != 0x5a4d)
                {
                    throw InvalidImage(displayName);
                }
                stream.Position = 0x3c;
                int headerOffset = reader.ReadInt32();
                if (headerOffset < 0 || headerOffset > stream.Length - 6)
                {
                    throw InvalidImage(displayName);
                }
                stream.Position = headerOffset;
                if (reader.ReadUInt32() != 0x00004550)
                {
                    throw InvalidImage(displayName);
                }
                ushort machine = reader.ReadUInt16();
                if (machine != ImageFileMachineI386)
                {
                    throw new OmnivoxRuntimeUnavailableException(
                        displayName + " must be a 32-bit x86 PE image; " +
                        "the selected DLL has machine type 0x" +
                        machine.ToString("x4"));
                }
            }
        }
        catch (OmnivoxRuntimeUnavailableException)
        {
            throw;
        }
        catch (Exception error)
        {
            throw new OmnivoxRuntimeUnavailableException(
                "Could not validate " + displayName + " at \"" + path +
                "\": " + error.Message, error);
        }
    }

    private static OmnivoxRuntimeUnavailableException InvalidImage(
        string displayName)
    {
        return new OmnivoxRuntimeUnavailableException(
            displayName + " is not a valid Windows PE image");
    }
}
