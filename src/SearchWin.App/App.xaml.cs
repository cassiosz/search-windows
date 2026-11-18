using Microsoft.UI.Xaml;

namespace SearchWin;

public partial class App : Application
{
    MainWindow? window;

    public App()
    {
        // As upstream: a crash is written down on this PC and goes nowhere else.
        UnhandledException += (_, e) => CrashLog.Write(e.Exception);
        AppDomain.CurrentDomain.UnhandledException += (_, e) => CrashLog.Write(e.ExceptionObject as Exception);
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        // The page engine is the slow part of a launch; start it before the window exists.
        PageEngine.Warm();
        window = new MainWindow();
        window.Activate();
    }
}
