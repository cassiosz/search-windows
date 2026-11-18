# search-windows

A Windows port of [Search](https://github.com/driceroland/Search), the small,
fast, quiet browser for the Mac by Office Commun — rewritten in Rust on plain
Win32 and WebView2, with the same point: one field, the tabs, and the page.

> Working name. "Search" and its icon belong to Office Commun; this fork will
> get a name and icon of its own before it is distributed.

## Build

Windows 10/11, Rust (stable, MSVC) and the Visual Studio C++ Build Tools.
WebView2 ships with Windows 11.

```
cd rust
cargo build --release      # rust\target\release\SearchWin.exe
cargo test --workspace
```

## Layout

- `rust/` — the app (`src/`) and one crate per concern (`crates/`: tabs,
  history, shield, curtain, bookmarks, downloads, prefs, bench).
- `src/`, `tests/` — the C# + WinUI 3 prototype used to choose Rust.
- `PLANO.md` — the plan, decisions and phases (in Portuguese).

## License

MIT — see [LICENSE](LICENSE). Derived from Search by Office Commun (MIT).
