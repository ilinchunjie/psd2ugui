using System;
using System.IO;

namespace PsdUi.Editor
{
    public static class Psd2UguiImportService
    {
        public sealed class ImportPsdResult
        {
            public string PrefabPath { get; set; } = string.Empty;
            public Psd2UguiCliRunner.PipelineResult Pipeline { get; set; }
        }

        internal static Func<Psd2UguiCliRunner.PipelineOptions, Psd2UguiCliRunner.PipelineResult> PipelineRunner =
            Psd2UguiCliRunner.RunPipeline;

        internal static Func<string, string, string> BundleImporter = PsdUiImporter.ImportBundle;

        public static ImportPsdResult ImportPsd(string psdPath)
        {
            var projectSettings = Psd2UguiProjectSettings.instance;
            var userSettings = Psd2UguiUserSettings.instance;

            if (!Psd2UguiSettingsValidator.TryValidatePsdPath(psdPath, out var pathError))
            {
                throw new InvalidOperationException(pathError);
            }

            if (!Psd2UguiSettingsValidator.TryValidatePsdImportSettings(
                    projectSettings,
                    userSettings,
                    out var settingsError))
            {
                throw new InvalidOperationException(settingsError);
            }

            Directory.CreateDirectory(projectSettings.CacheDirectory);

            var pipeline = PipelineRunner(new Psd2UguiCliRunner.PipelineOptions
            {
                PsdPath = psdPath,
                CacheDirectory = projectSettings.CacheDirectory,
                PhotoshopPath = userSettings.PhotoshopExePath
            });

            var prefabPath = BundleImporter(pipeline.BundleDir, projectSettings.UnityImportRoot);
            userSettings.LastPsdPath = psdPath;
            userSettings.LastBundlePath = pipeline.BundleDir;
            userSettings.SaveSettings();

            return new ImportPsdResult
            {
                PrefabPath = prefabPath,
                Pipeline = pipeline
            };
        }

        public static string ImportBundle(string bundlePath)
        {
            var projectSettings = Psd2UguiProjectSettings.instance;
            var userSettings = Psd2UguiUserSettings.instance;

            if (!Psd2UguiSettingsValidator.TryValidateBundleImportSettings(
                    projectSettings,
                    userSettings,
                    bundlePath,
                    out var error))
            {
                throw new InvalidOperationException(error);
            }

            var prefabPath = BundleImporter(bundlePath, projectSettings.UnityImportRoot);
            userSettings.LastBundlePath = bundlePath;
            userSettings.SaveSettings();
            return prefabPath;
        }
    }
}
