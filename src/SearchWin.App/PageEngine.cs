using Microsoft.Web.WebView2.Core;

namespace SearchWin;

/// <summary>
/// The one WebView2 environment every tab shares: one browser process, one
/// cookie jar per profile. Created once, as early as possible, because starting
/// the engine is most of what a launch costs.
/// </summary>
static class PageEngine
{
    static Task<CoreWebView2Environment>? shared;

    public static string DataFolder { get; } = Path.Combine(
        Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData), "SearchWin");

    /// Must run on the UI thread.
    public static void Warm() => shared ??= Create();

    public static Task<CoreWebView2Environment> Shared => shared ??= Create();

    static Task<CoreWebView2Environment> Create()
    {
        var options = new CoreWebView2EnvironmentOptions
        {
            // Extensions come in a later phase, through our own UI.
            AreBrowserExtensionsEnabled = false,
            // Nothing leaves the PC but the pages asked for: no SmartScreen lookups.
            AdditionalBrowserArguments = "--disable-features=msSmartScreenProtection",
        };
        return CoreWebView2Environment
            .CreateWithOptionsAsync(null, Path.Combine(DataFolder, "Pages"), options)
            .AsTask();
    }
}
