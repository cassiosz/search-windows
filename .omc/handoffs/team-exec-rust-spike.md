## Handoff: Rust spike → decision

- **Decided**: user asked for a Rust comparison; rust/ builds Phase 0 with windows 0.62 + webview2-com 0.39, plain Win32 + GDI, native EDIT field, static WebView2 loader, manifest embedded by build.rs.
- **Measured**: 442 KB single exe, app 3.4 MB private, first page ~360 ms warm / 0.8 s cold; C# was 103 MB, 80 MB, ~640 ms / 1.2 s. WebView2 ~170-180 MB in both.
- **Risks**: Rust UI is hand-drawn (tab drag, accessibility, animation by hand). Keyboard shortcuts and typing not verified end-to-end — automated SendKeys hit another window (foreground lock); do not repeat that method, use the bench/named-pipe approach or ask the user to test by hand.
- **Files**: rust/{Cargo.toml,build.rs,app.manifest,src/{main,page,address,engine,omnibox}.rs}; PLANO.md comparison table.
- **Remaining**: user picks C# or Rust; then Phase 1 tasks re-scoped to the chosen stack.
