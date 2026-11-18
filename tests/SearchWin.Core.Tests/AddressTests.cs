using SearchWin.Core;

namespace SearchWin.Core.Tests;

public class AddressTests
{
    [Theory]
    [InlineData("example.com", "https://example.com/")]
    [InlineData("github.com/driceroland/Search", "https://github.com/driceroland/Search")]
    [InlineData("localhost:3000", "http://localhost:3000/")]
    [InlineData("192.168.0.10", "http://192.168.0.10/")]
    [InlineData("printer.local", "http://printer.local/")]
    [InlineData("10.io", "https://10.io/")]
    [InlineData("[::1]:8080", "http://[::1]:8080/")]
    [InlineData("http://example.com", "http://example.com/")]
    public void Places(string typed, string expected) =>
        Assert.Equal(expected, Address.Url(typed)!.AbsoluteUri);

    [Theory]
    [InlineData("hello world")]
    [InlineData("todo")]
    [InlineData("1.2.3")]
    [InlineData("me@example.com")]
    [InlineData("mailto://x")]
    [InlineData("-bad.com")]
    public void NotPlaces(string typed) => Assert.Null(Address.Url(typed));

    [Fact]
    public void WordsAreSearched() =>
        Assert.Equal("https://www.google.com/search?q=gatos%20fofos", Omnibox.Resolve("gatos fofos")!.AbsoluteUri);

    [Fact]
    public void ZeroesOpenAsLocalhost() =>
        Assert.Equal("http://localhost:5173/", Omnibox.Resolve("0.0.0.0:5173")!.AbsoluteUri);

    [Theory]
    [InlineData("https://example.com/", "example.com")]
    [InlineData("http://example.com/", "http://example.com/")]
    [InlineData("http://localhost:3000/", "localhost:3000")]
    [InlineData("https://example.com/a?b=1", "example.com/a?b=1")]
    public void Editable(string page, string shown) => Assert.Equal(shown, Address.Editable(new Uri(page)));

    [Fact]
    public void Pretty() => Assert.Equal("github.com/x", Address.Pretty(new Uri("https://www.github.com/x")));

    [Fact]
    public void CustomEngineFromAnotherBrowser() =>
        Assert.Equal("https://s.example/?q=%s", Engines.Written("s.example/?q={searchTerms}"));
}
