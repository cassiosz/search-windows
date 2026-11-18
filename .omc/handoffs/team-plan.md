## Handoff: team-plan → team-exec

- **Decided (defaults, pending user confirmation)**: rewrite in C# .NET 10 + WinUI 3 + WebView2; v1 = core browser only; provisional name; pt-BR + en-US. Full plan in PLANO.md.
- **Rejected**: WPF (airspace issues over WebView2), CEF (+150 MB, own engine updates), Tauri (multi-webview unstable), line-by-line port (upstream is AppKit/WebKit only).
- **Risks**: WebView2 extension API is partial; ad blocker via WebResourceRequested needs a fast matcher; Chrome passwords only importable via CSV; "Search" name/icon belong to Office Commun.
- **Files**: PLANO.md, upstream/ (clone of driceroland/Search @ 117e5ff, v1.0.14).
- **Remaining**: user confirms decisions; install .NET 10 SDK + WinUI templates (needs permission); Phase 0 scaffold; then Phase 1 parallel workers.
- **Note**: OMC MCP server (state_write) failed to connect; team state is not persisted there.
