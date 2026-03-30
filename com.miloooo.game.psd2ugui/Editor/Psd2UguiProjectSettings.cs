using System;
using System.Collections.Generic;
using System.IO;
using TMPro;
using UnityEditor;
using UnityEngine;

namespace PsdUi.Editor {

    [FilePath("ProjectSettings/Psd2UguiProjectSettings.asset", FilePathAttribute.Location.ProjectFolder)]
    public sealed class Psd2UguiProjectSettings : ScriptableSingleton<Psd2UguiProjectSettings> {
        internal const string DefaultUnityImportRoot = "Assets/Generated/PsdUi";
        internal const string DefaultCacheDirectoryName = ".psd2ugui";

        [SerializeField] private string cacheDirectory = string.Empty;
        [SerializeField] private string unityImportRoot = DefaultUnityImportRoot;
        [SerializeField] private TMP_FontAsset textMeshProFont;

        public string UnityImportRoot {
            get => string.IsNullOrWhiteSpace(unityImportRoot) ? DefaultUnityImportRoot : unityImportRoot;
            set => unityImportRoot = string.IsNullOrWhiteSpace(value) ? DefaultUnityImportRoot : value.Replace('\\', '/');
        }

        public string CacheDirectory {
            get => ResolveCacheDirectory(cacheDirectory);
            set => cacheDirectory = value?.Trim() ?? string.Empty;
        }

        internal string CacheDirectoryDisplayValue =>
            string.IsNullOrWhiteSpace(cacheDirectory) ? GetDefaultCacheDirectory() : cacheDirectory;
        
        public TMP_FontAsset TextMeshProFont
        {
            get => textMeshProFont;
            set => textMeshProFont = value;
        }

        internal static string ResolveCacheDirectory(string configuredCacheDirectory) {
            if (string.IsNullOrWhiteSpace(configuredCacheDirectory)) {
                return GetDefaultCacheDirectory();
            }

            var trimmedPath = configuredCacheDirectory.Trim();
            if (Path.IsPathRooted(trimmedPath)) {
                return Path.GetFullPath(trimmedPath);
            }

            return Path.GetFullPath(Path.Combine(Psd2UguiPackageInfo.ProjectRootFileSystemPath, trimmedPath));
        }

        private static string GetDefaultCacheDirectory() {
            return Path.Combine(Psd2UguiPackageInfo.ProjectRootFileSystemPath, DefaultCacheDirectoryName);
        }

        public void SaveSettings() {
            Save(true);
        }
    }

}