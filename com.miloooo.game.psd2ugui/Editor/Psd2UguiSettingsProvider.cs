using System;
using System.IO;
using UnityEditor;
using UnityEngine;

namespace PsdUi.Editor
{
    internal static class Psd2UguiSettingsProvider
    {
        [SettingsProvider]
        public static SettingsProvider CreateProvider()
        {
            return new SettingsProvider("Project/PSD2UGUI", SettingsScope.Project)
            {
                label = "PSD2UGUI",
                guiHandler = _ => DrawGui()
            };
        }

        private static void DrawGui()
        {
            var projectSettings = Psd2UguiProjectSettings.instance;
            var userSettings = Psd2UguiUserSettings.instance;
            var serializedProjectSettings = new SerializedObject(projectSettings);
            serializedProjectSettings.Update();
            
            EditorGUILayout.LabelField("Pipeline", EditorStyles.boldLabel);

            EditorGUI.BeginChangeCheck();

            var cacheDirectory = DrawFolderField(
                "Export Cache Directory",
                projectSettings.CacheDirectoryDisplayValue,
                Psd2UguiPackageInfo.ProjectRootFileSystemPath);

            var importRoot = DrawAssetFolderField(
                "Unity Import Root",
                projectSettings.UnityImportRoot);

            var photoshopExePath = DrawFileField(
                "Photoshop Executable",
                userSettings.PhotoshopExePath,
                "exe");

            EditorGUILayout.Space();
            EditorGUILayout.LabelField("Import Defaults", EditorStyles.boldLabel);
            EditorGUILayout.PropertyField(
                serializedProjectSettings.FindProperty("textMeshProFont"),
                new GUIContent("TextMeshPro Font"));

            if (EditorGUI.EndChangeCheck())
            {
                projectSettings.CacheDirectory = cacheDirectory;
                projectSettings.UnityImportRoot = importRoot;
                userSettings.PhotoshopExePath = photoshopExePath;
                serializedProjectSettings.ApplyModifiedPropertiesWithoutUndo();
                projectSettings.SaveSettings();
                userSettings.SaveSettings();
            }

            EditorGUILayout.Space();
            if (!Psd2UguiSettingsValidator.TryValidatePsdImportSettings(
                    projectSettings,
                    userSettings,
                    out var error))
            {
                EditorGUILayout.HelpBox(error, MessageType.Warning);
            }
            else
            {
                EditorGUILayout.HelpBox("PSD import is ready.", MessageType.Info);
            }
        }

        private static string DrawFolderField(
            string label,
            string currentValue,
            string fallbackStartDirectory)
        {
            EditorGUILayout.BeginHorizontal();
            var newValue = EditorGUILayout.TextField(label, currentValue);
            if (GUILayout.Button("Browse", GUILayout.Width(72f)))
            {
                var startDirectory = fallbackStartDirectory;
                if (!string.IsNullOrWhiteSpace(currentValue))
                {
                    try
                    {
                        startDirectory = Psd2UguiProjectSettings.ResolveCacheDirectory(currentValue);
                    }
                    catch (Exception ex) when (
                        ex is ArgumentException
                        || ex is NotSupportedException
                        || ex is PathTooLongException)
                    {
                        startDirectory = fallbackStartDirectory;
                    }
                }

                var selected = EditorUtility.OpenFolderPanel(label, startDirectory, string.Empty);
                if (!string.IsNullOrWhiteSpace(selected))
                {
                    newValue = selected;
                }
            }
            EditorGUILayout.EndHorizontal();
            return newValue;
        }

        private static string DrawAssetFolderField(string label, string currentValue)
        {
            EditorGUILayout.BeginHorizontal();
            var newValue = EditorGUILayout.TextField(label, currentValue);
            if (GUILayout.Button("Browse", GUILayout.Width(72f)))
            {
                var startDirectory = Path.Combine(
                    Psd2UguiPackageInfo.ProjectRootFileSystemPath,
                    "Assets");
                if (!string.IsNullOrWhiteSpace(currentValue))
                {
                    var candidate = Path.Combine(
                        Psd2UguiPackageInfo.ProjectRootFileSystemPath,
                        currentValue.Replace('/', Path.DirectorySeparatorChar));
                    if (Directory.Exists(candidate))
                    {
                        startDirectory = candidate;
                    }
                }

                var selected = EditorUtility.OpenFolderPanel(label, startDirectory, string.Empty);
                if (!string.IsNullOrWhiteSpace(selected))
                {
                    if (Psd2UguiPackageInfo.TryConvertAbsolutePathToAssetPath(
                            selected,
                            out var assetPath))
                    {
                        newValue = assetPath;
                    }
                    else
                    {
                        EditorUtility.DisplayDialog(
                            "Invalid Import Root",
                            "The selected folder must be inside this Unity project's Assets directory.",
                            "OK");
                    }
                }
            }
            EditorGUILayout.EndHorizontal();
            return newValue;
        }

        private static string DrawFileField(
            string label,
            string currentValue,
            string extension)
        {
            EditorGUILayout.BeginHorizontal();
            var newValue = EditorGUILayout.TextField(label, currentValue);
            if (GUILayout.Button("Browse", GUILayout.Width(72f)))
            {
                var startDirectory = string.IsNullOrWhiteSpace(currentValue)
                    ? Psd2UguiPackageInfo.ProjectRootFileSystemPath
                    : Path.GetDirectoryName(currentValue) ?? Psd2UguiPackageInfo.ProjectRootFileSystemPath;
                var selected = EditorUtility.OpenFilePanel(label, startDirectory, extension);
                if (!string.IsNullOrWhiteSpace(selected))
                {
                    newValue = selected;
                }
            }
            EditorGUILayout.EndHorizontal();
            return newValue;
        }
    }
}
