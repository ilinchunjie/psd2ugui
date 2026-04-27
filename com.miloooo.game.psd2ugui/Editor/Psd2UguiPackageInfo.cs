using System;
using System.IO;
using UnityEngine;

namespace PsdUi.Editor
{
    internal static class Psd2UguiPackageInfo
    {
        internal const string WindowsExecutableRelativePath = "Packages/com.miloooo.game.psd2ugui/Editor/Tools/Win/psd2ugui.exe";
        internal const string MacOsExecutableRelativePath = "Packages/com.miloooo.game.psd2ugui/Editor/Tools/Mac/psd2ugui";

        internal static string ProjectRootFileSystemPath =>
            Path.GetFullPath(Path.Combine(Application.dataPath, ".."));

        internal static bool IsWindowsEditor =>
            Application.platform == RuntimePlatform.WindowsEditor;

        internal static bool IsMacOsEditor =>
            Application.platform == RuntimePlatform.OSXEditor;

        internal static bool IsAppleSiliconEditor =>
            IsMacOsEditor && SystemInfo.processorType.IndexOf("Apple", StringComparison.OrdinalIgnoreCase) >= 0;

        internal static bool IsSupportedEditorPlatform =>
            IsWindowsEditor || IsAppleSiliconEditor;

        internal static string GetEmbeddedExecutableFileSystemPath()
        {
            string relativePath;
            if (IsWindowsEditor)
            {
                relativePath = WindowsExecutableRelativePath;
            }
            else if (IsAppleSiliconEditor)
            {
                relativePath = MacOsExecutableRelativePath;
            }
            else
            {
                throw new InvalidOperationException(GetUnsupportedEditorMessage());
            }

            return Path.GetFullPath(Path.Combine(ProjectRootFileSystemPath, relativePath));
        }

        internal static string GetUnsupportedEditorMessage()
        {
            if (IsMacOsEditor)
            {
                return "Import PSD is only supported on Windows Unity Editor or macOS Apple Silicon Unity Editor. Intel Mac is not supported.";
            }

            return "Import PSD is only supported on Windows Unity Editor or macOS Apple Silicon Unity Editor.";
        }

        internal static bool TryConvertAbsolutePathToAssetPath(
            string absolutePath,
            out string assetPath)
        {
            assetPath = string.Empty;
            if (string.IsNullOrWhiteSpace(absolutePath))
            {
                return false;
            }

            var projectRoot = ProjectRootFileSystemPath.TrimEnd(
                Path.DirectorySeparatorChar,
                Path.AltDirectorySeparatorChar);
            var fullPath = Path.GetFullPath(absolutePath).TrimEnd(
                Path.DirectorySeparatorChar,
                Path.AltDirectorySeparatorChar);
            if (!fullPath.StartsWith(projectRoot, StringComparison.OrdinalIgnoreCase))
            {
                return false;
            }

            var relativePath = fullPath
                .Substring(projectRoot.Length)
                .TrimStart(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            assetPath = relativePath.Replace('\\', '/');
            return assetPath.StartsWith("Assets/", StringComparison.OrdinalIgnoreCase)
                || string.Equals(assetPath, "Assets", StringComparison.OrdinalIgnoreCase);
        }
    }
}
