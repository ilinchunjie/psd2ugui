using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using Newtonsoft.Json;

namespace PsdUi.Editor
{
    public static class Psd2UguiCliRunner
    {
        internal interface IProcessRunner
        {
            ProcessExecutionResult Run(ProcessStartInfo startInfo);
        }

        internal sealed class ProcessExecutionResult
        {
            public int ExitCode { get; set; }
            public string StandardOutput { get; set; } = string.Empty;
            public string StandardError { get; set; } = string.Empty;
        }

        public sealed class PipelineOptions
        {
            public string PsdPath { get; set; } = string.Empty;
            public string CacheDirectory { get; set; } = string.Empty;
            public string PhotoshopExePath { get; set; } = string.Empty;
        }

        public sealed class PipelineResult
        {
            [JsonProperty("bundle_dir")] public string BundleDir { get; set; } = string.Empty;
            [JsonProperty("plan_path")] public string PlanPath { get; set; } = string.Empty;
            [JsonProperty("validation_report_path")] public string ValidationReportPath { get; set; } = string.Empty;
            [JsonProperty("document_id")] public string DocumentId { get; set; } = string.Empty;
            [JsonProperty("warnings")] public string[] Warnings { get; set; } = Array.Empty<string>();
        }

        private sealed class SystemProcessRunner : IProcessRunner
        {
            public ProcessExecutionResult Run(ProcessStartInfo startInfo)
            {
                using var process = Process.Start(startInfo);
                if (process == null)
                {
                    throw new InvalidOperationException($"Failed to start process: {startInfo.FileName}");
                }

                var standardOutput = process.StandardOutput.ReadToEnd();
                var standardError = process.StandardError.ReadToEnd();
                process.WaitForExit();

                return new ProcessExecutionResult
                {
                    ExitCode = process.ExitCode,
                    StandardOutput = standardOutput,
                    StandardError = standardError
                };
            }
        }

        private static IProcessRunner processRunner = new SystemProcessRunner();
        private static Func<string> executablePathResolver = Psd2UguiPackageInfo.GetWindowsExecutableFileSystemPath;

        public static PipelineResult RunPipeline(PipelineOptions options)
        {
            if (!Psd2UguiPackageInfo.IsWindowsEditor)
            {
                throw new InvalidOperationException("The embedded PSD2UGUI executable is only supported on Windows Unity Editor.");
            }

            if (options == null)
            {
                throw new ArgumentNullException(nameof(options));
            }

            var executablePath = executablePathResolver();
            if (!File.Exists(executablePath))
            {
                throw new FileNotFoundException("Embedded psd2ugui.exe was not found.", executablePath);
            }

            var startInfo = new ProcessStartInfo
            {
                FileName = executablePath,
                Arguments = BuildPipelineArguments(options),
                CreateNoWindow = true,
                UseShellExecute = false,
                RedirectStandardOutput = true,
                RedirectStandardError = true,
                WorkingDirectory = Psd2UguiPackageInfo.ProjectRootFileSystemPath
            };

            var result = processRunner.Run(startInfo);
            if (result.ExitCode != 0)
            {
                throw new InvalidOperationException(
                    $"psd2ugui.exe exited with code {result.ExitCode}.{Environment.NewLine}" +
                    $"stdout: {result.StandardOutput}{Environment.NewLine}" +
                    $"stderr: {result.StandardError}");
            }

            var payload = JsonConvert.DeserializeObject<PipelineResult>(result.StandardOutput);
            if (payload == null ||
                string.IsNullOrWhiteSpace(payload.BundleDir) ||
                string.IsNullOrWhiteSpace(payload.PlanPath) ||
                string.IsNullOrWhiteSpace(payload.ValidationReportPath) ||
                string.IsNullOrWhiteSpace(payload.DocumentId))
            {
                throw new InvalidOperationException(
                    $"psd2ugui.exe returned invalid JSON payload:{Environment.NewLine}{result.StandardOutput}");
            }

            return payload;
        }

        internal static void SetProcessRunnerForTests(IProcessRunner runner)
        {
            processRunner = runner ?? new SystemProcessRunner();
        }

        internal static void SetExecutablePathResolverForTests(Func<string> resolver)
        {
            executablePathResolver = resolver ?? Psd2UguiPackageInfo.GetWindowsExecutableFileSystemPath;
        }

        internal static void ResetProcessRunnerForTests()
        {
            processRunner = new SystemProcessRunner();
            executablePathResolver = Psd2UguiPackageInfo.GetWindowsExecutableFileSystemPath;
        }

        private static string BuildPipelineArguments(PipelineOptions options)
        {
            var arguments = new List<string>
            {
                "pipeline",
                options.PsdPath,
                "--cache-dir",
                options.CacheDirectory,
                "--raster-backend",
                "auto",
                "--photoshop-exe",
                options.PhotoshopExePath
            };

            return string.Join(" ", arguments.ConvertAll(QuoteArgument));
        }

        private static string QuoteArgument(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
            {
                return "\"\"";
            }

            if (!value.Contains(" ") && !value.Contains("\""))
            {
                return value;
            }

            return $"\"{value.Replace("\"", "\\\"")}\"";
        }
    }
}
