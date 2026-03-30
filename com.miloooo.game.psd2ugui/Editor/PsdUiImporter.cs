using System;
using System.Collections.Generic;
using System.IO;
using Newtonsoft.Json;
using Newtonsoft.Json.Linq;
using TMPro;
using UnityEditor;
using UnityEngine;
using UnityEngine.UI;
using Object = UnityEngine.Object;
using RenderMode = UnityEngine.RenderMode;

namespace PsdUi.Editor
{
    public static class PsdUiImporter
    {
        private const string RootNodeName = "panel";
        private static readonly Vector2 RootCanvasReferenceResolution = new(1920f, 1080f);
        private static readonly Vector2 CenterAnchor = new(0.5f, 0.5f);

        public static string ImportBundle(string bundlePath)
        {
            return ImportBundle(bundlePath, Psd2UguiProjectSettings.instance.UnityImportRoot);
        }

        public static string ImportBundle(string bundlePath, string importRootAssetPath)
        {
            if (!Psd2UguiSettingsValidator.TryValidateUnityImportRoot(importRootAssetPath, out var error))
            {
                throw new InvalidOperationException(error);
            }

            var plan = LoadPlanFromBundle(bundlePath);
            var documentId = SanitizeAssetName(plan.SourceBundle.DocumentId);
            var importRoot = importRootAssetPath.Replace('\\', '/').TrimEnd('/');
            var targetFolder = EnsureAssetFolder($"{importRoot}/{documentId}");
            var assetContext = new ImportedAssetContext(bundlePath, targetFolder);
            var root = BuildTransientPrefab(plan, assetContext);
            try
            {
                var prefabPath = $"{targetFolder}/{RootNodeName}.prefab";
                PrefabUtility.SaveAsPrefabAsset(root, prefabPath);
                AssetDatabase.SaveAssets();
                AssetDatabase.Refresh();
                return prefabPath;
            }
            finally
            {
                Object.DestroyImmediate(root);
            }
        }

        public static GameObject BuildTransientPrefab(string planJson)
        {
            var plan = JsonConvert.DeserializeObject<UiPlanDocument>(planJson);
            if (plan == null)
            {
                throw new InvalidOperationException("Failed to deserialize ui_plan.json.");
            }

            return BuildTransientPrefab(plan, null);
        }

        internal static GameObject BuildTransientPrefab(UiPlanDocument plan, ImportedAssetContext assetContext)
        {
            var nodeNameCounters = new Dictionary<string, int>(StringComparer.OrdinalIgnoreCase);

            var root = new GameObject(
                RootNodeName,
                typeof(RectTransform),
                typeof(Canvas),
                typeof(CanvasScaler),
                typeof(GraphicRaycaster));
            var rootRect = root.GetComponent<RectTransform>();
            ConfigureRootRect(rootRect, plan.Document.Width, plan.Document.Height);
            ConfigureRootCanvas(root.GetComponent<Canvas>(), root.GetComponent<CanvasScaler>());

            var rootRectData = new PlanRectData
            {
                X = 0,
                Y = 0,
                Width = plan.Document.Width,
                Height = plan.Document.Height,
                LocalX = 0,
                LocalY = 0
            };

            foreach (var node in plan.Nodes)
            {
                BuildNodeRecursive(node, root.transform, rootRectData, assetContext, nodeNameCounters);
            }

            return root;
        }

        internal static UiPlanDocument LoadPlanFromBundle(string bundlePath)
        {
            if (string.IsNullOrWhiteSpace(bundlePath))
            {
                throw new ArgumentException("Bundle path is required.", nameof(bundlePath));
            }

            var candidatePaths = new[]
            {
                Path.Combine(bundlePath, "plan", "ui_plan.json"),
                Path.Combine(bundlePath, "ui_plan.json")
            };

            foreach (var candidate in candidatePaths)
            {
                if (File.Exists(candidate))
                {
                    var contents = File.ReadAllText(candidate);
                    var plan = JsonConvert.DeserializeObject<UiPlanDocument>(contents);
                    if (plan == null)
                    {
                        throw new InvalidOperationException($"Failed to parse ui plan at {candidate}.");
                    }

                    return plan;
                }
            }

            throw new FileNotFoundException("ui_plan.json was not found in the bundle.", bundlePath);
        }

        private static void BuildNodeRecursive(
            PlanNodeData node,
            Transform parent,
            PlanRectData parentRect,
            ImportedAssetContext assetContext,
            Dictionary<string, int> nodeNameCounters)
        {
            var nodeName = ResolveNodeName(node, nodeNameCounters);
            var gameObject = new GameObject(nodeName, typeof(RectTransform));
            gameObject.transform.SetParent(parent, false);

            var rectTransform = gameObject.GetComponent<RectTransform>();
            ApplyCenterRect(rectTransform, node.Rect, parentRect);

            switch (node.ComponentType)
            {
                case "Image":
                    ConfigureImageNode(gameObject, node, assetContext);
                    break;
                case "Text":
                    ConfigureText(gameObject, node);
                    break;
            }

            foreach (var child in node.Children)
            {
                BuildNodeRecursive(child, gameObject.transform, node.Rect, assetContext, nodeNameCounters);
            }

            gameObject.transform.SetSiblingIndex(Mathf.Max(0, node.RenderOrder));
        }

        private static string ResolveNodeName(PlanNodeData node, Dictionary<string, int> nodeNameCounters)
        {
            var componentName = GetUnifiedComponentName(node.ComponentType);
            var nextIndex = nodeNameCounters.TryGetValue(componentName, out var currentIndex)
                ? currentIndex + 1
                : 1;
            nodeNameCounters[componentName] = nextIndex;
            return $"{componentName}_{nextIndex}";
        }

        private static string GetUnifiedComponentName(string componentType)
        {
            return componentType switch
            {
                "Image" => "img",
                "Text" => "txt",
                _ => "box"
            };
        }

        private static void ConfigureRootRect(RectTransform rectTransform, int width, int height)
        {
            rectTransform.anchorMin = CenterAnchor;
            rectTransform.anchorMax = CenterAnchor;
            rectTransform.pivot = CenterAnchor;
            rectTransform.anchoredPosition = new Vector2(width * 0.5f, -height * 0.5f);
            rectTransform.sizeDelta = new Vector2(width, height);
        }

        private static void ConfigureRootCanvas(Canvas canvas, CanvasScaler canvasScaler)
        {
            if (canvas != null)
            {
                canvas.renderMode = RenderMode.ScreenSpaceOverlay;
                canvas.additionalShaderChannels =
                    AdditionalCanvasShaderChannels.TexCoord1 |
                    AdditionalCanvasShaderChannels.TexCoord2 |
                    AdditionalCanvasShaderChannels.TexCoord3 |
                    AdditionalCanvasShaderChannels.Normal |
                    AdditionalCanvasShaderChannels.Tangent;
                canvas.vertexColorAlwaysGammaSpace = true;
            }

            if (canvasScaler != null)
            {
                canvasScaler.uiScaleMode = CanvasScaler.ScaleMode.ScaleWithScreenSize;
                canvasScaler.referenceResolution = RootCanvasReferenceResolution;
                canvasScaler.screenMatchMode = CanvasScaler.ScreenMatchMode.MatchWidthOrHeight;
                canvasScaler.matchWidthOrHeight = 1f;
                canvasScaler.referencePixelsPerUnit = 100f;
            }
        }

        private static void ApplyCenterRect(RectTransform rectTransform, PlanRectData rect, PlanRectData parentRect)
        {
            rectTransform.anchorMin = CenterAnchor;
            rectTransform.anchorMax = CenterAnchor;
            rectTransform.pivot = CenterAnchor;
            rectTransform.anchoredPosition = CalculateCenteredAnchoredPosition(rect, parentRect);
            rectTransform.sizeDelta = new Vector2(rect.Width, rect.Height);
        }

        private static Vector2 CalculateCenteredAnchoredPosition(PlanRectData rect, PlanRectData parentRect)
        {
            // Convert PSD top-left coordinates into a centered anchor/pivot offset.
            var x = (rect.X - parentRect.X) + rect.Width * 0.5f - parentRect.Width * 0.5f;
            var y = parentRect.Height * 0.5f - (rect.Y - parentRect.Y) - rect.Height * 0.5f;
            return new Vector2(x, y);
        }


        private static void ConfigureImageNode(GameObject gameObject, PlanNodeData node, ImportedAssetContext assetContext) {
            var image = GetOrAddComponent<Image>(gameObject);
            image.sprite = LoadSpriteForPath(assetContext, GetMetadataString(node.Metadata, "asset_path"));
            image.color = ApplyOpacity(Color.white, GetNodeOpacity(node));
        }

        private static void ConfigureText(GameObject gameObject, PlanNodeData node)
        {
            var text = GetOrAddComponent<TextMeshProUGUI>(gameObject);
            var fontAsset = ResolveTextFontAsset();
            if (fontAsset != null)
            {
                text.font = fontAsset;
            }

            text.text = node.Text != null ? node.Text.Content ?? string.Empty : string.Empty;
            text.enableWordWrapping = false;
            text.fontSize = node.Text != null && node.Text.FontSize.HasValue
                ? node.Text.FontSize.Value
                : Mathf.Max(12f, node.Rect.Height * 0.75f);
            text.color = ApplyOpacity(
                ParseColor(node.Text != null ? node.Text.Color : string.Empty),
                GetNodeOpacity(node));
            text.alignment = ParseAlignment(ResolveTextAlignment(node.Text));
        }

        private static TMP_FontAsset ResolveTextFontAsset()
        {
            var configuredFont = Psd2UguiProjectSettings.instance.TextMeshProFont;
            return configuredFont != null ? configuredFont : TMP_Settings.defaultFontAsset;
        }

        private static string ResolveTextAlignment(PlanTextData text)
        {
            if (text == null)
            {
                return string.Empty;
            }

            if (!string.IsNullOrWhiteSpace(text.Alignment))
            {
                return text.Alignment;
            }

            if (text.ParagraphRuns != null && text.ParagraphRuns.Count > 0)
            {
                return text.ParagraphRuns[0].Alignment ?? string.Empty;
            }

            return string.Empty;
        }

        private static Color ParseColor(string colorHex)
        {
            if (!string.IsNullOrWhiteSpace(colorHex) && ColorUtility.TryParseHtmlString(colorHex, out var color))
            {
                return color;
            }

            return Color.white;
        }

        private static TextAlignmentOptions ParseAlignment(string alignment)
        {
            if (string.IsNullOrWhiteSpace(alignment))
            {
                return TextAlignmentOptions.Left;
            }

            var normalized = alignment.Trim().ToLowerInvariant();
            if (normalized.Contains("center"))
            {
                return TextAlignmentOptions.Center;
            }

            if (normalized.Contains("right"))
            {
                return TextAlignmentOptions.Right;
            }

            return TextAlignmentOptions.Left;
        }

        private static float GetNodeOpacity(PlanNodeData node)
        {
            return Mathf.Clamp01(GetMetadataFloat(node.Metadata, "opacity", 1f));
        }

        private static Color ApplyOpacity(Color color, float opacity)
        {
            color.a *= opacity;
            return color;
        }

        private static string EnsureAssetFolder(string assetPath)
        {
            var parts = assetPath.Split('/');
            var current = parts[0];
            for (var index = 1; index < parts.Length; index += 1)
            {
                var next = $"{current}/{parts[index]}";
                if (!AssetDatabase.IsValidFolder(next))
                {
                    AssetDatabase.CreateFolder(current, parts[index]);
                }

                current = next;
            }

            return current;
        }

        private static T GetOrAddComponent<T>(GameObject gameObject) where T : UnityEngine.Component
        {
            var component = gameObject.GetComponent<T>();
            return component != null ? component : gameObject.AddComponent<T>();
        }

        private static Sprite LoadSpriteForPath(ImportedAssetContext assetContext, string relativeSourcePath)
        {
            if (assetContext == null || string.IsNullOrWhiteSpace(relativeSourcePath))
            {
                return null;
            }

            return assetContext.ImportSprite(relativeSourcePath);
        }

        private static string GetMetadataString(JObject metadata, string field)
        {
            if (metadata == null)
            {
                return string.Empty;
            }

            if (!metadata.TryGetValue(field, out var token) || token == null || token.Type == JTokenType.Null)
            {
                return string.Empty;
            }

            return token.Type == JTokenType.String ? token.Value<string>() ?? string.Empty : token.ToString(Formatting.None);
        }

        private static float GetMetadataFloat(JObject metadata, string field, float fallback)
        {
            if (metadata == null)
            {
                return fallback;
            }

            if (!metadata.TryGetValue(field, out var token) || token == null || token.Type == JTokenType.Null)
            {
                return fallback;
            }

            return token.Value<float?>() ?? fallback;
        }

        private static string SanitizeAssetName(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
            {
                return "generated_ui";
            }

            var invalid = Path.GetInvalidFileNameChars();
            var cleaned = value.Trim();
            foreach (var invalidChar in invalid)
            {
                cleaned = cleaned.Replace(invalidChar, '_');
            }

            cleaned = cleaned.Replace(' ', '_');
            return string.IsNullOrWhiteSpace(cleaned) ? "generated_ui" : cleaned;
        }

        internal sealed class ImportedAssetContext
        {
            private readonly string bundlePath;
            private readonly string importedRoot;
            private readonly string projectRoot;
            private readonly Dictionary<string, Sprite> spriteCache = new();

            internal ImportedAssetContext(string bundlePath, string targetFolder)
            {
                this.bundlePath = Path.GetFullPath(bundlePath);
                importedRoot = EnsureAssetFolder($"{targetFolder}/Imported");
                projectRoot = Path.GetFullPath(Path.Combine(Application.dataPath, ".."));
            }

            internal Sprite ImportSprite(string relativeSourcePath)
            {
                if (string.IsNullOrWhiteSpace(relativeSourcePath))
                {
                    return null;
                }

                var normalizedRelativePath = relativeSourcePath.Replace('\\', '/');
                if (spriteCache.TryGetValue(normalizedRelativePath, out var cached))
                {
                    return cached;
                }

                var sourcePath = Path.GetFullPath(Path.Combine(bundlePath, normalizedRelativePath.Replace('/', Path.DirectorySeparatorChar)));
                if (!File.Exists(sourcePath))
                {
                    Debug.LogWarning($"PSD image asset not found: {sourcePath}");
                    spriteCache[normalizedRelativePath] = null;
                    return null;
                }

                var targetAssetPath = $"{importedRoot}/{normalizedRelativePath}";
                var targetDirectory = Path.GetDirectoryName(targetAssetPath)?.Replace('\\', '/');
                if (!string.IsNullOrWhiteSpace(targetDirectory))
                {
                    EnsureAssetFolder(targetDirectory);
                }

                var targetFileSystemPath = ToProjectFileSystemPath(targetAssetPath);
                var targetFileSystemDirectory = Path.GetDirectoryName(targetFileSystemPath);
                if (!string.IsNullOrWhiteSpace(targetFileSystemDirectory))
                {
                    Directory.CreateDirectory(targetFileSystemDirectory);
                }

                if (!File.Exists(targetFileSystemPath) || !FilesAreEqual(sourcePath, targetFileSystemPath))
                {
                    File.Copy(sourcePath, targetFileSystemPath, true);
                }

                AssetDatabase.ImportAsset(targetAssetPath, ImportAssetOptions.ForceSynchronousImport);
                ConfigureTextureImporter(targetAssetPath);

                var sprite = AssetDatabase.LoadAssetAtPath<Sprite>(targetAssetPath);
                spriteCache[normalizedRelativePath] = sprite;
                return sprite;
            }

            private string ToProjectFileSystemPath(string assetPath)
            {
                var relativeAssetPath = assetPath.Replace('/', Path.DirectorySeparatorChar);
                return Path.Combine(projectRoot, relativeAssetPath);
            }

            private static bool FilesAreEqual(string leftPath, string rightPath)
            {
                var leftInfo = new FileInfo(leftPath);
                var rightInfo = new FileInfo(rightPath);
                if (leftInfo.Length != rightInfo.Length)
                {
                    return false;
                }

                const int bufferSize = 81920;
                using var left = File.OpenRead(leftPath);
                using var right = File.OpenRead(rightPath);
                var leftBuffer = new byte[bufferSize];
                var rightBuffer = new byte[bufferSize];

                while (true)
                {
                    var leftRead = left.Read(leftBuffer, 0, leftBuffer.Length);
                    var rightRead = right.Read(rightBuffer, 0, rightBuffer.Length);
                    if (leftRead != rightRead)
                    {
                        return false;
                    }

                    if (leftRead == 0)
                    {
                        return true;
                    }

                    for (var index = 0; index < leftRead; index += 1)
                    {
                        if (leftBuffer[index] != rightBuffer[index])
                        {
                            return false;
                        }
                    }
                }
            }

            private static void ConfigureTextureImporter(string assetPath)
            {
                var importer = AssetImporter.GetAtPath(assetPath) as TextureImporter;
                if (importer == null)
                {
                    return;
                }

                importer.textureType = TextureImporterType.Sprite;
                importer.spriteImportMode = SpriteImportMode.Single;
                importer.alphaIsTransparency = true;
                importer.mipmapEnabled = false;
                importer.wrapMode = TextureWrapMode.Clamp;
                importer.filterMode = FilterMode.Bilinear;
                importer.textureCompression = TextureImporterCompression.Uncompressed;
                importer.spritePixelsPerUnit = 100f;
                importer.isReadable = false;
                importer.sRGBTexture = false;
                importer.SaveAndReimport();
            }
        }
    }
}
