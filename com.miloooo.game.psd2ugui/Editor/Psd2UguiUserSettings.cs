using System;
using System.Collections.Generic;
using System.IO;
using TMPro;
using UnityEditor;
using UnityEngine;

namespace PsdUi.Editor
{
    [FilePath("UserSettings/Psd2UguiUserSettings.asset", FilePathAttribute.Location.ProjectFolder)]
    public sealed class Psd2UguiUserSettings : ScriptableSingleton<Psd2UguiUserSettings>
    {
        [SerializeField] private string photoshopExePath = string.Empty;
        [SerializeField] private string lastPsdPath = string.Empty;
        [SerializeField] private string lastBundlePath = string.Empty;
        

        public string PhotoshopExePath
        {
            get => photoshopExePath ?? string.Empty;
            set => photoshopExePath = value ?? string.Empty;
        }

        public string LastPsdPath
        {
            get => lastPsdPath ?? string.Empty;
            set => lastPsdPath = value ?? string.Empty;
        }

        public string LastBundlePath
        {
            get => lastBundlePath ?? string.Empty;
            set => lastBundlePath = value ?? string.Empty;
        }

        public void SaveSettings()
        {
            Save(true);
        }
    }
}
