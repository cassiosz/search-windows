namespace SearchWin;

/// <summary>%LOCALAPPDATA%\SearchWin\crash.log. Never sent anywhere.</summary>
static class CrashLog
{
    public static void Write(Exception? exception)
    {
        try
        {
            Directory.CreateDirectory(PageEngine.DataFolder);
            File.AppendAllText(Path.Combine(PageEngine.DataFolder, "crash.log"),
                $"{DateTime.Now:O}\n{exception}\n\n");
        }
        catch { /* nothing more to do while going down */ }
    }
}
