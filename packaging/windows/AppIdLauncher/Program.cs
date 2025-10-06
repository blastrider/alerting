using System;
using System.Diagnostics;
using System.IO;
using System.Runtime.InteropServices;

namespace AlertingLauncher;

internal static class Program
{
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    private static extern int SetCurrentProcessExplicitAppUserModelID(string appID);

    private const string DefaultAppUserModelId = "Alerting";

    private static int Main(string[] args)
    {
        try
        {
            SetCurrentProcessExplicitAppUserModelID(DefaultAppUserModelId);
        }
        catch
        {
            // If we cannot set the AppUserModelID we still attempt to launch the worker.
        }

        var target = Environment.GetEnvironmentVariable("ALERTING_LAUNCH_TARGET");
        if (string.IsNullOrWhiteSpace(target))
        {
            target = Path.Combine(AppContext.BaseDirectory, "alerting.exe");
        }

        if (!File.Exists(target))
        {
            Console.Error.WriteLine($"alerting launcher: unable to locate '{target}'");
            return 1;
        }

        var psi = new ProcessStartInfo
        {
            FileName = target,
            Arguments = string.Join(" ", args),
            WorkingDirectory = Path.GetDirectoryName(target) ?? AppContext.BaseDirectory,
            UseShellExecute = false,
        };

        try
        {
            using var child = Process.Start(psi);
            if (child is null)
            {
                Console.Error.WriteLine("alerting launcher: failed to start child process");
                return 1;
            }
            child.WaitForExit();
            return child.ExitCode;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"alerting launcher: {ex.Message}");
            return 1;
        }
    }
}
