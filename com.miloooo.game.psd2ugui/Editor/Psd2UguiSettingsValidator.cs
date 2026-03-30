using System;
using System.IO;
using TMPro;

namespace PsdUi.Editor
{
    internal static class Psd2UguiSettingsValidator
    {
        internal static bool TryValidateUnityImportRoot(
            string importRootAssetPath,
            out string error)
        {
            if (string.IsNullOrWhiteSpace(importRootAssetPath))
            {
                error = "Unity import root must not be empty.";
                return false;
            }

            var normalized = importRootAssetPath.Replace('\\', '/').Trim();
            if (string.Equals(normalized, "Assets", StringComparison.OrdinalIgnoreCase))
            {
                error = "Unity import root must be a subfolder under Assets, not the Assets root itself.";
                return false;
            }

            if (!normalized.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase))
            {
                error = "Unity import root must start with Assets/.";
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidateBundlePath(string bundlePath, out string error)
        {
            if (string.IsNullOrWhiteSpace(bundlePath))
            {
                error = "Bundle path must not be empty.";
                return false;
            }

            if (!Directory.Exists(bundlePath))
            {
                error = $"Bundle directory does not exist: {bundlePath}";
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidatePsdPath(string psdPath, out string error)
        {
            if (string.IsNullOrWhiteSpace(psdPath))
            {
                error = "PSD path must not be empty.";
                return false;
            }

            if (!File.Exists(psdPath))
            {
                error = $"PSD file does not exist: {psdPath}";
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidateCacheDirectory(
            string cacheDirectory,
            out string error)
        {
            if (string.IsNullOrWhiteSpace(cacheDirectory))
            {
                error = "Export cache directory must not be empty.";
                return false;
            }

            try
            {
                _ = Psd2UguiProjectSettings.ResolveCacheDirectory(cacheDirectory);
            }
            catch (Exception ex) when (
                ex is ArgumentException
                || ex is NotSupportedException
                || ex is PathTooLongException)
            {
                error = $"Export cache directory is invalid: {cacheDirectory}";
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidatePhotoshopExecutable(
            string photoshopExePath,
            out string error)
        {
            if (string.IsNullOrWhiteSpace(photoshopExePath))
            {
                error = "Photoshop executable path must not be empty.";
                return false;
            }

            if (!File.Exists(photoshopExePath))
            {
                error = $"Photoshop executable does not exist: {photoshopExePath}";
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidateTextMeshProFont(
            Psd2UguiProjectSettings projectSettings,
            out string error)
        {
            if (projectSettings.TextMeshProFont != null || TMP_Settings.defaultFontAsset != null)
            {
                error = string.Empty;
                return true;
            }

            error = "TextMeshPro font must be configured in PSD2UGUI settings or TMP Settings.";
            return false;
        }

        internal static bool TryValidatePsdImportSettings(
            Psd2UguiProjectSettings projectSettings,
            Psd2UguiUserSettings userSettings,
            out string error)
        {
            if (!Psd2UguiPackageInfo.IsWindowsEditor)
            {
                error = "Import PSD is only supported on Windows Unity Editor.";
                return false;
            }

            if (!TryValidateCacheDirectory(projectSettings.CacheDirectoryDisplayValue, out error))
            {
                return false;
            }

            if (!TryValidateUnityImportRoot(projectSettings.UnityImportRoot, out error))
            {
                return false;
            }

            if (!TryValidateTextMeshProFont(projectSettings, out error))
            {
                return false;
            }

            if (!TryValidatePhotoshopExecutable(userSettings.PhotoshopExePath, out error))
            {
                return false;
            }

            error = string.Empty;
            return true;
        }

        internal static bool TryValidateBundleImportSettings(
            Psd2UguiProjectSettings projectSettings,
            Psd2UguiUserSettings userSettings,
            string bundlePath,
            out string error)
        {
            if (!TryValidateUnityImportRoot(projectSettings.UnityImportRoot, out error))
            {
                return false;
            }

            if (!TryValidateBundlePath(bundlePath, out error))
            {
                return false;
            }

            if (!TryValidateTextMeshProFont(projectSettings, out error))
            {
                return false;
            }

            error = string.Empty;
            return true;
        }
    }
}
