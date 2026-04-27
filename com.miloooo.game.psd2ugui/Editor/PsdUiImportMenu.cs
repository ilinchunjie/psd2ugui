using System;
using System.IO;
using UnityEditor;
using UnityEngine;

namespace PsdUi.Editor
{
    public static class Psd2UguiImportMenu
    {
        [MenuItem("美术/PSD2UGUI/Import PSD...", true)]
        private static bool CanImportPsdViaDialog()
        {
            return Psd2UguiPackageInfo.IsSupportedEditorPlatform;
        }

        [MenuItem("美术/PSD2UGUI/Import PSD...")]
        private static void ImportPsdViaDialog()
        {
            var initialDirectory = ResolveInitialFileDirectory(Psd2UguiUserSettings.instance.LastPsdPath);
            var psdPath = EditorUtility.OpenFilePanel("Select PSD file", initialDirectory, "psd");
            if (string.IsNullOrWhiteSpace(psdPath))
            {
                return;
            }

            ImportPsdAndLog(psdPath);
        }

        [MenuItem("美术/PSD2UGUI/Import Bundle...")]
        private static void ImportBundleViaDialog()
        {
            var initialDirectory = ResolveInitialFolderDirectory(Psd2UguiUserSettings.instance.LastBundlePath);
            var bundlePath = EditorUtility.OpenFolderPanel("Select PSD bundle", initialDirectory, string.Empty);
            if (string.IsNullOrWhiteSpace(bundlePath))
            {
                return;
            }

            ImportAndLog(bundlePath);
        }

        [MenuItem("美术/PSD2UGUI/Open Settings")]
        private static void OpenSettings()
        {
            SettingsService.OpenProjectSettings("Project/PSD2UGUI");
        }

        private static void ImportPsdAndLog(string psdPath)
        {
            try
            {
                var result = Psd2UguiImportService.ImportPsd(psdPath);
                if (result.Pipeline != null && result.Pipeline.Warnings != null)
                {
                    foreach (var warning in result.Pipeline.Warnings)
                    {
                        Debug.LogWarning($"PSD2UGUI warning: {warning}");
                    }
                }

                Debug.Log($"Imported PSD to prefab: {result.PrefabPath}");
            }
            catch (Exception exception)
            {
                Debug.LogError($"PSD import failed: {exception}");
            }
        }

        private static void ImportAndLog(string bundlePath)
        {
            try
            {
                var prefabPath = Psd2UguiImportService.ImportBundle(bundlePath);
                Debug.Log($"Imported PSD bundle to prefab: {prefabPath}");
            }
            catch (Exception exception)
            {
                Debug.LogError($"PSD bundle import failed: {exception}");
            }
        }

        private static string ResolveInitialFileDirectory(string lastFilePath)
        {
            if (!string.IsNullOrWhiteSpace(lastFilePath) && File.Exists(lastFilePath))
            {
                return Path.GetDirectoryName(lastFilePath) ?? Psd2UguiPackageInfo.ProjectRootFileSystemPath;
            }

            return Psd2UguiPackageInfo.ProjectRootFileSystemPath;
        }

        private static string ResolveInitialFolderDirectory(string lastFolderPath)
        {
            if (!string.IsNullOrWhiteSpace(lastFolderPath) && Directory.Exists(lastFolderPath))
            {
                return lastFolderPath;
            }

            return Psd2UguiPackageInfo.ProjectRootFileSystemPath;
        }
    }
}
