namespace SearchWin.Core;

/// <summary>Search engines, as upstream Sources/Search/Engine.swift.</summary>
public enum Engine { Google, DuckDuckGo, Bing, Ecosia, Startpage, Kagi, Brave, Qwant, Custom }

public static class Engines
{
    public const Engine Standard = Engine.Google;

    public static string Title(this Engine engine) => engine switch
    {
        Engine.Google => "Google",
        Engine.DuckDuckGo => "DuckDuckGo",
        Engine.Bing => "Bing",
        Engine.Ecosia => "Ecosia",
        Engine.Startpage => "Startpage",
        Engine.Kagi => "Kagi",
        Engine.Brave => "Brave Search",
        Engine.Qwant => "Qwant",
        _ => "Custom",
    };

    public static string Template(this Engine engine, string custom = "") => engine switch
    {
        Engine.Google => "https://www.google.com/search?q=%s",
        Engine.DuckDuckGo => "https://duckduckgo.com/?q=%s",
        Engine.Bing => "https://www.bing.com/search?q=%s",
        Engine.Ecosia => "https://www.ecosia.org/search?q=%s",
        Engine.Startpage => "https://www.startpage.com/sp/search?query=%s",
        Engine.Kagi => "https://kagi.com/search?q=%s",
        Engine.Brave => "https://search.brave.com/search?q=%s",
        Engine.Qwant => "https://www.qwant.com/?q=%s",
        _ => Accepts(Written(custom)) ? Written(custom) : Standard.Template(),
    };

    /// A custom template as people bring it: with {searchTerms}, or without https://.
    public static string Written(string template)
    {
        var text = template.Trim().Replace("{searchTerms}", "%s");
        if (text.Length > 0 && !text.Contains("://")) text = "https://" + text;
        return text;
    }

    public static bool Accepts(string template) =>
        template.Contains("%s") &&
        Uri.TryCreate(template.Replace("%s", "x"), UriKind.Absolute, out var uri) &&
        (uri.Scheme == "https" || uri.Scheme == "http") && uri.Host.Length > 0;

    public static Uri Search(this Engine engine, string words, string custom = "") =>
        new(engine.Template(custom).Replace("%s", Uri.EscapeDataString(words.Trim())));
}
