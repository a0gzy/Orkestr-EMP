// Synthetic, opt-in Windows deployment fixture. No reboot, networking or service.
using System;
using System.IO;
using System.Reflection;
using Microsoft.Win32;
public static class OrkestrPilotFixture {
    public static int Main(string[] args) {
        if (args.Length != 1 || (args[0] != "/pilot-0" && args[0] != "/pilot-3010")) return 64;
        bool rebootCode = args[0] == "/pilot-3010";
        string product = rebootCode ? "{5D6444C6-3D63-4E1F-AD2D-1D5000053010}" : "{5D6444C6-3D63-4E1F-AD2D-1D5000050000}";
        string folder = @"C:\OrkestrPilot\" + (rebootCode ? "Fixture3010" : "Fixture0");
        string marker = Path.Combine(folder, "orkestr-pilot-canary.txt");
        const string contents = "Orkestr synthetic deployment fixture 1.2.3";
        if (Directory.Exists(folder) && (!File.Exists(marker) || File.ReadAllText(marker) != contents)) return 65;
        Directory.CreateDirectory(folder);
        File.WriteAllText(marker, contents);
        File.AppendAllText(Path.Combine(folder, "launches.log"), DateTime.UtcNow.ToString("o") + Environment.NewLine);
        using (RegistryKey key = Registry.LocalMachine.CreateSubKey(@"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\" + product)) {
            key.SetValue("DisplayName", "Orkestr synthetic deployment fixture " + (rebootCode ? "3010" : "0"));
            key.SetValue("DisplayVersion", "1.2.3");
            key.SetValue("Publisher", "Orkestr pilot (synthetic)");
            key.SetValue("InstallLocation", folder);
            key.SetValue("SystemComponent", 1, RegistryValueKind.DWord);
        }
        return rebootCode ? 3010 : 0;
    }
}
