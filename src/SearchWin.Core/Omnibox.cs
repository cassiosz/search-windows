namespace SearchWin.Core;

/// <summary>
/// The one field: an address goes there, anything else is searched.
/// Nothing typed leaves the PC until Return.
/// </summary>
public static class Omnibox
{
    public static Uri? Resolve(string typed, Engine engine = Engines.Standard, string custom = "")
    {
        var text = typed.Trim();
        if (text.Length == 0) return null;
        var url = Address.File(text) ?? Address.Url(text);
        if (url is not null) return Address.Reachable(url) ?? url;
        return engine.Search(text, custom);
    }
}

/// <summary>The browser's own pages, search://settings and the like.</summary>
public static class InternalPage
{
    public const string Scheme = "search";
}
