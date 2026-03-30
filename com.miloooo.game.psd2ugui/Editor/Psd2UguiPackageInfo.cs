using System;
using System.IO;
using UnityEditor.PackageManager;
using UnityEngine;

namespace PsdUi.Editor
{
    internal static class Psd2UguiPackageInfo
    {
        internal const string WindowsExecutableRelativePath = "Assets/framework/Editor/psd2ugui/Editor/Tools/Win/psd2ugui.exe";

        internal static string ProjectRootFileSystemPath =>
            Path.GetFullPath(Path.Combine(Application.dataPath, ".."));

        internal static bool IsWindowsEditor =>
            Application.platform == RuntimePlatform.WindowsEditor;

        internal static string GetWindowsExecutableFileSystemPath()
        {
            return WindowsExecutableRelativePath;
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
