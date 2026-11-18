using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Input;
using Microsoft.Web.WebView2.Core;
using SearchWin.Core;
using Windows.Graphics;
using Windows.System;

namespace SearchWin;

public sealed partial class MainWindow : Window
{
    Task? ready;
    bool timed;

    public MainWindow()
    {
        InitializeComponent();
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(TitleBar);
        AppWindow.Title = "Search";
        AppWindow.Resize(new SizeInt32(1280, 840));

        // A link handed over on the command line (as the default browser will) opens at once;
        // otherwise the window starts on the field, as upstream: there is no start page.
        var first = Environment.GetCommandLineArgs().Skip(1).FirstOrDefault();
        if (first is not null) Go(first);
        else Root.Loaded += (_, _) => Field.Focus(FocusState.Programmatic);
    }

    /// The page engine is attached the first time a page is asked for, never before.
    Task Ready() => ready ??= Prepare();

    async Task Prepare()
    {
        // Attached before the control is in the window, the engine comes back null:
        // a link handed over at launch arrives before Loaded.
        if (!Page.IsLoaded)
        {
            var loaded = new TaskCompletionSource();
            Page.Loaded += (_, _) => loaded.TrySetResult();
            await loaded.Task;
        }
        await Page.EnsureCoreWebView2Async(await PageEngine.Shared);
        var core = Page.CoreWebView2;
        var settings = core.Settings;
        settings.IsStatusBarEnabled = false;
        settings.IsPasswordAutosaveEnabled = false;
        settings.IsGeneralAutofillEnabled = false;
        settings.IsReputationCheckingRequired = false;

        core.SourceChanged += (_, _) => ShowAddress();
        core.DocumentTitleChanged += (_, _) =>
            AppWindow.Title = string.IsNullOrEmpty(core.DocumentTitle) ? "Search" : core.DocumentTitle;
        core.NavigationCompleted += (_, _) => ReportLaunch();
        // One tab for now (Phase 0): a new window becomes this page. Phase 1 opens a tab.
        core.NewWindowRequested += (_, e) =>
        {
            e.Handled = true;
            core.Navigate(e.Uri);
        };
        Page.Visibility = Visibility.Visible;
    }

    async void Go(string typed)
    {
        var uri = Omnibox.Resolve(typed);
        if (uri is null) return;
        try
        {
            await Ready();
            Page.CoreWebView2.Navigate(uri.AbsoluteUri);
            Page.Focus(FocusState.Programmatic);
        }
        catch (Exception e)
        {
            CrashLog.Write(e);
            ready = null; // the next try starts the engine again
        }
    }

    Uri? PageUri => Page.CoreWebView2 is { } core && Uri.TryCreate(core.Source, UriKind.Absolute, out var uri)
        && uri.Scheme != "about" ? uri : null;

    void ShowAddress()
    {
        if (Field.FocusState != FocusState.Unfocused) return;
        Field.Text = PageUri is { } uri ? Address.Pretty(uri) : "";
    }

    void FieldGotFocus(object sender, RoutedEventArgs e)
    {
        if (PageUri is { } uri) Field.Text = Address.Editable(uri);
        Field.SelectAll();
    }

    void FieldLostFocus(object sender, RoutedEventArgs e) => ShowAddress();

    void FieldKeyDown(object sender, KeyRoutedEventArgs e)
    {
        if (e.Key == VirtualKey.Enter)
        {
            e.Handled = true;
            Go(Field.Text);
        }
        else if (e.Key == VirtualKey.Escape && Page.Visibility == Visibility.Visible)
        {
            e.Handled = true;
            Page.Focus(FocusState.Programmatic);
        }
    }

    void FocusField(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs e)
    {
        e.Handled = true;
        Field.Focus(FocusState.Keyboard);
    }

    void Reload(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs e)
    {
        e.Handled = true;
        Page.CoreWebView2?.Reload();
    }

    void Back(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs e)
    {
        e.Handled = true;
        if (Page.CoreWebView2?.CanGoBack == true) Page.CoreWebView2.GoBack();
    }

    void Forward(KeyboardAccelerator sender, KeyboardAcceleratorInvokedEventArgs e)
    {
        e.Handled = true;
        if (Page.CoreWebView2?.CanGoForward == true) Page.CoreWebView2.GoForward();
    }

    /// With SEARCHWIN_TIMING=1, the time from process start to the first page drawn,
    /// appended to launch.log beside the app's data. Nothing is ever sent anywhere.
    void ReportLaunch()
    {
        if (timed) return;
        timed = true;
        if (Environment.GetEnvironmentVariable("SEARCHWIN_TIMING") != "1") return;
        Directory.CreateDirectory(PageEngine.DataFolder);
        File.AppendAllText(Path.Combine(PageEngine.DataFolder, "launch.log"),
            $"{DateTime.Now:O} first page {(DateTime.Now - System.Diagnostics.Process.GetCurrentProcess().StartTime).TotalMilliseconds:F0} ms\n");
    }
}
