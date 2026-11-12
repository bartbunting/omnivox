using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;

internal static class EvoxWindowsLauncher
{
    [STAThread]
    private static void Main()
    {
        try
        {
            string directory = AppDomain.CurrentDomain.BaseDirectory;
            string script = Path.Combine(directory, "evox-windows.ps1");
            if (!File.Exists(script))
                throw new FileNotFoundException("Cannot find the Windows Emacsvox launcher.", script);

            ProcessStartInfo start = new ProcessStartInfo();
            start.FileName = Path.Combine(Environment.SystemDirectory,
                @"WindowsPowerShell\v1.0\powershell.exe");
            start.Arguments = "-NoLogo -NoProfile -NonInteractive -WindowStyle Hidden " +
                "-ExecutionPolicy Bypass -File \"" + script + "\"";
            start.WorkingDirectory = directory;
            start.UseShellExecute = false;
            start.CreateNoWindow = true;
            start.WindowStyle = ProcessWindowStyle.Hidden;
            start.RedirectStandardError = true;
            using (Process process = Process.Start(start))
            {
                string error = process.StandardError.ReadToEnd();
                process.WaitForExit();
                if (process.ExitCode != 0)
                    throw new Exception(String.IsNullOrWhiteSpace(error)
                        ? "Windows Emacsvox startup failed." : error);
            }
        }
        catch (Exception error)
        {
            MessageBox.Show(error.Message, "Emacsvox Windows",
                MessageBoxButtons.OK, MessageBoxIcon.Error);
        }
    }
}
