namespace SearchWin.Core;

/// <summary>
/// What you type has to be a place. This either hands back an address or hands
/// back nothing — never a guess. Port of upstream Sources/Search/Address.swift.
/// </summary>
public static class Address
{
    /// Schemes the window can show itself. Anything else typed with a scheme
    /// (mailto:, an app link) is somebody else's job and is refused here.
    static readonly HashSet<string> Ours = new(StringComparer.OrdinalIgnoreCase)
    {
        "http", "https", "file", "about", "data", InternalPage.Scheme,
    };

    /// A file on this PC, typed as C:\path or C:/path or ~\path. Only committed
    /// navigation checks the disk; suggestions never do.
    public static Uri? File(string typed)
    {
        var text = typed.Trim();
        string path;
        if (text.StartsWith("~\\") || text.StartsWith("~/"))
            path = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), text[2..]);
        else if (text.Length >= 3 && char.IsAsciiLetter(text[0]) && text[1] == ':' && (text[2] == '\\' || text[2] == '/'))
            path = text;
        else
            return null;
        return System.IO.File.Exists(path) ? new Uri(Path.GetFullPath(path)) : null;
    }

    public static Uri? Url(string typed)
    {
        var text = typed.Trim();
        if (text.Length == 0 || text.Contains(' ')) return null;

        // Written with a scheme, it is taken at its word.
        var split = text.IndexOf("://", StringComparison.Ordinal);
        if (split >= 0)
        {
            if (!Ours.Contains(text[..split])) return null;
            return Uri.TryCreate(text, UriKind.Absolute, out var given) ? given : null;
        }
        if (text.StartsWith("about:", StringComparison.OrdinalIgnoreCase) ||
            text.StartsWith("data:", StringComparison.OrdinalIgnoreCase))
            return Uri.TryCreate(text, UriKind.Absolute, out var special) ? special : null;

        // Everything else has to look like a host before it gets a scheme.
        // "hello world" is not a website, and neither is "todo".
        var end = text.IndexOfAny(['/', '?', '#']);
        var head = end < 0 ? text : text[..end];
        if (head.Contains('@')) return null; // an email address
        if (head.StartsWith("[::1]")) return Make("http://" + text);
        var host = head.Split(':')[0];
        if (!LooksLikeHost(host)) return null;

        // A local server almost never has a certificate.
        var local = host == "localhost"
            || host.EndsWith(".localhost", StringComparison.Ordinal)
            || host.EndsWith(".local", StringComparison.Ordinal)
            || host == "127.0.0.1"
            || host == "0.0.0.0"
            || PrivateRange(host);
        return Make((local ? "http://" : "https://") + text);
    }

    static Uri? Make(string text) => Uri.TryCreate(text, UriKind.Absolute, out var uri) ? uri : null;

    /// Four numbers and nothing more: 10.io is a website, not 10.0.0.0/8.
    static bool PrivateRange(string host)
    {
        var parts = host.Split('.');
        if (parts.Length != 4) return false;
        var octets = new byte[4];
        for (var i = 0; i < 4; i++)
            if (!byte.TryParse(parts[i], out octets[i])) return false;
        return octets[0] == 10
            || (octets[0] == 192 && octets[1] == 168)
            || (octets[0] == 172 && octets[1] >= 16 && octets[1] <= 31);
    }

    /// Dev servers print 0.0.0.0 as the address to open; it is opened as localhost.
    public static Uri? Reachable(Uri url)
    {
        if (url.Host != "0.0.0.0" || (url.Scheme != "http" && url.Scheme != "https")) return null;
        return new UriBuilder(url) { Host = "localhost" }.Uri;
    }

    static bool LooksLikeHost(string host)
    {
        if (host == "localhost") return true;
        var labels = host.Split('.');
        if (labels.Length == 4 && labels.All(l => byte.TryParse(l, out _))) return true;
        if (labels.Length < 2) return false;
        foreach (var label in labels)
        {
            if (label.Length == 0 || label[0] == '-' || label[^1] == '-') return false;
            foreach (var c in label)
                if (!char.IsLetterOrDigit(c) && c != '-') return false;
        }
        // A dotted thing ending in letters is a domain; ending in digits, a version number.
        var tld = labels[^1];
        return tld.Length >= 2 && tld.All(char.IsLetter);
    }

    /// A page's address as the field shows it for editing: something that, sent
    /// as it is, goes back to the same page.
    public static string Editable(Uri page)
    {
        var full = page.AbsoluteString();
        var prefix = page.Scheme + "://";
        if (!full.StartsWith(prefix, StringComparison.OrdinalIgnoreCase)) return full;
        var shortForm = full[prefix.Length..];
        var candidates = new List<string> { shortForm };
        if (page.AbsolutePath == "/" && page.Query.Length == 0 && page.Fragment.Length == 0 && shortForm.EndsWith('/'))
            candidates.Insert(0, shortForm[..^1]);
        foreach (var candidate in candidates)
        {
            var back = Url(candidate);
            if (back is null) continue;
            var b = back.AbsoluteString();
            if (b == full || b + "/" == full) return candidate;
        }
        return full;
    }

    /// An address as a row shows it: without a leading www.
    public static string WithoutWww(string address) =>
        address.StartsWith("www.", StringComparison.OrdinalIgnoreCase) ? address[4..] : address;

    /// What the tab says before the page has told us its title.
    public static string Pretty(Uri url)
    {
        if (string.IsNullOrEmpty(url.Host)) return url.AbsoluteString();
        var bare = WithoutWww(url.Host);
        var path = url.AbsolutePath;
        return path.Length == 0 || path == "/" ? bare : bare + path;
    }

    /// The address escaped as sent, not unescaped as Uri.ToString() would show it.
    public static string AbsoluteString(this Uri url) => url.AbsoluteUri;
}
